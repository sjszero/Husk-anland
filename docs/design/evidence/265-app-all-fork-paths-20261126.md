# 265 · App 全部 fork 子路径的后 fork 零分配审计

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。对象：`app-rs` 的会话 guard、guest PTY leader、SIGSYS
启动自检和可选 trace 子进程。

## 0 · 结论

FACT：H-53 是代码构造确定的 fork 安全缺口。上一轮 evidence 263 只把主 PTY 的
`if pid == 0` 区块纳入静态门禁；同一 `term.rs` 里的 `spawn_guard()` 以及
`lib.rs` 的 SIGSYS 自检、`trace.rs` 的诊断子进程仍在 `fork()` 后分配内存：

| 路径 | 旧代码 fork 后的操作 |
|---|---|
| `spawn_guard()` | 用 `.collect()` 新建 `argv`/`envp` 两个 `Vec` |
| `probe_sigsys_semantics()` | 在子进程调用 `format!()` 和 `log()` |
| `trace::spy()` | 用 `.collect()` 新建 `argv`/`envp` 两个 `Vec` |

FACT：App 是多线程进程。`fork()` 后子进程只应调用 async-signal-safe 操作；在这些
位置分配 Rust 容器，可能卡在 fork 瞬间由另一线程持有的 allocator 锁上。

## 1 · 修复

- `spawn_guard()` 在 `fork()` 前构建 `guard_argv` / `guard_env`，子进程只做
  `setsid`、`close`、`dup2`、`execve`、`_exit`。
- SIGSYS 自检子进程只安装处置并发出两次原始 `syscall`，不再 `format!()` 或写日志；
  结论由父进程根据退出码记录。
- `trace::spy()` 在 `fork()` 前构建 `argv_ptrs` / `env_ptrs`，子进程只设置 PTY、
  `ptrace` 和 `execve`。

`tools/ci/check-fork-safety.sh` 从只扫描一个 PTY 区块扩展为扫描四条子路径：

```text
session guard
guest PTY leader
SIGSYS semantics probe
trace child
```

门禁同时拒绝 `Vec::`、`vec!`、`format!`、`CString::new`、`.collect(`、
`.to_string(`、`.to_owned(`、`Box::`、`String::`、`println!`、`eprintln!`。

## 2 · 本地验证

```text
cargo check --manifest-path app-rs/Cargo.toml --target aarch64-linux-android \
  --lib --bin husk-session-guard
PASS

rustfmt --edition 2021 --check app-rs/src/term.rs app-rs/src/trace.rs
PASS

sh tools/ci/check-fork-safety.sh
fork safety: all child exec/exit paths have no allocations

make check
PASS
fork safety: all child exec/exit paths have no allocations
shell syntax: 81 script(s) checked
checked 289 markdown files, 0 broken link(s)
version ok: 0.0.32
```

## 3 · APK 构建

```text
make apk NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
BUILD SUCCESSFUL
```

产物 SHA-256：

```text
libhuskapp.so   ce74f50ea07b9375de3cb0feb3abe7e7fd68cb173366c61b2fedb24031245e24
libhuskguard.so 4b33119e1b5bb33860a83c1a7854b2c2cf51b4ee5cddc27b81391f8fd9262897
libld.so       47562cb9f69001eef783ee0d4106ff92023e5ca3a56c9722474541cca3472516
app-debug.apk  5f92910a378c99dcc912715b3e538a9492a93b5208ad43e59b56d044ddfc936f
```

## 4 · 边界

- FACT：这是 App 启动/诊断路径的静态 fork 安全修复，不修改 LinuxHub 2.0 数据或桌面。
- UNVERIFIED：没有在仓库自己的 `com.linuxhub.husk` App 上做多线程 allocator 饱和下的
  真实 fork 死锁复现；当前设备安装的是 `com.linuxhub.myth.husk`，不能安全覆盖安装。
- INFERENCE：旧代码在 allocator 锁正好被其他线程持有时可能卡死；动态探针没有把这个
  时序稳定复现，因此不写成“已动态复现”。
- UNKNOWN：其他 Android 版本和不同 libc allocator 实现上的同一概率。
