//! Extract only painted bitmap objects. Text, paths, annotations, page transforms and unused resources are never rendered.
use chromink_core::{
    control::{memory_budget, validate_decode, WorkControl},
    parallel::Parallelism,
};
use hayro_interpret::{
    hayro_syntax::{
        content::{ops::TypedInstruction, TypedIter},
        object::{Dict, Name, Stream},
        page::Resources,
        Pdf,
    },
    BitmapData, ImageData,
};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
#[derive(Clone, Debug)]
pub struct Bitmap {
    pub page: u32,
    pub ordinal: u32,
    pub width: u32,
    pub height: u32,
}
struct CachedDocument {
    path: PathBuf,
    bytes: u64,
    stamp: Option<SystemTime>,
    pdf: Arc<Pdf>,
}
static DOCUMENTS: OnceLock<Mutex<VecDeque<CachedDocument>>> = OnceLock::new();
fn open(path: &Path) -> Result<Arc<Pdf>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let stamp = metadata.modified().ok();
    let mut cache = DOCUMENTS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map_err(|_| "PDF 缓存锁不可用")?;
    if let Some(index) = cache
        .iter()
        .position(|d| d.path == path && d.bytes == metadata.len() && d.stamp == stamp)
    {
        let doc = cache.remove(index).unwrap();
        let pdf = doc.pdf.clone();
        cache.push_back(doc);
        return Ok(pdf);
    }
    cache.retain(|d| d.path != path);
    drop(cache);
    if metadata.len() > memory_budget() as u64 / 2 {
        return Err("PDF 文件超过当前内存预算".into());
    }
    let pdf = Arc::new(
        Pdf::new(std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("无法读取 PDF（需要密码请先解密）：{e:?}"))?,
    );
    let mut cache = DOCUMENTS
        .get()
        .unwrap()
        .lock()
        .map_err(|_| "PDF 缓存锁不可用")?;
    let limit = (memory_budget() / 16).min(64 << 20) as u64;
    if metadata.len() <= limit {
        cache.push_back(CachedDocument {
            path: path.to_owned(),
            bytes: metadata.len(),
            stamp,
            pdf: pdf.clone(),
        });
        while cache.len() > 8 || cache.iter().map(|d| d.bytes).sum::<u64>() > limit {
            cache.pop_front();
        }
    }
    Ok(pdf)
}
pub fn forget(path: &Path) {
    if let Some(cache) = DOCUMENTS.get() {
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|doc| doc.path != path);
        }
    }
}
fn identity(stream: &Stream<'_>) -> String {
    if let Some(id) = stream.dict().obj_id() {
        format!("{id:?}")
    } else {
        format!(
            "{:p}:{}",
            stream.dict().data().as_ptr(),
            stream.dict().data().len()
        )
    }
}
fn walk<'a>(
    content: &[u8],
    resources: &Resources<'a>,
    depth: usize,
    forms: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    visit: &mut impl FnMut(&Stream<'_>, &Resources<'a>, Option<&Name<'_>>) -> Result<bool, String>,
    control: &WorkControl,
) -> Result<bool, String> {
    if depth > 32 {
        return Err("PDF 图片嵌套层数超过限制".into());
    }
    let mut iter = TypedIter::new(content);
    let mut position = 0usize;
    while let Some(op) = iter.next() {
        position += 1;
        if position & 255 == 0 {
            control.check()?;
        }
        match op {
            TypedInstruction::XObject(object) => {
                let name = object.0;
                if let Some(stream) = resources.get_x_object(name) {
                    match stream.dict().get::<Name<'_>>(b"Subtype").as_deref() {
                        Some(b"Image") => {
                            if seen.insert(identity(&stream))
                                && visit(&stream, resources, Some(name))?
                            {
                                return Ok(true);
                            }
                        }
                        Some(b"Form") => {
                            let key = identity(&stream);
                            if forms.insert(key.clone()) {
                                let decoded =
                                    stream.decoded().map_err(|_| "PDF 图片容器解压失败")?;
                                let child = Resources::from_parent(
                                    stream
                                        .dict()
                                        .get::<Dict<'_>>(b"Resources")
                                        .unwrap_or_default(),
                                    resources.clone(),
                                );
                                if walk(&decoded, &child, depth + 1, forms, seen, visit, control)? {
                                    return Ok(true);
                                }
                                forms.remove(&key);
                            }
                        }
                        _ => {}
                    }
                }
            }
            TypedInstruction::InlineImage(image) => {
                if seen.insert(identity(image.0)) && visit(image.0, resources, None)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    control.check()?;
    Ok(false)
}
pub fn images(path: &Path) -> Result<Vec<Bitmap>, String> {
    let pdf = open(path)?;
    let mut images = Vec::new();
    let control = WorkControl::unlimited();
    for (page_number, page) in pdf.pages().iter().enumerate() {
        let mut ordinal = 0;
        let mut visit = |stream: &Stream<'_>, _: &Resources<'_>, _: Option<&Name<'_>>| {
            let dict = stream.dict();
            let width = dict
                .get::<u32>(b"Width")
                .or_else(|| dict.get::<u32>(b"W"))
                .ok_or("PDF 图片缺少宽度")?;
            let height = dict
                .get::<u32>(b"Height")
                .or_else(|| dict.get::<u32>(b"H"))
                .ok_or("PDF 图片缺少高度")?;
            validate_decode(width, height)?;
            images.push(Bitmap {
                page: page_number as u32,
                ordinal,
                width,
                height,
            });
            ordinal += 1;
            Ok(false)
        };
        if let Some(content) = page.page_stream() {
            walk(
                content,
                page.resources(),
                0,
                &mut HashSet::new(),
                &mut HashSet::new(),
                &mut visit,
                &control,
            )?;
        }
    }
    if images.is_empty() {
        forget(path);
        return Err("PDF 中没有正在使用的图片，已排除文字或矢量 PDF".into());
    }
    Ok(images)
}
pub fn decode(
    path: &Path,
    bitmap: &Bitmap,
    workers: &Parallelism,
    control: &WorkControl,
) -> Result<image::DynamicImage, String> {
    let pdf = open(path)?;
    let page = pdf
        .pages()
        .get(bitmap.page as usize)
        .ok_or("找不到 PDF 图片页面")?;
    let mut ordinal = 0;
    let mut result = None;
    let mut visit = |stream: &Stream<'_>, resources: &Resources<'_>, _name: Option<&Name<'_>>| {
        let current = ordinal;
        ordinal += 1;
        if current != bitmap.ordinal {
            return Ok(false);
        }
        control.check()?;
        let dict = stream.dict();
        let filters = stream.filters();
        if filters.len() == 1
            && matches!(
                dict.get::<Name<'_>>(b"Filter").as_deref(),
                Some(b"DCTDecode") | Some(b"DCT")
            )
            && !dict.contains_key(b"Decode")
            && !dict.contains_key(b"SMask")
            && !dict.contains_key(b"Mask")
            && matches!(
                dict.get::<Name<'_>>(b"ColorSpace").as_deref(),
                Some(b"DeviceRGB") | Some(b"DeviceGray")
            )
        {
            if let Ok(image) =
                image::load_from_memory_with_format(&stream.raw_data(), image::ImageFormat::Jpeg)
            {
                validate_decode(image.width(), image.height())?;
                result = Some(image);
                return Ok(true);
            }
        }
        let data = hayro_interpret::decode_bitmap(stream, resources)
            .ok_or("PDF 内嵌图片解码失败，可能包含不支持的色彩空间或编码")?;
        control.check()?;
        let rgba = match data {
            BitmapData::Raster(data, alpha) => {
                let (width, height) = (data.width(), data.height());
                validate_decode(width, height)?;
                let mut rgba = match data {
                    ImageData::Rgb(rgb) => image::DynamicImage::ImageRgb8(
                        image::RgbImage::from_raw(width, height, rgb.data)
                            .ok_or("PDF RGB 数据长度错误")?,
                    )
                    .into_rgba8(),
                    ImageData::Luma(gray) => image::DynamicImage::ImageLuma8(
                        image::GrayImage::from_raw(width, height, gray.data)
                            .ok_or("PDF 灰度数据长度错误")?,
                    )
                    .into_rgba8(),
                };
                if let Some(alpha) = alpha {
                    let alpha = image::GrayImage::from_raw(alpha.width, alpha.height, alpha.data)
                        .ok_or("PDF 透明度数据长度错误")?;
                    let alpha = if alpha.dimensions() == (width, height) {
                        alpha
                    } else {
                        image::imageops::resize(
                            &alpha,
                            width,
                            height,
                            image::imageops::FilterType::Triangle,
                        )
                    };
                    for (index, (pixel, a)) in rgba.pixels_mut().zip(alpha.pixels()).enumerate() {
                        if index & 8191 == 0 {
                            control.check()?;
                        }
                        pixel[3] = a[0];
                    }
                }
                rgba
            }
            BitmapData::Stencil(gray) => {
                validate_decode(gray.width, gray.height)?;
                let mut rgba = image::RgbaImage::new(gray.width, gray.height);
                for (index, (pixel, alpha)) in rgba.pixels_mut().zip(gray.data).enumerate() {
                    if index & 8191 == 0 {
                        control.check()?;
                    }
                    *pixel = image::Rgba([0, 0, 0, alpha]);
                }
                rgba
            }
        };
        result = Some(image::DynamicImage::ImageRgba8(rgba));
        Ok(true)
    };
    workers
        .install(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                walk(
                    page.page_stream().unwrap_or_default(),
                    page.resources(),
                    0,
                    &mut HashSet::new(),
                    &mut HashSet::new(),
                    &mut visit,
                    control,
                )
            }))
        })
        .map_err(|_| "PDF 内嵌图片损坏或编码不受支持")??;
    control.check()?;
    result.ok_or_else(|| "找不到 PDF 内嵌图片".into())
}
