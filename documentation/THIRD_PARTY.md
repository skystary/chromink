# 源码来源与许可

Chromink 的算法实现沿用 Color Trace 项目的 GPL-2.0-or-later 来源；原项目许可证和来源记录随仓库保留。本 Rust 版本包含 GPL-3.0-or-later 的 libimagequant，以 GPL-3.0-or-later 发布；保留原项目 LICENSE-UPSTREAM.txt，新版本许可证为 LICENSE。

| 组件 | 来源及用途 | 许可 |
|---|---|---|
| Potrace 轮廓 | Peter Selinger 的 Potrace 1.16；参考提交 886392d351417d6753aff61de08e294d1506e502，理解 C 算法后重写到 outline.rs，无 C ABI 或外部进程 | GPL-2.0-or-later |
| libimagequant | ImageOptim/libimagequant 4.5.0，9388d26902c53854c4f666dc3f07db6504b16924；纯 Rust，保留当前量化、映射和拟色模块，关闭 C ABI | GPL-3.0-or-later |
| Quantette | IanManske/quantette 0.6.0，0140c6abcc2262766bd2833fadedde1ac3358473；Oklab Wu 初始化及 SIMD 色彩转换。完整 Lloyd 在应用 lloyd.rs 实现；退役的小批量 K-means 及随机数依赖删除 | MIT / Apache-2.0 |
| Vecline | shunyagatha/Vecline，dcb2908a3c50e5276e5fd7e9d143f6fdd0d54bcd；理解 TypeScript 减色、区域轮廓、拟合、预处理及预设后用 Rust 重写。像素矩形、精确轮廓和自动几何对接应用减色结果；没有 Node、MCP、Office、其他导出或位图嵌入 | MIT，见 algorithms/VECLINE-LICENSE.txt |
| 图像 | image 0.24、TIFF/GIF/PNG/JPEG/WebP 等按需编解码模块，统一 RGBA 与应用缩放器 | MIT / Apache-2.0 等，版本见 Cargo.lock |
| PDF | Hayro-interpret 0.7.0、Hayro-syntax 0.7.2；vendor 增加图片解码接口，关闭字体/CMap 嵌入，仅提取实际绘制图片 | MIT / Apache-2.0，见 third-party/pdf |
| 预览 | tiny-skia 0.11.4、tiny-skia-path、应用原生场景渲染器；不链接 resvg/usvg 字体栈 | BSD-3-Clause 等，见 third-party/preview |
| 并行与色彩 | Rayon、palette 0.7.6，共享 CPU 工作池及色彩转换 | MIT / Apache-2.0 |
| GPU | wgpu 0.20、pollster，应用 WGSL、Rust SIMD 和少量 inline asm；系统 DX12/Vulkan/Metal，能力不足自动回退 CPU | MIT / Apache-2.0；应用代码 GPL-3.0-or-later，见 third-party/gpu |
| 桌面与对话框 | Tauri 2、rfd、dirs，系统 WebView/文件对话框/目录查询 | MIT / Apache-2.0，见 third-party/dirs |
| 命令行 | clap、glob，同一 EXE 内的无头命令 | MIT / Apache-2.0，见 third-party/cli |

本地源码库的原始版权和许可证保持完整，修改记录见各 vendor 的 PROVENANCE.md。应用不调用 Python、ImageMagick、独立 Potrace、pngquant 或其他外部图像处理软件。Windows 桌面运行需要系统 WebView2 Runtime；WebView2Loader.dll 随 GNU 便携包提供，无头模式不启动 WebView。

依赖版本锁定在 Cargo.lock / package-lock.json；逐项传递依赖许可保留在 third-party，属于版权声明，不是运行模块或测试依赖。
