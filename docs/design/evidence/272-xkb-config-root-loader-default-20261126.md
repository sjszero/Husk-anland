# 272 · XKB_CONFIG_ROOT 由加载器补齐

日期：2026-11-26（CST）。

## 0 · 现象与根因

FACT：GNOME 46 在缺少 `XKB_CONFIG_ROOT` 时，`libxkbregistry.so + 0x48ec` 在
`rxkb_context_parse()` 前拿到 NULL context；LLDB 记录 `x0=0`，随后在地址 `0x28` 崩溃。
调用方是 `libgnome-desktop`，它没有检查 `rxkb_context_new()` 的 NULL 返回值。

FACT：这个变量设置在 `/etc/profile.d` 或桌面启动脚本里仍然可能被绕过。登录 shell、桌面 fork、
`env -i` 和嵌套 `execve` 会走到不同的环境边界，不能保证每个 GUI 进程都继承到。

## 1 · 修法

`runtime/crates/path-shim/src/lib.rs` 在 loader 的 `getenv` / `secure_getenv` 路径中补默认值：

```text
<guest rootfs>/usr/share/X11/xkb
```

优先级明确：

- 调用方显式设置的 `XKB_CONFIG_ROOT` 原样返回；
- `AT_SECURE` 下 `secure_getenv` 仍按 glibc 语义返回 NULL；
- 没有显式值时，返回由 guest rootfs 拼出的静态路径，不依赖登录 shell 或桌面脚本。

值与 GPU 默认值分开处理：`HUSK_GPU=llvmpipe` 只关闭 GPU 注入，不能关闭键盘数据路径。

## 2 · 真机证据

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB。

用 guest 内编译的探针直接调用 `getenv` 与 `secure_getenv`，默认输出：

```text
XKB_SECURE=/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/usr/share/X11/xkb
XKB_GETENV=/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/usr/share/X11/xkb
```

显式覆盖后输出不变值：

```text
XKB_SECURE=/tmp/xkb-user
XKB_GETENV=/tmp/xkb-user
```

FACT：同一版 loader 的完整门禁 A 为 `PASS=38 FAIL=0`；coverage-probe、dpkg conffile、
V4L2、DRM、fd identity、补充组、locale、身份并发、组表并发、session guard、
rootfd 与路径竞态全部通过。

FACT：候选 loader 的 SHA-256 为
`a2ecbb1ea74aebd98e77ca0bc6af9d37f7accd603ecbbebb85b378641596813d`。

## 3 · 边界

- `UNKNOWN`：其它发行版、其它 glibc 版本是否也在同一符号位置触发；本次只验证 Ubuntu 24.04 /
  glibc 2.39。
- `UNKNOWN`：显式空字符串 `XKB_CONFIG_ROOT=` 的第三方库行为；loader 按“显式值”处理，不覆盖。
- 本次不把 XKB 修复写成 Wayland 或合成器结论；会话仍保持 X11。
