# 同一 EXE 无头命令的许可

0.6.8 桌面 EXE 使用已锁定的 clap / clap_builder / clap_lex 和 glob；clap_derive 为构建宏，终端格式化依赖来自 anstream / anstyle 系列、colorchoice、is_terminal_polyfill、utf8parse、strsim。它们在进程内编译，不调用外部软件。版本以 Cargo.lock 为准，版权和许可证文件按原包内容保留在本目录。原始包来源为 crates.io。
