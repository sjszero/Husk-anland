# 变更记录

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
