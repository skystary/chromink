//! The GUI executable dispatches here before constructing Tauri or touching session state.
use clap::{Parser, Subcommand, ValueEnum};
use chromink_core::{
    control::{memory_budget, WorkControl},
    parallel::Parallelism,
    trace_document, TraceOptions,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

#[derive(Parser)]
#[command(
    name = "Chromink",
    version,
    about = "彩色 SVG 描摹：同一应用支持桌面和无头模式",
    long_about = "不带参数启动桌面。带子命令时无头执行，不启动 WebView，不读写桌面会话。\n支持位图、多帧 GIF、多页 TIFF、图片 PDF；PDF 仅提取实际使用的图片，不渲染文字和矢量。\n算法、解码与文件处理均内置，无需 Python、ImageMagick、Potrace 或 PDF 软件。",
    after_long_help = "快速开始：\n  Chromink trace art.png --colors 3 --directory output\n  Chromink trace -i \"scans/*.png\" --precision 1.5 --json\n  Chromink inspect scan.pdf --json\n  Chromink trace scan.pdf --pages 1,3-5 --json\n  Chromink trace art.png --stdout > art.svg\n  Chromink schema\n\n学习接口：Chromink trace --help / inspect --help / schema。\nWindows GUI 子系统 EXE：自动化建议重定向 stdout/stderr 并等待进程。\nPowerShell：& .\\Chromink.exe trace art.png --json | Out-String\ncmd：start /wait \"\" Chromink.exe trace art.png --json\n退出码：0 成功/帮助；1 处理或 I/O 失败（批量可能部分成功）；2 参数错误。\nGPU 环境变量：CHROMINK_GPU=off；CHROMINK_GPU_BACKEND=auto|dx12|vulkan|metal；CHROMINK_GPU_LOG=1（stderr 诊断）。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// 描摹并保存 SVG；支持通配符、批量和多页，默认不会覆盖文件
    Trace(TraceArgs),
    /// 列出可以处理的图片及原始尺寸，不执行描摹、不产生 SVG
    Inspect(Inputs),
    /// 输出 JSON 接口说明、默认值、范围、结果结构和退出码，供 AI/脚本使用
    Schema,
}
#[derive(clap::Args)]
struct Inputs {
    /// 输入文件或带引号的通配符，例如 "images/*.png"；按给定顺序去重
    #[arg(value_name="INPUT", num_args=0..)]
    files: Vec<String>,
    /// 与位置参数等价，可重复使用；支持同一 PDF 的多个图片对象
    #[arg(short='i',long="input",num_args=1..,value_name="INPUT")]
    inputs: Vec<String>,
    /// PDF 页码，1 开始，逗号列表或闭区间，例如 1,3-5；默认所有包含图片的页
    #[arg(long, value_parser=parse_selection)]
    pages: Option<Selection>,
    /// GIF/TIFF 帧或页，1 开始，例如 1,2-4；默认所有帧/页
    #[arg(long, value_parser=parse_selection)]
    frames: Option<Selection>,
    /// stdout 输出一个 JSON 对象（schemaVersion、success、results、errors），不混入日志
    #[arg(long)]
    json: bool,
}
#[derive(clap::Args)]
struct TraceArgs {
    #[command(flatten)]
    engine: EngineArgs,
    #[command(flatten)]
    inputs: Inputs,
    /// SVG 输出目录，默认当前工作目录；每项一个文件
    #[arg(short = 'd', long, default_value = ".")]
    directory: PathBuf,
    /// 输出文件或模板；多项必须含 {stem}、{page}、{image} 或 {frame}；默认自动命名
    #[arg(
        short = 'o',
        long,
        value_name = "PATH_OR_TEMPLATE",
        conflicts_with = "stdout"
    )]
    output: Option<String>,
    /// 仅一个图片时，将 SVG 原文写入 stdout，不写输出文件；不可与 --json 一起使用
    #[arg(long,conflicts_with_all=["json","overwrite"])]
    stdout: bool,
    /// 明确允许原子替换同名文件；默认遇到已存在的输出返回错误，原文件不变
    #[arg(long)]
    overwrite: bool,
    /// 颜色数 1–32；1 为单色（明亮区域透明），更多颜色通常增加路径和体积
    #[arg(short='c',long,default_value_t=3,value_parser=clap::value_parser!(u16).range(1..=32))]
    colors: u16,
    /// 减色算法：median-cut、vecline（RGB Wu + 分桶 Oklab Lloyd）、oklab-lloyd（Oklab Wu + 完整 Lloyd）
    #[arg(short = 'q', long, value_enum, default_value = "median-cut")]
    quantization: Quantizer,
    /// Vecline 平均色精修次数 0–32；0 跳过精修
    #[arg(long,default_value_t=4,value_parser=clap::value_parser!(u8).range(0..=32))]
    vecline_iterations: u8,
    /// Oklab Wu 后的完整 Lloyd 迭代次数 0–32；0 仅运行 Oklab Wu
    #[arg(long, default_value = "4")]
    lloyd_iterations: u8,
    /// Vecline 取色策略：mean、dominant、median；后两者不精修
    #[arg(long, value_enum, default_value = "mean")]
    vecline_strategy: VeclineStrategy,
    /// 误差扩散拟色：none、floyd-steinberg、riemersma；可能增加路径，拟色在 CPU 执行
    #[arg(long, value_enum, default_value = "none")]
    dither: Dither,
    /// 描摹精度 0.5–8，默认 1；改变内部采样，SVG 输出保留输入尺寸
    #[arg(short = 'p', long, alias = "prescale", default_value_t = 1.)]
    precision: f64,
    /// 斑点过滤 0–1000；过滤小区域，0 保留所有区域
    #[arg(long,default_value_t=2,value_parser=clap::value_parser!(u32).range(0..=1000))]
    despeckle: u32,
    /// 转角平滑 0–1.3，默认 0；0 全部生成角点，跳过平滑判定和曲线合并
    #[arg(long, default_value_t = 0.)]
    smooth_corners: f64,
    /// 路径优化容差 0–5，默认 5，内部位图像素单位；0 或平滑为 0 时跳过合并
    #[arg(long, default_value_t = 5.)]
    optimize_paths: f64,
    /// 关闭默认的颜色堆叠，分别输出颜色区域
    #[arg(long)]
    no_stack: bool,
    /// 工作线程数，默认可用逻辑 CPU 数；文件顺序处理，每项内并行以限制峰值内存
    #[arg(short='j',long,value_parser=clap::value_parser!(u16).range(1..=256))]
    threads: Option<u16>,
}
#[derive(Clone, ValueEnum)]
enum Quantizer {
    #[value(alias = "mc")]
    MedianCut,
    #[value(alias = "vc", alias = "wu-oklab")]
    Vecline,
    #[value(name = "oklab-lloyd", alias = "wl")]
    OklabLloyd,
}
#[derive(Clone, ValueEnum)]
enum VeclineStrategy {
    Mean,
    Dominant,
    Median,
}
#[derive(Clone, ValueEnum)]
enum Dither {
    None,
    #[value(alias = "floydsteinberg")]
    FloydSteinberg,
    Riemersma,
}
#[derive(clap::Args)]
struct EngineArgs {
    /// Independent tracing engine, combined with the selected reducer.
    #[arg(long,default_value="potrace",value_parser=["potrace","vecline"])]
    tracing: String,
    /// Vecline geometry after reduction: trace curves, auto, pixel rectangles, exact contours. Never embeds a bitmap.
    #[arg(long,default_value="trace",value_parser=["trace","auto","pixel","exact"])]
    vc_mode: String,
    /// Official content preset; shared palette size and reducer are preserved. Other stages use preset defaults.
    #[arg(long,default_value="default",value_parser=["default","logo","lineart","poster","photo","detailed","clean"])]
    vc_preset: String,
    /// Vecline monochrome: Otsu, fixed Rec.709 cutoff, or Bradley local threshold.
    #[arg(long,default_value="auto",value_parser=["auto","fixed","adaptive"])]
    mono_threshold: String,
    /// Fixed monochrome cutoff, 0..255; used only by Vecline reduction at one color.
    #[arg(long, default_value = "128")]
    mono_cutoff: u8,
    /// Douglas-Peucker tolerance in internal bitmap pixels, 0..10; overrides preset.
    #[arg(long)]
    vc_tolerance: Option<f64>,
    /// Bezier fitting error in internal pixels, 0..10; overrides preset.
    #[arg(long)]
    vc_fit_error: Option<f64>,
    /// Absorb connected regions smaller than this area, 0..100000; overrides preset.
    #[arg(long)]
    vc_min_area: Option<u32>,
    /// Curve merge error in internal pixels, 0..10; overrides preset; zero disables merging.
    #[arg(long)]
    vc_optimize_error: Option<f64>,
    /// Reconstruct gradients in multi-color curve mode; true/false overrides preset.
    #[arg(long,action=clap::ArgAction::Set)]
    vc_gradients: Option<bool>,
}
impl EngineArgs {
    fn apply(&self, o: &mut TraceOptions) {
        o.tracing = self.tracing.clone();
        o.mono_threshold = self.mono_threshold.clone();
        o.mono_cutoff = self.mono_cutoff;
        o.vecline_trace.vc_mode = self.vc_mode.clone();
        o.vecline_trace.set_preset(&self.vc_preset);
        if let Some(v) = self.vc_tolerance {
            o.vecline_trace.vc_tolerance = v;
        }
        if let Some(v) = self.vc_fit_error {
            o.vecline_trace.vc_fit_error = v;
        }
        if let Some(v) = self.vc_min_area {
            o.vecline_trace.vc_min_area = v;
        }
        if let Some(v) = self.vc_optimize_error {
            o.vecline_trace.vc_optimize_error = v;
        }
        if let Some(v) = self.vc_gradients {
            o.vecline_trace.vc_gradients = v;
        }
    }
}
impl TraceArgs {
    fn options(&self) -> TraceOptions {
        let mut options = TraceOptions {
            colors: self.colors,
            quantization: match self.quantization {
                Quantizer::MedianCut => "mc",
                Quantizer::OklabLloyd => "wl",
                Quantizer::Vecline => "vc",
            }
            .into(),
            dither: match self.dither {
                Dither::None => "none",
                Dither::FloydSteinberg => "floydsteinberg",
                Dither::Riemersma => "riemersma",
            }
            .into(),
            prescale: self.precision,
            despeckle: self.despeckle as usize,
            smooth_corners: self.smooth_corners,
            optimize_paths: self.optimize_paths,
            stacked: !self.no_stack,
            vecline_iterations: self.vecline_iterations,
            lloyd_iterations: self.lloyd_iterations,
            vecline_strategy: match self.vecline_strategy {
                VeclineStrategy::Mean => "mean",
                VeclineStrategy::Dominant => "dominant",
                VeclineStrategy::Median => "median",
            }
            .into(),
            ..Default::default()
        };
        self.engine.apply(&mut options);
        options
    }
}
#[derive(Clone)]
struct Selection(Vec<u32>);
fn parse_selection(s: &str) -> Result<Selection, String> {
    parse_ranges(s).map(Selection)
}
fn parse_ranges(s: &str) -> Result<Vec<u32>, String> {
    let mut result = Vec::new();
    for part in s.split(',') {
        let bounds = part.split('-').collect::<Vec<_>>();
        if bounds.len() > 2 {
            return Err("页码格式为 1,3-5".into());
        }
        let first = bounds[0].parse::<u32>().map_err(|_| "页码必须是正整数")?;
        let last = if bounds.len() == 2 {
            bounds[1].parse::<u32>().map_err(|_| "页码必须是正整数")?
        } else {
            first
        };
        if first == 0
            || last < first
            || last > 1_000_000
            || last - first > 4096
            || result.len() + (last - first + 1) as usize > 4096
        {
            return Err("页码必须从 1 开始，最多选择 4096 项".into());
        }
        result.extend(first..=last);
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    input: PathBuf,
    page: Option<u32>,
    image: Option<u32>,
    frame: Option<u32>,
    width: u32,
    height: u32,
    #[serde(skip)]
    bitmap: Option<super::pdf::Bitmap>,
}
#[derive(Serialize)]
struct Failure {
    input: Option<PathBuf>,
    error: String,
}
fn gather(args: &Inputs) -> (Vec<Item>, Vec<Failure>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for input in args.files.iter().chain(&args.inputs) {
        if input.contains('*') || input.contains('?') {
            match glob::glob(input) {
                Ok(paths) => {
                    let mut found = false;
                    for path in paths {
                        match path {
                            Ok(path) => {
                                files.push(path);
                                found = true
                            }
                            Err(e) => errors.push(Failure {
                                input: None,
                                error: e.to_string(),
                            }),
                        }
                    }
                    if !found {
                        errors.push(Failure {
                            input: None,
                            error: format!("没有匹配的文件：{input}"),
                        });
                    }
                }
                Err(e) => errors.push(Failure {
                    input: None,
                    error: e.to_string(),
                }),
            }
        } else {
            files.push(PathBuf::from(input));
        }
    }
    if files.is_empty() && errors.is_empty() {
        errors.push(Failure {
            input: None,
            error: "至少提供一个 INPUT 或 --input".into(),
        });
    }
    let mut seen = HashSet::new();
    files.retain(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())));
    let mut items = Vec::new();
    for input in files {
        let scanned = (|| -> Result<Vec<Item>, String> {
            if !super::input_supported(&input) {
                return Err("不支持此格式；仅支持位图和图片 PDF，拒绝 SVG 和 ICO".into());
            }
            let is_pdf = input
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("pdf"));
            if is_pdf {
                if args.frames.is_some() {
                    return Err("PDF 使用 --pages，不能使用 --frames".into());
                }
                let bitmaps = super::pdf::images(&input)?;
                let filtered: Vec<_> = bitmaps
                    .into_iter()
                    .filter(|b| {
                        args.pages
                            .as_ref()
                            .is_none_or(|p| p.0.contains(&(b.page + 1)))
                    })
                    .collect();
                if filtered.is_empty() {
                    return Err("所选 PDF 页中没有可处理的图片（文字和矢量已忽略）".into());
                }
                Ok(filtered
                    .into_iter()
                    .map(|b| Item {
                        input: input.clone(),
                        page: Some(b.page + 1),
                        image: Some(b.ordinal + 1),
                        frame: None,
                        width: b.width,
                        height: b.height,
                        bitmap: Some(b),
                    })
                    .collect())
            } else {
                if args.pages.is_some() {
                    return Err("--pages 仅用于 PDF；GIF/TIFF 请用 --frames".into());
                }
                let dimensions = if super::frames::supported(&input) {
                    super::frames::dimensions(&input)?
                } else {
                    vec![chromink_core::image_dimensions(&input)?]
                };
                let multiple = super::frames::supported(&input);
                let selected: Vec<_> = dimensions
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        args.frames
                            .as_ref()
                            .is_none_or(|f| f.0.contains(&(*i as u32 + 1)))
                    })
                    .collect();
                if selected.is_empty() {
                    return Err("所选帧不存在".into());
                }
                Ok(selected
                    .into_iter()
                    .map(|(i, (width, height))| Item {
                        input: input.clone(),
                        page: None,
                        image: None,
                        frame: multiple.then_some(i as u32 + 1),
                        width,
                        height,
                        bitmap: None,
                    })
                    .collect())
            }
        })();
        match scanned {
            Ok(mut scanned) => items.append(&mut scanned),
            Err(error) => errors.push(Failure {
                input: Some(input),
                error,
            }),
        }
    }
    (items, errors)
}
fn filename(item: &Item, template: Option<&str>) -> String {
    let stem = item.input.file_stem().unwrap_or_default().to_string_lossy();
    if let Some(template) = template {
        return template
            .replace("{stem}", &stem)
            .replace("{page}", &item.page.unwrap_or(1).to_string())
            .replace("{image}", &item.image.unwrap_or(1).to_string())
            .replace("{frame}", &item.frame.unwrap_or(1).to_string());
    }
    if let Some(page) = item.page {
        format!("{stem}-page-{page}-image-{}.svg", item.image.unwrap())
    } else if let Some(frame) = item.frame {
        format!("{stem}-frame-{frame}.svg")
    } else {
        format!("{stem}.svg")
    }
}
fn save(path: &Path, svg: &str, overwrite: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(svg.as_bytes()).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    if overwrite {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    }
    .map_err(|e| {
        format!(
            "无法保存 {}：{}（覆盖需 --overwrite）",
            path.display(),
            e.error
        )
    })?;
    Ok(())
}
fn emit(value: &serde_json::Value) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, value).map_err(|e| e.to_string())?;
    writeln!(out).map_err(|e| e.to_string())
}
fn report(command: &str, results: Vec<serde_json::Value>, errors: Vec<Failure>, json: bool) -> i32 {
    let failed = !errors.is_empty();
    if json {
        if let Err(e) = emit(
            &serde_json::json!({"schemaVersion":1,"command":command,"success":!failed,"results":results,"errors":errors}),
        ) {
            eprintln!("{e}");
            return 1;
        }
    } else {
        for result in results {
            println!("{}", result);
        }
        for error in errors {
            eprintln!(
                "{}{}",
                error
                    .input
                    .map(|p| format!("{}: ", p.display()))
                    .unwrap_or_default(),
                error.error
            );
        }
    }
    i32::from(failed)
}
fn invalid(command: &str, json: bool, error: &str) -> i32 {
    if json {
        let _ = emit(
            &serde_json::json!({"schemaVersion":1,"command":command,"success":false,"results":[],"errors":[{"input":null,"error":error}]}),
        );
    } else {
        eprintln!("{error}");
    }
    2
}
fn trace(args: TraceArgs) -> i32 {
    let options = args.options();
    if let Err(error) = options.validate() {
        if args.inputs.json {
            let _ = emit(
                &serde_json::json!({"schemaVersion":1,"command":"trace","success":false,"results":[],"errors":[{"input":null,"error":error}]}),
            );
        } else {
            eprintln!("{error}");
        }
        return 2;
    }
    if args.inputs.files.is_empty() && args.inputs.inputs.is_empty() {
        return invalid("trace", args.inputs.json, "至少提供一个 INPUT 或 --input");
    }
    let (items, mut errors) = gather(&args.inputs);
    if args.stdout && (items.len() != 1 || !errors.is_empty()) {
        eprintln!("--stdout 必须恰好选择一个有效图片");
        return 2;
    }
    if items.len() > 1
        && args.output.as_ref().is_some_and(|t| {
            !["{stem}", "{page}", "{image}", "{frame}"]
                .iter()
                .any(|p| t.contains(p))
        })
    {
        return invalid(
            "trace",
            args.inputs.json,
            "多项 --output 必须使用模板占位符，或改用 --directory",
        );
    }
    let workers = Parallelism::new(args.threads.map(usize::from).unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    }));
    workers.set_active(workers.maximum());
    let sources: HashSet<_> = items
        .iter()
        .filter_map(|item| std::fs::canonicalize(&item.input).ok())
        .collect();
    let mut results = Vec::new();
    let mut targets = HashSet::new();
    for item in items {
        let start = Instant::now();
        let output = args.directory.join(filename(&item, args.output.as_deref()));
        let result = (|| -> Result<(String, f64), String> {
            if !args.stdout
                && std::fs::canonicalize(&output).is_ok_and(|path| sources.contains(&path))
            {
                return Err("输出路径指向输入文件，拒绝覆盖原图".into());
            }
            if !args.stdout && !targets.insert(output.clone()) {
                return Err("输出名称冲突；请更换目录或模板".into());
            }
            let control = WorkControl::new(Arc::new(AtomicBool::new(false)), memory_budget());
            let source = if let Some(bitmap) = &item.bitmap {
                super::pdf::decode(&item.input, bitmap, &workers, &control)?.into_rgba8()
            } else if let Some(frame) = item.frame {
                super::frames::decode(&item.input, frame - 1)?.into_rgba8()
            } else {
                chromink_core::load_image(&item.input)?
            };
            let trace_start = Instant::now();
            let document = trace_document(&source, &options, &workers, &control)?;
            let trace_ms = trace_start.elapsed().as_secs_f64() * 1000.;
            if !args.stdout {
                save(&output, &document.svg, args.overwrite)?;
            }
            Ok((document.svg, trace_ms))
        })();
        match result {
            Ok((svg, trace_ms)) => {
                if args.stdout {
                    let mut stdout = std::io::stdout().lock();
                    if let Err(error) = stdout.write_all(svg.as_bytes()) {
                        eprintln!("{error}");
                        return 1;
                    }
                } else {
                    let mut metadata = serde_json::to_value(&item).unwrap();
                    metadata["output"] =
                        serde_json::json!(std::fs::canonicalize(&output).unwrap_or(output));
                    metadata["bytes"] = serde_json::json!(svg.len());
                    metadata["paths"] = serde_json::json!(svg.matches("<path").count());
                    metadata["elapsedMs"] = serde_json::json!(start.elapsed().as_millis());
                    metadata["traceMs"] = serde_json::json!(trace_ms);
                    results.push(metadata);
                }
            }
            Err(error) => errors.push(Failure {
                input: Some(item.input.clone()),
                error,
            }),
        }
    }
    report("trace", results, errors, args.inputs.json)
}
fn schema() -> serde_json::Value {
    let mut schema = serde_json::json!({
 "schemaVersion":1,"application":"Chromink","version":env!("CARGO_PKG_VERSION"),"launch":{"desktop":"no arguments","headless":"trace | inspect | schema; no WebView or desktop-session changes"},
 "commands":{"trace":"trace INPUT... or trace --input INPUT...; one SVG per selected bitmap","inspect":"inspect INPUT... --json; dimensions/page/frame metadata without tracing","schema":"this machine-readable contract","help":"--help, trace --help, inspect --help"},
 "options":{"colors":{"default":3,"min":1,"max":32},"precision":{"default":1,"min":0.5,"max":8},"quantization":{"default":"median-cut","values":["median-cut","vecline","oklab-lloyd"],"aliases":["mc","vc","wl"]},"vecline-iterations":{"default":4,"min":0,"max":32},"vecline-strategy":{"default":"mean","values":["mean","dominant","median"]},"dither":{"default":"none","values":["none","floyd-steinberg","riemersma"]},"despeckle":{"default":2,"min":0,"max":1000},"smooth-corners":{"default":0,"min":0,"max":1.3,"guiStep":0.05,"zero":"all corners; skips smoothing tests and curve merging"},"optimize-paths":{"default":5,"min":0,"max":5,"guiStep":0.5,"unit":"internal bitmap pixels","zero":"skips curve merging","inactive":"smoothing is zero or contour contains only corners"},"threads":{"default":"logical CPU count","min":1,"max":256},"directory":{"default":"current working directory"},"output":"path or template {stem}/{page}/{image}/{frame}","overwrite":{"default":false},"stdout":"single SVG only; conflicts with json/output/overwrite","pages":"PDF: 1-based list/range; e.g. 1,3-5","frames":"GIF/TIFF: 1-based list/range","no-stack":{"default":false},"json":"single JSON report on stdout; diagnostics on stderr"},
 "formats":["png","jpg","jpeg","bmp","webp","gif","tif","tiff","ppm","pgm","pbm","tga","pdf"],"excluded":["svg","ico","text/vector-only PDF"],
 "pdf":"each painted bitmap is extracted at its native size; page layout/text/paths ignored; empty selected pages skipped; no selected bitmaps is an error",
 "dimensions":"SVG keeps input pixel dimensions; precision changes internal sampling","transparency":"source alpha preserved; monochrome bright regions may be transparent; no preview backing is exported",
 "removedOptions":{"background":"removed ineffective legacy option; SVG alpha remains unchanged"},
 "gpu":"automatic for high-color large images once background device initialization is ready; first task uses CPU; explicit backend requests initialize synchronously; dithering feedback uses CPU",
 "environment":{"CHROMINK_GPU":"off disables GPU","CHROMINK_GPU_BACKEND":"auto | dx12 | vulkan | metal; unsupported backend falls back to CPU","CHROMINK_GPU_LOG":"1 enables GPU diagnostics on stderr"},
 "report":{"schemaVersion":1,"command":"trace or inspect","success":"boolean; false on partial failure","results":"array: input,page,image,frame,width,height; trace adds output,bytes,paths,elapsedMs,traceMs","errors":"array: input (nullable),error"},
 "exitCodes":{"0":"success/help","1":"processing/I/O failure; possible partial batch success","2":"invalid arguments"},
 "examples":["Chromink trace art.png --colors 3 --directory out --json","Chromink inspect scan.pdf --json","Chromink trace scan.pdf --pages 1,3-5 --output {stem}-p{page}-i{image}.svg --json","Chromink trace art.png --stdout > art.svg"]});
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../crates/chromink-core/algorithm-parameters.json"
    ))
    .unwrap();
    for control in controls.as_array().unwrap() {
        let id = control["id"].as_str().unwrap();
        let mut flag = String::new();
        for ch in id.chars() {
            if ch.is_ascii_uppercase() {
                flag.push('-');
                flag.push(ch.to_ascii_lowercase());
            } else {
                flag.push(ch);
            }
        }
        let mut definition = serde_json::json!({"default":control["value"],"type":control["type"],"description":control["hint"],"availability":control["scope"]});
        for (from, to) in [
            ("min", "min"),
            ("max", "max"),
            ("step", "guiStep"),
            ("values", "values"),
        ] {
            if !control[from].is_null() {
                definition[to] = control[from].clone();
            }
        }
        schema["options"][flag] = definition;
    }
    schema["algorithmControls"] = controls;
    schema["combinations"] = serde_json::json!({"quantization":["mc","vc","wl"],"tracing":["potrace","vecline"],"independent":true,"veclineQuantization":"RGB Wu + histogram Oklab Lloyd","oklabLloyd":"Oklab Wu + full weighted Lloyd","booleanFlags":"--vc-gradients true or false; omitted controls use preset/default values"});
    schema["traceOptionsDefaults"] = serde_json::to_value(TraceOptions::default()).unwrap();
    schema["monochrome"] = serde_json::json!({"mc":"two trained colors then dark-color mask","wl":"Oklab Wu and full Lloyd train two classes; dark classes become ink","vc":"Otsu/fixed/Bradley luminance threshold; no artificial common threshold"});
    schema
}
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return code;
        }
    };
    match cli.command {
        Command::Trace(args) => trace(args),
        Command::Inspect(args) => {
            if args.files.is_empty() && args.inputs.is_empty() {
                return invalid("inspect", args.json, "至少提供一个 INPUT 或 --input");
            }
            let (items, errors) = gather(&args);
            report(
                "inspect",
                items
                    .iter()
                    .map(|i| serde_json::to_value(i).unwrap())
                    .collect(),
                errors,
                args.json,
            )
        }
        Command::Schema => match emit(&schema()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
    }
}
#[cfg(not(windows))]
pub fn attach_console() {}
#[cfg(windows)]
pub fn attach_console() {
    // Preserve inherited pipe/file handles while attaching to the caller's console.
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
        fn GetFileType(handle: isize) -> u32;
    }
    unsafe {
        let selectors = [(-10i32) as u32, (-11i32) as u32, (-12i32) as u32];
        let old = selectors.map(|key| GetStdHandle(key));
        let valid = old.map(|handle| handle != 0 && handle != -1 && GetFileType(handle) != 0);
        AttachConsole(u32::MAX);
        for (i, key) in selectors.into_iter().enumerate() {
            if valid[i] {
                SetStdHandle(key, old[i]);
            }
        }
    }
}
