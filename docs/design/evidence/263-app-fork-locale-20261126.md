# 263 · App 直接 PTY：locale 统一与 fork 后零分配

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。被测源码：`app-rs/src/term.rs`、新增
`runtime/crates/session-env/`、`tools/ci/check-fork-safety.sh`。

## 0 · 结论

FACT：`Term::spawn()` 是 App 直接启动 guest PTY 的路径，并不经过
`tools/guest/husk-session.sh`。旧代码在 `execve()` 前把 `LANG` 固定为
`C.UTF-8`，所以即使 rootfs 和 shell session 已经支持中文默认值，App 直接打开的
终端仍可能拿到英文环境。

FACT：旧代码在 `fork()` 后的子进程块里才构建 `argv`/`envp` 的 `Vec`。App 是
多线程进程；`fork()` 后子进程不能安全分配内存，否则可能卡在另一个线程持有的
分配器锁上，表现为 fork 成功但永远不进入 `execve()`。

FACT：这次修改只落在 Husk 的 App PTY 启动路径和共享 Rust locale 选择器，没有修改
LinuxHub 2.0、桌面应用或 `com.linuxhub.myth` 的数据。

## 1 · 本地静态与语义验证

```text
make check
PASS: runtime fmt/check/clippy
PASS: session-env 4 tests
PASS: app-rs aarch64-linux-android lib + husk-session-guard check
PASS: tools/ci/check-fork-safety.sh
PASS: tools/ci/test-locale-defaults.sh
PASS: tools/ci/check-shell.sh，79 个 shell 脚本
PASS: Markdown 链接检查，0 个坏链
version ok: 0.0.32
```

`check-fork-safety.sh` 的原始输出：

```text
fork safety: child exec path has no allocations
```

`session-env` 的四个单元测试分别覆盖：无调用者语言时选择中文、显式英文不被
`LANGUAGE` 覆盖、显式 `LANGUAGE` 原样保留、没有中文 locale archive 时回退
`C.UTF-8`。

## 2 · 真机 guest 回归

```text
DEV=UKHMMB8PYTGECMNV
PKG=com.linuxhub.myth.husk
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs
LD=files/linux/husk/libld.so
make gate-a
```

结果：

```text
PASS=38 FAIL=0
coverage-probe: PASS
coverage-sweep: SKIP (rootfs 没有 strace)
supplementary groups: PASS=1 FAIL=0
session locale: PASS default=zh_CN.UTF-8 explicit=en_US.UTF-8 explicit-lang-only=not-zh
path race: PASS host sentinel unchanged
fd-identity-probe: PASS
```

这说明本次共享 locale 选择器没有破坏现有 shell session、用户/组、路径、
V4L2、DRM、netlink 和设备回归；但由于当前设备安装的是
`com.linuxhub.myth.husk`，仓库自己的 APK 包名是 `com.linuxhub.husk`，两者不能
安全覆盖安装，因此 App 直接 PTY 的“真实启动后环境变量”和“多线程 fork 压力”
在本轮仍是 **UNVERIFIED**，不能把静态检查写成真机完成。

## 3 · APK 构建

使用实际 NDK 工具目录构建成功：

```text
make apk NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
BUILD SUCCESSFUL
app-debug.apk
libhuskapp.so
libhuskguard.so
libld.so
```

本轮只构建 APK，没有安装或清空任何现有 App 数据。构建产物中的
`libhuskapp.so` SHA-256 为：

```text
079568d6082cbf2f975879e86a05ce185b3069743d66c2218098827257da9527
```

`libld.so` 仍为：

```text
f21663f1d9699faf1e18fbc2d90bc5bd68cf5fbfb5e496d765437f47aeab841e
```

## 4 · 修复内容

- 新增 `runtime/crates/session-env`，把 `LANG`/`LANGUAGE` 选择规则做成可测试的
  Rust 函数；App PTY 和 shell session 使用同一套“默认中文、显式英文不覆盖”语义。
- `Term::spawn()` 在 `fork()` 前完成 `argv_s`、`env_s`、`argv_ptrs`、`env_ptrs`；
  子进程块只调用 `close/setsid/sigprocmask/open/ioctl/dup2/chdir/execve/_exit` 等
  路径，不再分配 Rust 容器。
- 新增 `tools/ci/check-fork-safety.sh`，扫描 child exec 区块中的 `Vec::`、`vec!`、
  `format!`、`CString::new`、`.collect`、`.to_string`、`.to_owned`，并接进
  `make check`。
- `Term::spawn()` 在进入 `fork()` 前探测 rootfs 是否存在中文 locale，再决定传给
  guest 的 `LANG`/`LANGUAGE`；显式英文环境保持英文。

## 5 · 边界

- FACT：验证覆盖的是共享 locale 逻辑、App 编译/链接、fork 子块静态门禁和现有
  guest 真机门禁 A。
- UNVERIFIED：新 APK 安装后直接 PTY 的动态 `LANG`/`LANGUAGE` 值，以及多线程 App
  下真实 fork+exec 死锁复现。
- UNKNOWN：其他 Android 版本、16 KiB/64 KiB 真机和不同 GPU 家族上的同一行为。
- FACT：本轮没有修改桌面合成器，也没有把 X11 换成 Wayland；X11 仍保持原路线。
