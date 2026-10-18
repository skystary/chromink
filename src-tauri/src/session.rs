//! One parameter set per physical source file. Only configuration is persisted.
use chromink_core::TraceOptions;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub path: PathBuf,
    pub item: Option<(u32, u32)>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct File {
    pub path: PathBuf,
    pub options: TraceOptions,
    pub items: Vec<Option<(u32, u32)>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LibraryNode {
    Source {
        key: PathBuf,
    },
    Group {
        id: String,
        name: String,
        children: Vec<LibraryNode>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub files: Vec<File>,
    pub selected: Option<Selection>,
    #[serde(default)]
    pub library: Vec<LibraryNode>,
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(skip)]
    pub library_revision: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 2,
            files: Vec::new(),
            selected: None,
            library: Vec::new(),
            expanded: Vec::new(),
            library_revision: 0,
        }
    }
}
#[derive(Default, Clone)]
pub struct Session(pub Arc<Mutex<(u64, Config)>>);
pub fn default_options() -> TraceOptions {
    TraceOptions {
        colors: 3,
        ..Default::default()
    }
}
impl Session {
    pub fn set_library(&self, revision: u64, library: Vec<LibraryNode>, expanded: Vec<String>) {
        if let Ok(mut state) = self.0.lock() {
            if revision < state.1.library_revision {
                return;
            }
            state.1.library_revision = revision;
            state.1.library = library;
            state.1.expanded = expanded;
        }
    }
    pub fn options(&self, path: &Path) -> TraceOptions {
        self.0
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .1
                    .files
                    .iter()
                    .find(|f| f.path == path)
                    .map(|f| f.options.clone())
            })
            .unwrap_or_else(default_options)
    }
    pub fn update(
        &self,
        revision: u64,
        selection: Option<Selection>,
        options: Option<TraceOptions>,
    ) {
        if let Ok(mut state) = self.0.lock() {
            if revision < state.0 {
                return;
            }
            state.0 = revision;
            if let Some(selected) = &selection {
                if let Some(options) = options.filter(|o| o.validate().is_ok()) {
                    if let Some(file) = state.1.files.iter_mut().find(|f| f.path == selected.path) {
                        file.options = options;
                    } else {
                        state.1.files.push(File {
                            path: selected.path.clone(),
                            options,
                            items: Vec::new(),
                        });
                    }
                }
            }
            state.1.selected = selection;
        }
    }
}
pub fn path() -> PathBuf {
    std::env::temp_dir().join("config.chromink.json")
}
pub fn load(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 8 << 20 {
        return Err("恢复配置超过 8 MB 上限".into());
    }
    let mut text = String::new();
    file.take((8 << 20) + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let mut config: Config =
        serde_json::from_str(&text).map_err(|e| format!("恢复配置无效：{e}"))?;
    if config.version != 1 && config.version != 2 {
        return Err("恢复配置版本不受支持".into());
    }
    config.version = 2;
    for file in &mut config.files {
        file.options.colors = file.options.colors.clamp(1, 32);
        if ["as", "nq", "km"].contains(&file.options.quantization.as_str()) {
            file.options.quantization = "wl".into();
        }
        file.options.vecline_iterations = file.options.vecline_iterations.min(32);
        file.options.lloyd_iterations = file.options.lloyd_iterations.min(32);
        if !["mean", "dominant", "median"].contains(&file.options.vecline_strategy.as_str()) {
            file.options.vecline_strategy = "mean".into();
        }
        if file.options.transparency == "transparent" {
            file.options.transparency = "off".into();
        }
        file.options.prescale = file.options.prescale.clamp(0.5, 8.);
        file.options.smooth_corners = file.options.smooth_corners.clamp(0., 1.3);
        if file.path.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case("gif")
                || e.eq_ignore_ascii_case("tif")
                || e.eq_ignore_ascii_case("tiff")
        }) {
            for item in &mut file.items {
                if item.is_none() {
                    *item = Some((0, u32::MAX));
                }
            }
        }
    }
    let mut unique = std::collections::HashSet::new();
    config.files.retain(|file| {
        file.path.is_absolute()
            && file.options.validate().is_ok()
            && unique.insert(file.path.clone())
    });
    Ok(config)
}
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径无效")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, config).map_err(|e| e.to_string())?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
