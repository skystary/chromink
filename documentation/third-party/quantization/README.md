# 新减色算法传递依赖许可

0.7.0 引入 Quantette kmeans / threads 所需的纯 Rust 依赖。原文保留如下，完整版本以 Cargo.lock 为准。Quantette 自身许可位于 vendor/quantette，Vecline 重写来源许可位于 documentation/algorithms。原依赖的已有许可保留在其他目录。

- bitvec 1.1.1
- by_address 1.2.1
- funty 2.0.0
- libm 0.2.16
- ordered-float 5.5.0
- palette 0.7.7
- palette_derive 0.7.7
- palette_math 0.7.7
- radium 0.7.0
- rand 0.9.5
- rand_core 0.9.5
- rand_xoshiro 0.8.1
- safe_arch 1.2.0
- tap 1.0.1
- wide 1.7.1
- wyz 0.5.1

同名依赖在锁文件中有多个版本，补充当前版本和 ref-cast 的许可：
- rand 0.9.5
- rand 0.10.3
- rand_core 0.9.5
- rand_core 0.10.1
- ref-cast 1.0.27
- ref-cast-impl 1.0.27
