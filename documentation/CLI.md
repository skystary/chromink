# Chromink 无头命令行

同一个应用可执行文件，无参数为桌面，有子命令为无头。不会启动 WebView、读取或改写桌面会话配置。运行时不需要 Python、Node、独立 CLI 或外部描摹/PDF 工具。使用程序所在目录中的同一 EXE 和随包 DLL。

先执行 Chromink --help、Chromink trace --help、Chromink inspect --help 学习人类可读说明；Chromink schema 输出 JSON 接口契约、参数默认值和范围，适合 AI 学习。

## Windows 调用

GUI 子系统 EXE 在部分终端不会自动等待。PowerShell 建议接入管道；自动化建议通过进程 API 重定向 stdout/stderr 并等待退出。cmd 使用 start /wait。stdout 为 UTF-8，支持中文和空格路径。

```powershell
& .\Chromink.exe trace '图像 sample.png' --colors 3 --directory output --json | Out-String
$LASTEXITCODE
& .\Chromink.exe inspect 'scan.pdf' --json | Out-String
& .\Chromink.exe trace 'scan.pdf' --pages '1,3-5' --output '{stem}-p{page}-i{image}.svg' --json | Out-String
& .\Chromink.exe trace --input 'images/*.png' --dither floyd-steinberg --threads 8 --directory output --json | Out-String
```

Linux/macOS 使用对应应用可执行文件执行相同子命令；无头模式不需要启动系统 WebView。

## 命令与参数

trace 生成 SVG；inspect 只列出可处理图片及其尺寸/页/帧；schema 输出机器契约。输入可用位置参数或 -i/--input；通配符必须加引号，由程序展开，去重并按给定顺序处理。支持 PNG、JPEG、BMP、WebP、GIF、TIFF、PNM、TGA、图片 PDF，拒绝 SVG/ICO。PDF 只提取实际绘制的图片对象，忽略文字、矢量、排版；纯文字 PDF 返回失败。一个页可输出多个图片，用 image 序号区分。

--pages 是 1 起始 PDF 页号列表/闭区间，例如 1,3-5；--frames 是 GIF/TIFF 帧或页。默认所有图片。无可处理图片时失败。--directory 默认当前工作目录；--output 为文件或模板，支持 {stem}、{page}、{image}、{frame}。多项不能共享没有占位符的单个输出文件。默认自动命名，多页/帧有序号；不覆盖已有文件，--overwrite 才允许原子替换；拒绝覆盖输入原图。

--stdout 仅选择一张图时输出 SVG 原文，不写文件，不能和 --json、--output、--overwrite 同用。程序不打印额外成功日志到 SVG stdout。

| 参数 | 默认 | 范围/意义 |
|---|---|---|
| --colors / -c | 3 | 1–32；1 为单色 |
| --precision / -p | 1 | 0.5–8；内部采样精度，SVG 保持输入像素尺寸 |
| --quantization / -q | median-cut | median-cut、vecline、oklab-lloyd；简写 mc/vc/wl |
| --tracing | potrace | potrace、vecline；与三种减色独立组合 |
| --vecline-iterations | 4 | 0–32；0 跳过精修，仅 mean 策略有效 |
| --lloyd-iterations | 4 | 0–32；Oklab Wu 后的完整颜色 Lloyd，0 仅运行 Oklab Wu |
| --vecline-strategy | mean | mean、dominant、median |
| --dither | none | none、floyd-steinberg、riemersma |
| --despeckle | 2 | 0–1000；小区域过滤 |
| --smooth-corners | 0 | 0–1.3；0 全为角点，跳过平滑判定和曲线合并 |
| --optimize-paths | 5 | 0–5；内部位图像素单位的曲线合并误差容限，0 跳过合并；平滑 0 或全角点也跳过 |
| --threads / -j | 可用逻辑 CPU 数 | 1–256；单项内并行，文件顺序处理 |
| --no-stack | false | 关闭默认颜色堆叠 |

Median cut 的颜色映射由 GPU 按工作规模自动选择，不能保证每项使用 GPU；否则回退到并行 CPU。两种 Lloyd 方案使用并行 CPU 和 Oklab 映射；原来的 RGB GPU 内核不适用于该距离公式。拟色的反馈计算使用 CPU。保留输入透明度，单色描摹的明亮区域可能变为透明；预览白底不导出。

## Vecline 描摹参数

0.9 界面及无头参数统一保留重点项，其他描摹阶段使用默认值或预设值。

| 参数 | 默认 | 用途 |
|---|---|---|
| --vc-mode | trace | trace 曲线、auto 自动、pixel 合并矩形、exact 精确轮廓 |
| --vc-preset | default | default、logo、lineart、poster、photo、detailed、clean |
| --vc-tolerance | 预设值，默认 0.4 | 轮廓简化像素容差，0–10 |
| --vc-fit-error | 预设值，默认 0.4 | 贝塞尔拟合像素误差，0–10 |
| --vc-min-area | 预设值，默认 0 | 小区域过滤面积，0–100000 |
| --vc-optimize-error | 预设值，默认 0.4 | 曲线合并像素误差，0–10；0 跳过合并 |
| --vc-gradients | 预设值，默认 false | 多色曲线模式的渐变重建，显式 true/false |
| --mono-threshold | auto | Vecline 单色：auto Otsu、fixed 固定、adaptive Bradley |
| --mono-cutoff | 128 | fixed 阈值的 Rec.709 亮度截止，0–255 |

pixel / exact 保留的是减色后的像素几何及输入透明度，不保证与原图颜色相同。模式均输出矢量几何，无 embed 模式；auto 按原图颜色数与行密度决定使用矩形或曲线。非默认图像预设在 auto 下选择曲线。preset 不修改共享色数或减色算法；其余五个重点参数可覆盖预设值。预设参数取自锁定的 Vecline 源码，不随网络版本自动变化。

```powershell
& .\Chromink.exe trace art.png --quantization wl --lloyd-iterations 4 --tracing potrace --json | Out-String
& .\Chromink.exe trace scan.png --quantization vc --colors 1 --mono-threshold adaptive --tracing vecline --vc-preset lineart --directory out --json | Out-String
& .\Chromink.exe trace logo.png --tracing vecline --vc-preset logo --vc-fit-error 0.5 --output logo.svg --json | Out-String
```

## 机器结果和退出码


--json 在 stdout 输出一个 JSON 对象，schemaVersion=1、command、success、results、errors；批量部分失败时 success=false，已成功文件保留。results 包含 input/page/image/frame/width/height，trace 增加 output/bytes/paths/elapsedMs/traceMs；errors 包含 input（可能为 null）和 error。解析参数错误由 clap 输出 stderr。成功为退出码 0，处理/I/O 错误为 1，参数错误为 2。traceMs 仅计解码后到 SVG 完成的核心处理时间；elapsedMs 包含解码与写盘。完整结构见 Chromink schema。

GPU 环境变量：CHROMINK_GPU=off 禁用；CHROMINK_GPU_BACKEND=auto|dx12|vulkan|metal 选择后端；CHROMINK_GPU_LOG=1 输出 stderr 诊断。默认仅在高色数大图上尝试 GPU，缺少设备、能力不足或运行失败自动走 CPU。

自动模式首次符合 GPU 条件时后台初始化，当前任务和设备尚未就绪的任务先用 CPU；显式后端同步初始化，方便诊断。单文件无头进程可能在 GPU 就绪前完成，因此自动模式不保证实际使用 GPU。
