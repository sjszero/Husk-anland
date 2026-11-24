# Compatibility

What actually runs, and what does not, on the same Ubuntu 24.04 (glibc 2.39) rootfs
used for the benchmarks. "可用" means a command was executed inside the guest on
the target device and produced the expected result, not that a package is present.

Target device: RMX3357 (MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 /
4 KiB / arm64), `untrusted_app`, no root.

## Shell, toolchain, language runtimes

| 项 | Husk | proot | proot-distro | 证据 |
|---|---|---|---|---|
| `bash` / coreutils | 可用 | 可用 | 可用 | bench `have_*` |
| `git`（clone + status） | 可用，928 ms | 可用 | 可用 | bench `git_clone` |
| `python3` + `json` | 可用，86 ms | 可用 | 可用 | bench `python_json` |
| `python3 -c 'import numpy'` | 可用 1.26.4，370 ms | 可用 | 可用 | bench `python_numpy`、evidence 155 |
| `gcc` 编译 hello | 可用，157 ms | 可用 | 可用 | bench `gcc_hello` |
| `make -j4`（200 目标，链接产物 91 KB） | 可用，5 033 ms | 可用 | 可用 | bench `make_j4`、evidence 155 |
| `vim` / `nano` / `node` | 可用 | 可用 | 可用 | bench `have_*` |
| `apt-get install` / `dpkg -l` / `dpkg --configure -a` | 可用 | 可用 | 可用 | evidence 154 |
| `ldconfig`（`husk-ldconfig`） | 可用，950 条 / 6 目录 | 用 guest 自带静态 ldconfig | 同 proot | evidence 154 |

## Desktop

| 项 | Husk | proot（本仓库的 runner） | 证据 |
|---|---|---|---|
| GNOME Shell 46 X11 会话 | 可用（顶栏 / dock / 壁纸 / 通知） | **不可用**：2.0 已删除 proot 运行时（evidence 144），proot runner 没有显示服务集成；rootfs 内也没有 X server（`xvfb` 不在离线包索引里） | evidence 147 |
| 窗口 / 输入（触摸、XI2） | 可用，`TouchInputHandler` → X server → mutter | 同上 | evidence 148 |
| 会话总线（`dbus-send --session`） | 可用 | 未测 | evidence 152 |
| 无障碍总线 at-spi | 可用 | 未测 | evidence 151 |
| gvfs（`gio mount -l`） | 可用，不回退到会话总线 | 未测 | evidence 152 |
| polkit 授权（`pkcheck` power-off） | 可用，rc=0 | 未测 | evidence 151 |
| 文件管理器 `nautilus` 46.4 | 已安装，桌面可用 | — | `rootfs-packages.md` |
| 终端 `gnome-terminal` 3.52 / `xterm` | 已安装，桌面可用 | — | 同上 |
| 设置 `gnome-control-center` | 已安装，桌面可用 | — | 同上 |
| GPU 硬件加速 | **不可用**（`swrast`/`llvmpipe` 软件渲染），见 `LIMITATIONS.md` §1 | 不可用 | `STATUS.md` 第 4 条 |

## 浏览器

不可用，原因明确：rootfs 来自 2.0 的离线包索引，其中没有浏览器包
（`epiphany-browser` / `firefox` 均 `Unable to locate package`，`xterm`、
`gnome-terminal`、`nautilus` 等则在索引内）。要装浏览器需要给 rootfs 换一份
完整镜像源，不在本版范围。

## 不在支持范围

Termux 不是交付形态（`LIMITATIONS.md` §4）。传感器 / GPS / USB / 蓝牙 / NFC /
指纹、硬件视频编解码、WSI/swapchain 均未开始。
