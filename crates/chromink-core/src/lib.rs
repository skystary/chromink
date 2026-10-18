pub mod control;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod outline;
#[cfg(feature = "preview")]
pub mod raster;
#[derive(Clone)]
pub struct TraceLayer {
    pub gradient: Option<Gradient>,
    pub stroke: f64,
    pub evenodd: bool,
    pub stroke_only: bool,
    pub clip: Option<Vec<outline::Curve>>,
    pub svg_shapes: Vec<Option<String>>,
    pub color: [u8; 4],
    pub curves: Vec<outline::Curve>,
}
#[derive(Clone)]
pub struct Gradient {
    pub radial: bool,
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub stops: Vec<(f64, [u8; 3])>,
}
pub struct TraceDocument {
    pub svg: String,
    pub layers: Vec<TraceLayer>,
    pub view_width: u32,
    pub view_height: u32,
}
pub type ColorIndex = u16;
pub const TRANSPARENT: ColorIndex = u16::MAX;
mod lloyd;
mod matching;
pub mod parallel;
mod perceptual;
#[cfg(feature = "preview")]
pub mod preview_png;
mod quantize;
pub mod resample;
mod threshold;
mod vecline;
pub mod vecline_trace;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};
/// Absolute decoder safety ceiling; runtime budgets normally impose a lower limit.
pub const MAX_PIXELS: u64 = 128_000_000;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceOptions {
    pub colors: u16,
    #[serde(deserialize_with = "read_quantizer")]
    pub quantization: String,
    pub dither: String,
    pub prescale: f64,
    pub despeckle: usize,
    pub smooth_corners: f64,
    pub optimize_paths: f64,
    pub stacked: bool,
    pub transparency: String,
    pub lloyd_iterations: u8,
    pub vecline_iterations: u8,
    pub vecline_strategy: String,
    pub tracing: String,
    pub mono_threshold: String,
    pub mono_cutoff: u8,
    pub mono_invert: bool,
    pub mono_adaptive_window: u16,
    pub mono_adaptive_percent: u8,
    #[serde(flatten)]
    pub vecline_trace: vecline_trace::Options,
}
fn read_quantizer<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    Ok(if matches!(value.as_str(), "km" | "nq" | "as") {
        "wl".into()
    } else {
        value
    })
}
impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            colors: 3,
            quantization: "mc".into(),
            dither: "none".into(),
            prescale: 1.,
            despeckle: 2,
            smooth_corners: 0.,
            optimize_paths: 5.,
            stacked: true,
            transparency: "white".into(),
            lloyd_iterations: 4,
            vecline_iterations: 4,
            vecline_strategy: "mean".into(),
            tracing: "potrace".into(),
            mono_threshold: "auto".into(),
            mono_cutoff: 128,
            mono_invert: false,
            mono_adaptive_window: 0,
            mono_adaptive_percent: 15,
            vecline_trace: vecline_trace::Options::default(),
        }
    }
}
impl TraceOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=32).contains(&self.colors) {
            return Err("颜色数必须在 1–32 之间".into());
        }
        if !["potrace", "vecline"].contains(&self.tracing.as_str()) {
            return Err("未知描摹方式".into());
        }
        if !["auto", "fixed", "adaptive"].contains(&self.mono_threshold.as_str())
            || self.mono_adaptive_percent > 100
        {
            return Err("单色阈值模式或局部阈值比例无效".into());
        }
        self.vecline_trace.validate()?;
        if !["mc", "vc", "wl"].contains(&self.quantization.as_str()) {
            return Err("未知减色算法".into());
        }
        if self.lloyd_iterations > 32 {
            return Err("Lloyd 迭代次数必须在 0–32 之间".into());
        }
        if self.vecline_iterations > 32
            || !["mean", "dominant", "median"].contains(&self.vecline_strategy.as_str())
        {
            return Err("Vecline 精修次数或取色策略无效".into());
        }
        if !["none", "floydsteinberg", "riemersma"].contains(&self.dither.as_str()) {
            return Err("未知拟色模式".into());
        }
        if !["off", "white"].contains(&self.transparency.as_str()) {
            return Err("未知透明图片预览模式".into());
        }
        if !self.prescale.is_finite() || !(0.5..=8.).contains(&self.prescale) {
            return Err("描摹精度必须在 0.5–8 之间".into());
        }
        if self.despeckle > 1000 {
            return Err("斑点过滤必须在 0–1000 之间".into());
        }
        if !self.smooth_corners.is_finite() || !(0. ..=1.3).contains(&self.smooth_corners) {
            return Err("转角平滑必须在 0–1.3 之间".into());
        }
        if !self.optimize_paths.is_finite() || !(0. ..=5.).contains(&self.optimize_paths) {
            return Err("路径优化必须在 0–5 之间".into());
        }
        Ok(())
    }
}
pub fn load_image(path: &Path) -> Result<RgbaImage, String> {
    let reader = image::io::Reader::open(path)
        .map_err(|e| format!("无法打开图片：{e}"))?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("无法读取图片尺寸：{e}"))?;
    control::validate_decode(w, h)?;
    image::open(path)
        .map(image::DynamicImage::into_rgba8)
        .map_err(|e| format!("无法解码图片：{e}"))
}
pub fn image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    image::image_dimensions(path).map_err(|e| format!("无法读取图片尺寸：{e}"))
}
pub fn trace(path: &Path, options: &TraceOptions) -> Result<String, String> {
    trace_with_parallelism(path, options, &parallel::Parallelism::new(1))
}
pub fn trace_with_parallelism(
    path: &Path,
    options: &TraceOptions,
    workers: &parallel::Parallelism,
) -> Result<String, String> {
    options.validate()?;
    let source = workers.install(|| load_image(path))?;
    trace_source_with_parallelism(&source, options, workers)
}
/// Trace a decoded input without re-reading the file on every parameter change.
pub fn trace_source_with_parallelism(
    source: &RgbaImage,
    options: &TraceOptions,
    workers: &parallel::Parallelism,
) -> Result<String, String> {
    trace_document(
        source,
        options,
        workers,
        &control::WorkControl::new(
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            control::memory_budget(),
        ),
    )
    .map(|document| document.svg)
}
pub fn trace_document(
    source: &RgbaImage,
    options: &TraceOptions,
    workers: &parallel::Parallelism,
    control: &control::WorkControl,
) -> Result<TraceDocument, String> {
    control.check()?;
    options.validate()?;
    let (original_w, original_h) = source.dimensions();
    control::validate_decode(original_w, original_h)?;
    let w = (original_w as f64 * options.prescale).round().max(1.) as u32;
    let h = (original_h as f64 * options.prescale).round().max(1.) as u32;
    control::validate_decode(w, h)?;
    let source_bytes = (u64::from(original_w) * u64::from(original_h) * 4) as usize;
    let pixels = (u64::from(w) * u64::from(h)) as usize;
    let quant_bytes = 28; // Additional full-Lloyd scratch is reserved within its reducer.
    control.claim(
        source_bytes
            .saturating_add(pixels.saturating_mul(quant_bytes + 2))
            .saturating_add(32 << 20),
    )?;
    let rgba = if w == original_w && h == original_h {
        std::borrow::Cow::Borrowed(source)
    } else {
        std::borrow::Cow::Owned(resample::resize(source, w, h, workers, control)?)
    };
    control.check()?;
    let vecline_options = options.vecline_trace.resolved(&rgba);
    let prepared = if options.tracing == "vecline" {
        std::borrow::Cow::Owned(
            if vecline_options.vc_mode == "trace" || vecline_options.vc_mode == "auto" {
                vecline_trace::prepare(&rgba, &vecline_options, control)?
            } else {
                rgba.into_owned()
            },
        )
    } else {
        rgba
    };
    let (palette, pixels) =
        workers.install(|| quantize::reduce_controlled(&prepared, options, control))?;
    let w = w as usize;
    let h = h as usize;
    let mut svg = String::new();
    let width = original_w;
    let height = original_h;
    write!(svg,"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}px\" height=\"{height}px\" viewBox=\"0 0 {w} {h}\">").unwrap();
    if options.tracing == "vecline" {
        let layers = workers.install(|| {
            vecline_trace::trace(
                &palette,
                &pixels,
                &prepared,
                &vecline_options,
                options.colors == 1,
                control,
            )
        })?;
        let mut layers = layers;
        vecline_trace::emit::round_scene(&mut layers, h, options.vecline_trace.vc_precision);
        svg.push_str(&vecline_trace::emit::layers(
            &layers,
            h,
            options.vecline_trace.vc_precision,
        ));
        svg.push_str("</svg>");
        control.claim(svg.len())?;
        control.check()?;
        return Ok(TraceDocument {
            svg,
            layers,
            view_width: w as u32,
            view_height: h as u32,
        });
    }
    // 半透明图层不能堆叠，否则会改变合成后的透明度。
    let stacked = options.stacked && palette.iter().all(|p| p[3] == 255);
    let layers: Vec<_> = palette.iter().enumerate().collect();
    let layer_limit = (control.remaining() / ((w * h).saturating_mul(5) / 4 + (16 << 20)))
        .max(1)
        .min(workers.maximum());
    let paths = workers.map_limit(
        &layers,
        layer_limit,
        |&(layer, color)| -> Result<(String, TraceLayer), String> {
            control.check()?;
            let mut svg = String::new();
            let empty = || TraceLayer {
                gradient: None,
                stroke: 0.,
                evenodd: false,
                stroke_only: false,
                clip: None,
                svg_shapes: Vec::new(),
                color: *color,
                curves: Vec::new(),
            };
            if color[3] == 0 {
                return Ok((svg, empty()));
            }
            let _scratch = control.reserve(w * h + w.div_ceil(64) * h * 16)?;
            let mut mask = Vec::with_capacity(pixels.len());
            for chunk in pixels.chunks(8192) {
                control.check()?;
                mask.extend(chunk.iter().map(|&pixel| {
                    pixel != TRANSPARENT
                        && (usize::from(pixel) == layer || (stacked && usize::from(pixel) > layer))
                }));
            }
            let curves = outline::trace_controlled(
                &mask,
                w,
                h,
                options.despeckle,
                options.smooth_corners,
                options.optimize_paths,
                workers,
                control,
            )?;
            if !curves.is_empty() {
                control.check()?;
                let data = outline::path_data(&curves, h);
                control.claim(data.len())?;
                write!(
                    svg,
                    "<path fill=\"#{:02x}{:02x}{:02x}\" fill-opacity=\"{:.4}\" d=\"{data}\"/>",
                    color[0],
                    color[1],
                    color[2],
                    color[3] as f64 / 255.
                )
                .unwrap();
            }
            control.check()?;
            Ok((
                svg,
                TraceLayer {
                    gradient: None,
                    stroke: 0.,
                    evenodd: false,
                    stroke_only: false,
                    clip: None,
                    svg_shapes: Vec::new(),
                    color: *color,
                    curves,
                },
            ))
        },
    );
    let mut scene = Vec::with_capacity(paths.len());
    for path in paths {
        let (path, layer) = path?;
        svg.push_str(&path);
        scene.push(layer);
    }
    svg.push_str("</svg>");
    control.check()?;
    Ok(TraceDocument {
        svg,
        layers: scene,
        view_width: w as u32,
        view_height: h as u32,
    })
}
