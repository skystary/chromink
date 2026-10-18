# PDF 依赖许可

新增依赖由 Cargo.lock 锁定，可从 crates.io 按名称和版本取得源码。Hayro 以 Rust 库静态链接，不启动外部程序，不分发 PDF DLL。未加入上游示例、开发测试或 GPU PDF 渲染器。

0.4.0 关闭内嵌标准字体和 CMap，不再链接 hayro / Vello 整页渲染。只扫描实际绘制的图片对象并解码，颜色配置许可继续保留。下表与目录包含 0.3.0 的历史组件声明，最终依赖以当前 Cargo.lock 为准；旧字体、CMap 和 Vello 声明保留用于回溯，不表示新版仍携带这些资源。Hayro 组件选择 MIT 许可。

| 依赖 | 版本 | 声明许可 |
| --- | --- | --- |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause |
| alloc-stdlib | 0.2.4 | BSD-3-Clause |
| brotli | 8.0.4 | BSD-3-Clause AND MIT |
| brotli-decompressor | 5.0.3 | BSD-3-Clause/MIT |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| color | 0.3.3 | Apache-2.0 OR MIT |
| euclid | 0.22.14 | MIT OR Apache-2.0 |
| fearless_simd | 0.4.1 | Apache-2.0 OR MIT |
| font-types | 0.11.3 | MIT OR Apache-2.0 |
| glifo | 0.1.1 | Apache-2.0 OR MIT |
| guillotiere | 0.7.0 | MIT/Apache-2.0 |
| hayro | 0.7.1 | Apache-2.0 OR MIT |
| hayro-ccitt | 0.3.0 | Apache-2.0 OR MIT |
| hayro-cmap | 0.1.0 | Apache-2.0 OR MIT |
| hayro-interpret | 0.7.0 | Apache-2.0 OR MIT |
| hayro-jbig2 | 0.3.0 | Apache-2.0 OR MIT |
| hayro-jpeg2000 | 0.3.5 | Apache-2.0 OR MIT |
| hayro-postscript | 0.1.0 | Apache-2.0 OR MIT |
| hayro-syntax | 0.7.2 | Apache-2.0 OR MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| kurbo | 0.13.1 | Apache-2.0 OR MIT |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| peniko | 0.6.1 | Apache-2.0 OR MIT |
| pic-scale | 0.7.12 | BSD-3-Clause OR Apache-2.0 |
| polycool | 0.4.0 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| read-fonts | 0.39.2 | MIT OR Apache-2.0 |
| skrifa | 0.42.1 | MIT OR Apache-2.0 |
| vello_common | 0.0.8 | Apache-2.0 OR MIT |
| vello_common | 0.0.9 | Apache-2.0 OR MIT |
| vello_cpu | 0.0.8 | Apache-2.0 OR MIT |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |
