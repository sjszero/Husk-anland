# 原型：纯 Rust、零 ptrace 的路径运行时（2026-11-22）

## 结果

Ubuntu 24.04 glibc `bash` + `cat` 在 Android 应用目录内原生执行，`/etc/os-release`
正确解析到 rootfs，全程没有 ptrace、没有 chroot、没有 namespace。

```
NAME="Ubuntu"            # 有 shim
VERSION="24.04.4 LTS (Noble Numbat)"
ID=ubuntu

cat: /etc/os-release: No such file or directory   # 去掉 shim 即失败（对照）
```

100 次 `cat` 进程 exec（每次含 loader + 路径重写）≈ 0.25 s，约 2.5 ms/次；
100 次 bash 空循环 < 10 ms。原始输出见 `evidence/04-pure-rust-path-shim-20261122.txt`。

## 实现

`runtime/crates/path-shim`：

- `#![no_std]` + `cdylib`，交叉编译 `aarch64-unknown-linux-gnu`，rust-lld 链接，
  **没有任何 `DT_NEEDED`**（不依赖 guest 的 libc 版本）。
- 通过 `LD_PRELOAD` 注入 glibc 进程，拦截 `open/open64/openat/openat64/stat/lstat/
  fstatat/access/readlinkat/execve`。
- 绝对路径 → `NR_ROOTFS + path`；`/dev` `/proc` `/sys` 直通；相对路径保持原样。
- `execve` 对动态 ELF 做"解释器补全"：`ld.so --library-path <root>/usr/lib/aarch64-linux-gnu <bin> ...`，
  这是绕过 `PT_INTERP` 绝对路径限制的关键，且依然不需要 ptrace。
- 错误码按 glibc ABI 写回 `__errno_location`。

## 为什么这比 PRoot 快

PRoot：每个带路径 syscall 两次 ptrace 停（微秒级），所有进程都在 tracer 下。
本方案：一次用户态路径拼接（纳秒级）+ 普通 syscall；exec 时多一次 loader 启动。
没有 tracer 进程，没有逐 syscall 上下文切换。

## 已知边界（必须继续解决，不能宣称完成）

1. 只覆盖 libc 符号；静态 ELF、Go、直接 `svc` 的程序绕过 → 需要 ptrace/seccomp 兜底后端。
2. 相对路径与 cwd 未虚拟化；`..` 逃逸未处理（`/../../etc/passwd` 会落到 Android 根）。
3. 脚本 `#!` 解释器、`/proc/self/exe`、Unix socket 路径、`dlopen` 绝对路径尚未处理。
4. `argv[0]` 变成 rootfs 真实路径，部分程序自检会暴露。
5. shim 必须放在可执行映射的位置（`nativeLibraryDir`）；当前 demo 用 runas_app 验证了机制，
   还需在 `untrusted_app` + APK `nativeLibraryDir` 下复验。
6. 未做并发压力测试与多进程共享 rootfs 的一致性验证。

## 下一步

1. 把 shim 与 loader 打进 APK `nativeLibraryDir`，在真实 `untrusted_app` 域复验。
2. 补 `cwd` 虚拟化与 `..` 归一化（防止逃逸，这是安全边界）。
3. 加 `execve` 对脚本/静态 ELF 的分支：脚本交给 guest `/bin/sh`，静态 ELF 走 ptrace 兜底。
4. 引入 `runtime-core` 的统一路径解析器（Rust，供 shim、loader、bench 共用）。
5. 基准：同一 rootfs 下 PRoot vs 本方案（stat、readdir、小文件、进程启动、编译）。
