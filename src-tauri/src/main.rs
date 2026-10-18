#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod frames;
mod palette_metadata;
mod headless;
mod original;
mod pdf;
mod session;
use base64::{engine::general_purpose::STANDARD, Engine};
use chromink_core::{
    control::{memory_budget, validate_decode, WorkControl, CANCELLED},
    load_image,
    parallel::Parallelism,
    raster::Scene,
    trace_document, TraceOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
};
use tauri::Manager;

#[derive(Default, Clone)]
struct Store(Arc<Mutex<HashMap<u64, Entry>>>);
#[derive(Clone)]
struct Entry {
    path: PathBuf,
    page: Option<pdf::Bitmap>,
    frame: Option<u32>,
    width: u32,
    height: u32,
}
#[derive(Default, Clone)]
struct InputCache {
    current: Arc<AtomicU64>,
    cached: Arc<Mutex<Option<(u64, Arc<image::RgbaImage>)>>>,
    loading: Arc<Mutex<HashMap<u64, Arc<OnceLock<Result<Arc<image::RgbaImage>, String>>>>>>,
}
impl InputCache {
    fn select(&self, id: u64) {
        if self.current.swap(id, Ordering::Relaxed) != id {
            self.clear(None);
        }
    }
    fn clear(&self, id: Option<u64>) {
        if let Ok(mut cache) = self.cached.lock() {
            if id.is_none() || cache.as_ref().is_some_and(|(key, _)| Some(*key) == id) {
                *cache = None;
            }
        }
    }
}
// Bound concurrent decoders, including current previews and list thumbnails.
static DECODERS: OnceLock<(Mutex<(usize, usize)>, Condvar)> = OnceLock::new();
struct DecodePermit(usize);
impl DecodePermit {
    fn acquire(entry: &Entry) -> Result<Self, String> {
        let bytes = (entry.width as usize)
            .saturating_mul(entry.height as usize)
            .saturating_mul(
                if entry
                    .path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
                {
                    24
                } else {
                    12
                },
            );
        validate_decode(entry.width, entry.height)?;
        let (lock, changed) = DECODERS.get_or_init(|| (Mutex::new((0, 0)), Condvar::new()));
        let mut slot = lock.lock().map_err(|_| "解码预算锁不可用")?;
        while slot.0 >= 2 || (slot.0 > 0 && slot.1.saturating_add(bytes) > memory_budget()) {
            slot = changed.wait(slot).map_err(|_| "解码预算锁不可用")?;
        }
        validate_decode(entry.width, entry.height)?;
        slot.0 += 1;
        slot.1 += bytes;
        Ok(Self(bytes))
    }
}
impl Drop for DecodePermit {
    fn drop(&mut self) {
        if let Some((lock, changed)) = DECODERS.get() {
            if let Ok(mut slot) = lock.lock() {
                slot.0 -= 1;
                slot.1 -= self.0;
                changed.notify_all();
            }
        }
    }
}
fn source_image(
    id: u64,
    entry: &Entry,
    cache: &InputCache,
    workers: &Parallelism,
    control: &WorkControl,
) -> Result<Arc<image::RgbaImage>, String> {
    control.check()?;
    if let Some((key, image)) = &*cache.cached.lock().map_err(|_| "输入缓存锁不可用")? {
        if *key == id {
            return Ok(image.clone());
        }
    }
    let pending = cache
        .loading
        .lock()
        .map_err(|_| "输入任务锁不可用")?
        .entry(id)
        .or_insert_with(|| Arc::new(OnceLock::new()))
        .clone();
    let loaded = pending
        .get_or_init(|| {
            let _permit = DecodePermit::acquire(entry)?;
            let source = if let Some(bitmap) = &entry.page {
                pdf::decode(&entry.path, bitmap, workers, &WorkControl::unlimited())?.into_rgba8()
            } else {
                workers.install(|| {
                    if let Some(frame) = entry.frame {
                        frames::decode(&entry.path, frame).map(image::DynamicImage::into_rgba8)
                    } else {
                        load_image(&entry.path)
                    }
                })?
            };
            Ok(Arc::new(source))
        })
        .clone();
    cache
        .loading
        .lock()
        .map_err(|_| "输入任务锁不可用")?
        .remove(&id);
    if let Ok(image) = &loaded {
        let mut slot = cache.cached.lock().map_err(|_| "输入缓存锁不可用")?;
        if cache.current.load(Ordering::Relaxed) == id {
            *slot = Some((id, image.clone()));
        }
    }
    control.check()?;
    loaded
}
#[derive(Default, Clone)]
struct Jobs(Arc<Mutex<(u64, Option<WorkControl>)>>);
impl Jobs {
    fn start(&self, revision: u64, budget: usize) -> Result<WorkControl, String> {
        let mut slot = self.0.lock().map_err(|_| "任务状态锁不可用")?;
        if revision < slot.0 {
            return Err(CANCELLED.into());
        }
        if let Some(old) = &slot.1 {
            old.cancel();
        }
        let control = WorkControl::new(Arc::new(AtomicBool::new(false)), budget);
        *slot = (revision, Some(control.clone()));
        Ok(control)
    }
    fn cancel_before(&self, revision: u64) -> bool {
        if let Ok(mut slot) = self.0.lock() {
            if revision < slot.0 {
                return false;
            }
            if revision > slot.0 {
                if let Some(job) = &slot.1 {
                    job.cancel();
                }
                slot.0 = revision;
            }
            return true;
        }
        false
    }
    fn cancel(&self) {
        if let Ok(slot) = self.0.lock() {
            if let Some(job) = &slot.1 {
                job.cancel();
            }
        }
    }
}
struct RasterJobs(Jobs);
struct OriginalJobs(Jobs);
#[derive(Default, Clone)]
struct PreviewCache(Arc<Mutex<Option<(u64, u64, Arc<Scene>, WorkControl)>>>);
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    stage_width: f32,
    stage_height: f32,
}

