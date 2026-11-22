# 变更记录

## 2026-11-22 夜 — 壳应用改为纯 Rust

- **去掉 Java 与 C**：APK 改为声明 `android.app.NativeActivity`，入口是 `app-rs/` 里的
  `ANativeActivity_onCreate`。Java Activity、JNI 外壳、`pty.c` 全部删除。
- **壳应用现在包含**：PTY 与 `fork`/`execve`（直接跑 guest loader）、EGL + GLES2 渲染、
  运行时字体光栅化（`fontdue`，字模取自系统等宽字体）、ANSI 子集解析、光标块、
  生命周期（surface 销毁/重建、窗口尺寸→`TIOCSWINSZ`、失焦不画光标、会话随进程存活、
  子进程退出后下一次点击自动重启）。
- **单元测试**：`app-rs` 的终端解析与屏幕缓冲有 9 个主机侧测试（`cargo test`）。
- **构建工具**：Python 参考实现删除（逐字节对拍已在 evidence 22/23 记录，可从 v0.1.0 取回）；
  loader 重构后重新构建，sha256 仍为 `1600a850…1cbee`。
- **基准**：新增 `runtime/crates/guest-bench`（Rust 版客舱基准）；`bench.c` 保留为无预编译
  二进制时的兜底。
- 语言构成：Rust 5,681 行 / 汇编 271 行；Java 0、Python 0。


## 2026-11-22 — v0.1 原型（一天内完成）

**结论：在无 root、无虚拟化、无 ptrace 的 `untrusted_app` 域里跑通了 bash + coreutils +
完整 gcc 13.3 / make 工具链。**

### 运行时
- 路径 shim 从 `LD_PRELOAD` 下沉进 loader：成为 `ld.so` 的一个额外 `R|E` `PT_LOAD`，
  由 `_dl_init` 钩子完成符号介入（76 个名字的 GOT 槽）。运行时**零 syscall 拦截**。
- `set_robust_list` / `rseq` 两处 `svc` 构建期 `nop`：app seccomp 政策会把它们判成
  `SECCOMP_RET_TRAP`，且 glibc 在 loader 内部就调用它们。
- 起始 cwd 改为 `/`；`..` 在 rootfs 边界截断；符号链接逃逸测试 0 escapes。
- `execve` 认出动态解释器后改走同一个 loader，子进程天然带 shim。

### 构建工具（全部 Rust，零依赖）
- `nr-tools scan-syscalls`：按 `mov x8,#N; svc #0` 模式定位调用点，读 `.gnu.version_d`
  自检 glibc 版本，四种异常一律拒绝写文件。
- `nr-tools embed`：把 shim 与服务表嵌进 `ld.so`，所有重定位在构建期烘好。
- `nr-tools provision`：替代 sh+python 的工具链供应脚本。
- 三个工具都与原 Python / sh 实现**逐字节对拍**通过（evidence 22、23）。

### 验证
- 31/31 功能矩阵；11 项越界测试 0 escapes；gcc+make 全链 exit 0。
- 成本：`getpid` 196 ns、`stat` 单组件 5.4 us（+2.5 us）、`fork+execve+wait` 2.49 ms
  （同机原生 bionic 6.8–8.0 ms）。

### 已知缺口
- `execl/execle/execlp` 拦不到；`freopen/glob/nftw/dlmopen` 未覆盖；
  `posix_spawn` 用 `fork` 而非 `vfork`；16K 页内核未测。
