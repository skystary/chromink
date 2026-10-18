# Chromink · 描彩

独立发布的 Rust + Tauri 2 位图减色与 SVG 描摹应用。无需安装 Python、ImageMagick、Potrace 或其他图像处理软件；算法来源及许可见 [第三方说明](documentation/THIRD_PARTY.md)。

## 使用

空白预览区点击或拖入文件。文件库在左下角展开，面板右下角添加按钮导入更多图片；拖到另一文件或组上组成分组，删除分组只解散分组。支持 PNG、JPEG、BMP、WebP、GIF、多页 TIFF、PNM、TGA、图片 PDF。不支持 SVG、ICO；PDF 只提取实际绘制的图片，忽略文字和其他内容。

添加或修改参数后自动描摹当前图片。预览支持卷帘比较、滚轮中心缩放、左键拖动，以及双击切换适应窗口和原图大小。Logo 切换矢量结果的白色预览衬底，不修改原图或导出透明度。系统深浅色自动匹配。

底部三组胶囊：

1. 基础：色数、描摹精度、减色算法、拟色、描摹方式。
2. 减色：两种 Lloyd 的迭代次数、Vecline 单色阈值与截止亮度；不适用时置灰，其他使用默认值。
3. 描摹与保存：Potrace 显示过滤、平滑、优化；Vecline 显示 mode、preset、简化、拟合、小区域过滤、合并、渐变。切换描摹方式时替换这组参数。

所有触发区为 40 × 40 px。悬停看说明或数值，滚轮修改，点击数字输入，双击恢复默认。色数 1–32，默认 3；描摹精度 0.5–8。

减色算法：Median cut、Vecline RGB Wu + 加权 Oklab Lloyd、Oklab Wu + 完整加权 Lloyd。描摹算法：Potrace 1.16 的 Rust 重写及 Vecline 的 Rust 重写；三种减色与两种描摹互相组合。

Vecline 模式：曲线、自动、像素矩形、精确轮廓；后两者保留减色后的像素几何，不保证恢复减色前的原图。自动依据原图颜色数与行密度选择矩形或曲线。预设：默认、标志、线稿、海报、照片、细节、清理，采用锁定上游的描摹/预处理设置；应用的色数和减色选择独立保留。更改预设重置关键描摹参数，随后可以微调。没有位图嵌入模式。

保存按钮左键保存到应用目录下 chromink 文件夹，同名追加编号；右键另存为，默认下载目录。右下角通知汇总错误和保存结果；点击保存消息打开文件位置。启动恢复文件库及每个源文件的一套参数，配置保存在系统临时目录 config.chromink.json。自动预览不写临时 SVG。

## 命令行

同一个 EXE 支持无头执行，不启动 WebView：

```powershell
& .\Chromink.exe --help | Out-String
& .\Chromink.exe trace --help | Out-String
& .\Chromink.exe trace input.png --colors 8 --quantization vecline --tracing vecline --vc-preset logo --output output.svg --json | Out-String
& .\Chromink.exe schema | Out-String
```

完整接口见 [CLI.md](documentation/CLI.md)。

## 构建

需要 Rust 1.96+、Node.js 和 [Tauri 平台构建工具](https://v2.tauri.app/start/prerequisites/)。

```powershell
npm ci
npm run dev
npm run build
```

Windows GNU 可使用 scripts/build-chromink-windows-gnu.ps1 -LlvmMingwBin C:/tools/llvm-mingw/bin。运行桌面版使用系统 WebView2 Runtime，GNU 发布同时携带 WebView2Loader.dll；无头模式不需要 WebView2。GPU 按设备能力选择并自动回退 CPU，Lloyd 使用并行 CPU。当前提供 Windows x64 便携包；Linux/macOS 需在对应系统构建，尚未实机验证。

源代码只保留当前运行模块、构建配置、使用文档和许可声明。许可证见 [LICENSE](LICENSE)；算法来源见 [第三方说明](documentation/THIRD_PARTY.md)。