struct Workers(Arc<Parallelism>);
fn output_name(path: &std::path::Path, o: &TraceOptions) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let colors = format!("c{}_{}", o.colors, o.dither);
    format!(
        "{stem}_{colors}_{}_p{}_D{}_S{}_O{}_{}_{}",
        match o.quantization.as_str() {
            "wl" => format!("wl-{}", o.lloyd_iterations),
            "vc" => format!(
                "vc-{}-{}",
                o.vecline_strategy,
                if o.vecline_strategy == "mean" {
                    o.vecline_iterations
                } else {
                    0
                }
            ),
            _ => o.quantization.clone(),
        },
        o.prescale,
        o.despeckle,
        o.smooth_corners,
        o.optimize_paths,
        if o.stacked { "stack" } else { "flat" },
        if o.tracing == "vecline" {
            format!(
                "vc-{}-{}-t{}-f{}-a{}-o{}-g{}",
                o.vecline_trace.vc_mode,
                o.vecline_trace.vc_preset,
                o.vecline_trace.vc_tolerance,
                o.vecline_trace.vc_fit_error,
                o.vecline_trace.vc_min_area,
                o.vecline_trace.vc_optimize_error,
                o.vecline_trace.vc_gradients as u8
            )
        } else {
            "potrace".into()
        },
    )
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Imported {
    id: u64,
    name: String,
    preview: String,
    width: u32,
    height: u32,
    bytes: u64,
    palette_colors: Option<u32>,
    pdf_group: Option<String>,
    pdf_name: Option<String>,
    page: Option<u32>,
    source_key: String,
    options: TraceOptions,
    source_name: String,
    frame: Option<u32>,
    ordinal: Option<u32>,
}
#[derive(Serialize)]
struct ImportResult {
    images: Vec<Imported>,
    errors: Vec<String>,
    selected: Option<u64>,
    library: Vec<session::LibraryNode>,
    expanded: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Traced {
    svg: String,
    bytes: usize,
    paths: usize,
    preview: String,
}

#[tauri::command]
async fn import_images(
    paths: Option<Vec<PathBuf>>,
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    workers: tauri::State<'_, Workers>,
    session: tauri::State<'_, session::Session>,
) -> Result<ImportResult, String> {
    let store = store.inner().clone();
    let session = session.inner().clone();
    let workers = workers.0.clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let paths = paths.unwrap_or_else(|| {
            rfd::FileDialog::new()
                .set_title("选择图片或 PDF")
                .add_filter(
                    "图片与 PDF",
                    &[
                        "png", "jpg", "jpeg", "bmp", "webp", "gif", "tif", "tiff", "ppm", "pgm",
                        "pbm", "tga", "pdf",
                    ],
                )
                .pick_files()
                .unwrap_or_default()
        });
        let mut result = ImportResult {
            images: Vec::new(),
            errors: Vec::new(),
            selected: None,
            library: Vec::new(),
            expanded: Vec::new(),
        };
        let decoded = workers.map_limit(&paths, 4, |path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let imported = (|| -> Result<Vec<Imported>, String> {
                if !input_supported(path) {
                    return Err("仅支持位图和图片 PDF，不支持矢量图或 ICO".into());
                }
                let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
                let bytes = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                let is_pdf = path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
                let dimensions = if is_pdf {
                    pdf::images(&path)?
                        .into_iter()
                        .map(|image| (image.width, image.height, Some(image), None))
                        .collect::<Vec<_>>()
                } else if frames::supported(&path) {
                    // GIF metadata scans composited frames; TIFF only reads directories.
                    let _permit = if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
                    {
                        let (w, h) = chromink_core::image_dimensions(&path)?;
                        Some(DecodePermit::acquire(&Entry {
                            path: path.clone(),
                            page: None,
                            frame: None,
                            width: w,
                            height: h,
                        })?)
                    } else {
                        None
                    };
                    frames::dimensions(&path)?
                        .into_iter()
                        .enumerate()
                        .map(|(index, (w, h))| (w, h, None, Some(index as u32)))
                        .collect()
                } else {
                    let (w, h) = chromink_core::image_dimensions(&path)
                        .map_err(|e| format!("无法读取图片尺寸：{e}"))?;
                    validate_decode(w, h)?;
                    vec![(w, h, None, None)]
                };
                let palette_colors = if is_pdf { None } else { palette_metadata::color_count(&path) };
                let mut entries = store.0.lock().map_err(|_| "状态锁不可用")?;
                let mut imported = Vec::with_capacity(dimensions.len());
                for (_index, (width, height, page, frame)) in dimensions.into_iter().enumerate() {
                    let id = if let Some((&id, _)) = entries.iter().find(|(_, entry)| {
                        entry.path == path
                            && entry.frame == frame
                            && entry.page.as_ref().map(|p| (p.page, p.ordinal))
                                == page.as_ref().map(|p| (p.page, p.ordinal))
                    }) {
                        let entry = entries.get_mut(&id).unwrap();
                        entry.width = width;
                        entry.height = height;
                        entry.page = page.clone();
                        entry.frame = frame;
                        id
                    } else {
                        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                        entries.insert(
                            id,
                            Entry {
                                path: path.clone(),
                                page: page.clone(),
                                frame,
                                width,
                                height,
                            },
                        );
                        id
                    };
                    imported.push(Imported {
                        id,
                        name: if is_pdf {
                            format!(
                                "{name} · 第 {} 页 · 图片 {}",
                                page.as_ref().unwrap().page + 1,
                                page.as_ref().unwrap().ordinal + 1
                            )
                        } else if let Some(frame) = frame {
                            format!("{name} · 第 {} 页 / 帧", frame + 1)
                        } else {
                            name.clone()
                        },
                        preview: String::new(),
                        width,
                        height,
                        bytes,
                        palette_colors,
                        pdf_group: is_pdf.then(|| path.to_string_lossy().into_owned()),
                        pdf_name: is_pdf.then(|| name.clone()),
                        ordinal: page.as_ref().map(|p| p.ordinal),
                        page: page.map(|p| p.page + 1),
                        source_name: name.clone(),
                        frame,
                        source_key: path.to_string_lossy().into_owned(),
                        options: session.options(&path),
                    });
                }
                Ok(imported)
            })();
            (name, imported)
        });
        for (name, imported) in decoded {
            match imported {
                Ok(imported) => result.images.extend(imported),
                Err(e) => result.errors.push(format!("{name}: {e}")),
            }
        }
        if !result.images.is_empty() {
            let mut state = session.0.lock().map_err(|_| "配置锁不可用")?;
            for image in &result.images {
                let path = PathBuf::from(&image.source_key);
                if !state.1.files.iter().any(|f| f.path == path) {
                    state.1.files.push(session::File {
                        path,
                        options: image.options.clone(),
                        items: Vec::new(),
                    });
                }
            }
            drop(state);
            cache.clear(None);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn session_config(store: &Store, session: &session::Session) -> Result<session::Config, String> {
    let saved = session.0.lock().map_err(|_| "配置锁不可用")?.1.clone();
    let entries = store.0.lock().map_err(|_| "状态锁不可用")?;
    let file_order: HashMap<_, _> = saved
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| (&file.path, index))
        .collect();
    let mut ordered: Vec<_> = entries.iter().collect();
    ordered.sort_by_key(|(id, entry)| {
        (
            file_order.get(&entry.path).copied().unwrap_or(usize::MAX),
            **id,
        )
    });
    let mut config = session::Config {
        selected: saved.selected.clone(),
        library: saved.library.clone(),
        expanded: saved.expanded.clone(),
        ..Default::default()
    };
    for (_, entry) in ordered {
        let item = entry
            .page
            .as_ref()
            .map(|p| (p.page, p.ordinal))
            .or(entry.frame.map(|f| (f, u32::MAX)));
        if let Some(file) = config.files.iter_mut().find(|f| f.path == entry.path) {
            file.items.push(item);
        } else {
            config.files.push(session::File {
                path: entry.path.clone(),
                options: saved
                    .files
                    .iter()
                    .find(|f| f.path == entry.path)
                    .map(|f| f.options.clone())
                    .unwrap_or_else(session::default_options),
                items: vec![item],
            });
        }
    }
    if config.selected.as_ref().is_some_and(|s| {
        !config
            .files
            .iter()
            .any(|f| f.path == s.path && f.items.contains(&s.item))
    }) {
        config.selected = None;
    }
    drop(entries);
    Ok(config)
}
fn persist_session(store: &Store, session: &session::Session) -> Result<(), String> {
    session::save(&session::path(), &session_config(store, session)?)
}
fn remember_session(
    id: Option<u64>,
    options: Option<TraceOptions>,
    revision: u64,
    store: &Store,
    session: &session::Session,
) {
    let selection = id.and_then(|id| {
        store
            .0
            .lock()
            .ok()?
            .get(&id)
            .map(|entry| session::Selection {
                path: entry.path.clone(),
                item: entry
                    .page
                    .as_ref()
                    .map(|p| (p.page, p.ordinal))
                    .or(entry.frame.map(|f| (f, u32::MAX))),
            })
    });
    session.update(revision, selection, options);
}
#[tauri::command]
fn update_preview_setting(
    id: Option<u64>,
    options: TraceOptions,
    revision: u64,
    store: tauri::State<'_, Store>,
    session: tauri::State<'_, session::Session>,
) {
    remember_session(id, Some(options), revision, &store, &session);
}
#[tauri::command]
fn save_session(
    id: Option<u64>,
    options: Option<TraceOptions>,
    revision: u64,
    store: tauri::State<'_, Store>,
    session: tauri::State<'_, session::Session>,
) -> Result<(), String> {
    remember_session(id, options, revision, &store, &session);
    persist_session(&store, &session)
}
#[tauri::command]
fn set_library(
    revision: u64,
    library: Vec<session::LibraryNode>,
    expanded: Vec<String>,
    session: tauri::State<'_, session::Session>,
) {
    session.set_library(revision, library, expanded);
}
#[tauri::command]
async fn restore_session(
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    workers: tauri::State<'_, Workers>,
    session: tauri::State<'_, session::Session>,
) -> Result<ImportResult, String> {
    let path = session::path();
    let legacy_file = std::env::temp_dir().join("config.cotrace.json");
    let legacy_folder = std::env::temp_dir().join("ColorTrace/config.json");
    let config_path = if path.exists() { &path } else if legacy_file.exists() { &legacy_file } else if legacy_folder.exists() { &legacy_folder } else { &path };
    let config = session::load(config_path)?;
    *session.0.lock().map_err(|_| "配置锁不可用")? = (0, config.clone());
    let paths = config.files.iter().map(|f| f.path.clone()).collect();
    let mut imported = import_images(Some(paths), store.clone(), cache, workers, session).await?;
    let mut entries = store.0.lock().map_err(|_| "状态锁不可用")?;
    imported.images.retain(|image| {
        let Some(entry) = entries.get(&image.id) else {
            return false;
        };
        let keep = config
            .files
            .iter()
            .find(|f| f.path == entry.path)
            .is_some_and(|f| {
                f.items.contains(
                    &entry
                        .page
                        .as_ref()
                        .map(|p| (p.page, p.ordinal))
                        .or(entry.frame.map(|f| (f, u32::MAX))),
                )
            });
        if !keep {
            entries.remove(&image.id);
        }
        keep
    });
    imported.selected = config.selected.and_then(|selected| {
        entries
            .iter()
            .find(|(_, e)| {
                e.path == selected.path
                    && e.page
                        .as_ref()
                        .map(|p| (p.page, p.ordinal))
                        .or(e.frame.map(|f| (f, u32::MAX)))
                        == selected.item
            })
            .map(|(id, _)| *id)
    });
    imported.library = config.library;
    imported.expanded = config.expanded;
    Ok(imported)
}
#[tauri::command]
async fn reveal_file(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !path.is_file() {
            return Err("保存的文件已不存在".into());
        }
        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn();
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn();
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let result = std::process::Command::new("xdg-open")
            .arg(path.parent().ok_or("文件位置无效")?)
            .spawn();
        result
            .map(|_| ())
            .map_err(|error| format!("无法打开文件位置：{error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
async fn preview_image(
    id: u64,
    thumbnail: bool,
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    workers: tauri::State<'_, Workers>,
) -> Result<String, String> {
    let entry = store
        .0
        .lock()
        .map_err(|_| "状态锁不可用")?
        .get(&id)
        .cloned()
        .ok_or("找不到图片")?;
    let cache = cache.inner().clone();
    let workers = workers.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let control = WorkControl::unlimited();
        let image = if thumbnail {
            let cached = cache
                .cached
                .lock()
                .map_err(|_| "输入缓存锁不可用")?
                .as_ref()
                .filter(|(key, _)| *key == id)
                .map(|(_, image)| image.clone());
            if let Some(image) = cached {
                image
            } else if cache.current.load(Ordering::Relaxed) == id {
                source_image(id, &entry, &cache, &workers, &control)?
            } else if let Some(bitmap) = &entry.page {
                let _permit = DecodePermit::acquire(&entry)?;
                Arc::new(pdf::decode(&entry.path, bitmap, &workers, &control)?.into_rgba8())
            } else {
                {
                    let _permit = DecodePermit::acquire(&entry)?;
                    Arc::new(workers.install(|| {
                        if let Some(frame) = entry.frame {
                            frames::decode(&entry.path, frame).map(image::DynamicImage::into_rgba8)
                        } else {
                            load_image(&entry.path)
                        }
                    })?)
                }
            }
        } else {
            source_image(id, &entry, &cache, &workers, &control)?
        };
        let limit = if thumbnail { 240 } else { 1200 };
        let factor = (limit as f64 / image.width() as f64)
            .min(limit as f64 / image.height() as f64)
            .min(1.);
        let preview = chromink_core::resample::resize(
            &image,
            (image.width() as f64 * factor).round().max(1.) as u32,
            (image.height() as f64 * factor).round().max(1.) as u32,
            &workers,
            &control,
        )?;
        let png = chromink_core::preview_png::encode_rgba(
            preview.as_raw(),
            preview.width(),
            preview.height(),
        )?;
        Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn cancel_trace(
    revision: u64,
    id: Option<u64>,
    jobs: tauri::State<'_, Jobs>,
    cache: tauri::State<'_, InputCache>,
    raster_jobs: tauri::State<'_, RasterJobs>,
    previews: tauri::State<'_, PreviewCache>,
    store: tauri::State<'_, Store>,
    session: tauri::State<'_, session::Session>,
    options: Option<TraceOptions>,
) {
    if !jobs.cancel_before(revision) {
        return;
    }
    if let Ok(mut scene) = previews.0.lock() {
        if scene
            .as_ref()
            .is_some_and(|(image, key, _, _)| Some(*image) != id || *key < revision)
        {
            *scene = None;
        }
    }
    let selection = id.and_then(|id| {
        store
            .0
            .lock()
            .ok()?
            .get(&id)
            .map(|entry| session::Selection {
                path: entry.path.clone(),
                item: entry
                    .page
                    .as_ref()
                    .map(|p| (p.page, p.ordinal))
                    .or(entry.frame.map(|f| (f, u32::MAX))),
            })
    });
    session.update(revision, selection, options);
    cache.select(id.unwrap_or(0));
    raster_jobs.0.cancel();
}
#[tauri::command]
async fn trace_image(
    id: u64,
    revision: u64,
    options: TraceOptions,
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    workers: tauri::State<'_, Workers>,
    jobs: tauri::State<'_, Jobs>,
    previews: tauri::State<'_, PreviewCache>,
) -> Result<Traced, String> {
    let store = store.inner().clone();
    let cache = cache.inner().clone();
    let workers = workers.0.clone();
    let control = jobs.start(revision, memory_budget())?;
    let previews = previews.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entry = store
            .0
            .lock()
            .map_err(|_| "状态锁不可用")?
            .get(&id)
            .cloned()
            .ok_or("找不到导入的图片")?;
        let source = source_image(id, &entry, &cache, &workers, &control)?;
        let document = trace_document(&source, &options, &workers, &control)?;
        let scene = Arc::new(Scene::compile(&document, &control)?);
        let ratio = (1200f32 / source.width().max(source.height()) as f32).min(1.);
        let width = (source.width() as f32 * ratio).ceil().max(1.) as u32;
        let height = (source.height() as f32 * ratio).ceil().max(1.) as u32;
        let preview = scene.png(
            width,
            height,
            0.,
            0.,
            width as f32,
            height as f32,
            &workers,
            &control,
        )?;
        control.check()?;
        let mut preview_slot = previews.0.lock().map_err(|_| "预览缓存锁不可用")?;
        control.check()?;
        if preview_slot
            .as_ref()
            .is_some_and(|(_, key, _, _)| *key > revision)
        {
            return Err(CANCELLED.into());
        }
        *preview_slot = Some((id, revision, scene, control));
        drop(preview_slot);
        Ok(Traced {
            bytes: document.svg.len(),
            paths: document.svg.matches("<path").count(),
            svg: document.svg,
            preview: format!("data:image/png;base64,{}", STANDARD.encode(preview)),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn original_preview(
    id: u64,
    request: u64,
    geometry: Geometry,
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    workers: tauri::State<'_, Workers>,
    jobs: tauri::State<'_, OriginalJobs>,
) -> Result<String, String> {
    let entry = store
        .0
        .lock()
        .map_err(|_| "状态锁不可用")?
        .get(&id)
        .cloned()
        .ok_or("找不到图片")?;
    let cache = cache.inner().clone();
    let workers = workers.0.clone();
    let control = jobs.0.start(request, memory_budget().min(128 << 20))?;
    tauri::async_runtime::spawn_blocking(move || {
        if cache.current.load(Ordering::Relaxed) != id {
            return Err(CANCELLED.into());
        }
        let source = source_image(id, &entry, &cache, &workers, &control)?;
        let pixels = source.as_ref();
        let viewport = original::viewport(pixels, &geometry, &workers, &control)?;
        let bytes = chromink_core::preview_png::encode_rgba(
            viewport.as_raw(),
            viewport.width(),
            viewport.height(),
        )?;
        control.check()?;
        if cache.current.load(Ordering::Relaxed) != id {
            return Err(CANCELLED.into());
        }
        Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn raster_preview(
    revision: u64,
    request: u64,
    geometry: Geometry,
    previews: tauri::State<'_, PreviewCache>,
    workers: tauri::State<'_, Workers>,
    jobs: tauri::State<'_, RasterJobs>,
) -> Result<String, String> {
    let scene = {
        let cache = previews.0.lock().map_err(|_| "预览缓存锁不可用")?;
        let (_, key, scene, control) = cache.as_ref().ok_or(CANCELLED)?;
        if *key != revision {
            return Err(CANCELLED.into());
        }
        control.check()?;
        scene.clone()
    };
    let control = jobs.0.start(request, memory_budget().min(128 << 20))?;
    let workers = workers.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = scene.png(
            geometry.width,
            geometry.height,
            geometry.x,
            geometry.y,
            geometry.stage_width,
            geometry.stage_height,
            &workers,
            &control,
        )?;
        control.check()?;
        Ok(format!("data:image/png;base64,{}", STANDARD.encode(data)))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn forget_image(
    id: u64,
    session: tauri::State<'_, session::Session>,
    store: tauri::State<'_, Store>,
    cache: tauri::State<'_, InputCache>,
    previews: tauri::State<'_, PreviewCache>,
) -> Result<(), String> {
    let mut entries = store.0.lock().map_err(|_| "状态锁不可用")?;
    let entry = entries.remove(&id).ok_or("找不到图片")?;
    if !entries.values().any(|other| other.path == entry.path) {
        if entry.page.is_some() {
            pdf::forget(&entry.path);
        }
        session
            .0
            .lock()
            .map_err(|_| "配置锁不可用")?
            .1
            .files
            .retain(|f| f.path != entry.path);
    }
    drop(entries);
    let _ = cache
        .current
        .compare_exchange(id, 0, Ordering::Relaxed, Ordering::Relaxed);
    if let Ok(mut scene) = previews.0.lock() {
        if scene.as_ref().is_some_and(|(image, _, _, _)| *image == id) {
            *scene = None;
        }
    }
    cache.clear(Some(id));
    Ok(())
}

#[tauri::command]
fn window_action(action: String, window: tauri::WebviewWindow) -> Result<bool, String> {
    match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => {
            if window.is_maximized().map_err(|e| e.to_string())? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "drag" => window.start_dragging(),
        "close" => {
            window.close().map_err(|e| e.to_string())?;
            return Ok(false);
        }
        _ => return Err("未知窗口操作".into()),
    }
    .map_err(|e| e.to_string())?;
    window.is_maximized().map_err(|e| e.to_string())
}
fn input_supported(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        [
            "png", "jpg", "jpeg", "bmp", "webp", "gif", "tif", "tiff", "ppm", "pgm", "pbm", "tga",
            "pdf",
        ]
        .contains(&e.to_ascii_lowercase().as_str())
    })
}
fn application_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(image) = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return image
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| "应用路径无效".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return bundle
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| "应用路径无效".into());
    }
    exe.parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "应用路径无效".into())
}
fn quick_save(directory: &std::path::Path, name: &str, svg: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    std::fs::create_dir_all(directory).map_err(|e| format!("无法创建保存目录：{e}"))?;
    let safe: String = name
        .chars()
        .take(160)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    for serial in 0..100000 {
        let filename = if serial == 0 {
            format!("{safe}.svg")
        } else {
            format!("{safe}_{serial}.svg")
        };
        let path = directory.join(filename);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(svg.as_bytes()).and_then(|_| file.sync_all()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(e.to_string());
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("无法快速保存：{e}；可右键使用另存为")),
        }
    }
    Err("保存文件重名过多，请右键另存为".into())
}
#[tauri::command]
async fn save_image(
    id: u64,
    options: TraceOptions,
    svg: String,
    save_as: bool,
    store: tauri::State<'_, Store>,
) -> Result<Option<String>, String> {
    let name = {
        let entries = store.0.lock().map_err(|_| "状态锁不可用")?;
        let entry = entries.get(&id).ok_or("图片已移除")?;
        let name = output_name(&entry.path, &options);
        entry
            .page
            .as_ref()
            .map(|page| format!("{name}_page{}_image{}", page.page + 1, page.ordinal + 1))
            .or_else(|| entry.frame.map(|f| format!("{name}_frame{}", f + 1)))
            .unwrap_or(name)
    };
    tauri::async_runtime::spawn_blocking(move || {
        if !save_as {
            let directory = application_directory()?.join("chromink");
            return quick_save(&directory, &name, &svg).map(|p| Some(p.display().to_string()));
        }
        let directory = dirs::download_dir().ok_or("无法定位系统下载文件夹")?;
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(directory)
            .set_title("另存为 SVG")
            .set_file_name(format!("{name}.svg"))
            .add_filter("SVG", &["svg"])
            .save_file()
        {
            std::fs::write(&path, svg).map_err(|e| e.to_string())?;
            Ok(Some(path.display().to_string()))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    if std::env::args_os().len() > 1 {
        headless::attach_console();
        std::process::exit(headless::run());
    }
    let workers = Arc::new(Parallelism::new(
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
    ));
    let focus_workers = workers.clone();
    tauri::Builder::default()
        .setup(|app| {
            if let (Some(window), Some(icon)) =
                (app.get_webview_window("main"), app.default_window_icon())
            {
                window.set_icon(icon.clone())?;
            }
            // Shell launchers can pass a hidden initial show state. Restore the
            // application explicitly once its WebView has been constructed.
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
                window.unminimize()?;
                window.set_focus()?;
            }
            Ok(())
        })
        .manage(Store::default())
        .manage(session::Session::default())
        .manage(InputCache::default())
        .manage(Jobs::default())
        .manage(RasterJobs(Jobs::default()))
        .manage(OriginalJobs(Jobs::default()))
        .manage(PreviewCache::default())
        .manage(Workers(workers))
        .on_window_event(move |window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Err(error) = persist_session(
                    &window.state::<Store>(),
                    &window.state::<session::Session>(),
                ) {
                    eprintln!("无法保存配置：{error}");
                }
            }
            if let tauri::WindowEvent::Focused(active) = event {
                focus_workers.set_active(if *active { focus_workers.maximum() } else { 1 });
            }
        })
        .invoke_handler(tauri::generate_handler![
            import_images,
            restore_session,
            save_session,
            update_preview_setting,
            set_library,
            preview_image,
            reveal_file,
            trace_image,
            cancel_trace,
            raster_preview,
            original_preview,
            forget_image,
            save_image,
            window_action
        ])
        .run(tauri::generate_context!())
        .expect("无法启动 Chromink");
}
