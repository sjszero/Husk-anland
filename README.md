<!-- 首屏四段：标题 → 图标（下一格）→ 一句话 → 徽章。
     GitHub 只在空行处结束 HTML 块，标题与图标必须是两个独立块，否则标题会退回左对齐。 -->
<h1 align="center">Husk</h1>

<p align="center">
  <img src="docs/assets/brand/husk-mark-hexagon-h-512.png" width="128" alt="Husk">
</p>

<p align="center">让完整的 Linux 发行版以普通应用进程的身份，直接被 Android 内核执行。</p>

<!-- 徽章沿用 LinuxHub 仓库的 house style（flat-square），只写项目自身、可核对的事实：
     平台与架构、代码构成、依赖的 guest 树、运行方式。设备型号与系统版本只出现在下方
     「当前状态」表里——那是验收机的信息，不是项目属性。 -->
<p align="center">
  <img alt="License: Apache-2.0" src="https://img.shields.io/badge/License-Apache--2.0-2563EB?style=flat-square">
  <img alt="Rust: 96% of code" src="https://img.shields.io/badge/Rust-96%25%20of%2064.6k%20lines-DEA584?style=flat-square&logo=rust&logoColor=white">
  <img alt="Platform: Android arm64" src="https://img.shields.io/badge/Platform-Android%20arm64-3DDC84?style=flat-square&logo=android&logoColor=white">
  <img alt="Guest: Ubuntu 24.04 glibc" src="https://img.shields.io/badge/Guest-Ubuntu%2024.04%20glibc-E95420?style=flat-square&logo=ubuntu&logoColor=white">
</p>

<p align="center">
  <img alt="Runtime: in-process, no ptrace" src="https://img.shields.io/badge/Runtime-in--process%2C%200%20ptrace-0F766E?style=flat-square">
  <img alt="Root: not required" src="https://img.shields.io/badge/Root-not%20required-0F766E?style=flat-square">
  <img alt="Virtualization: none" src="https://img.shields.io/badge/Virtualization-none-0F766E?style=flat-square">
  <img alt="Syscalls: native" src="https://img.shields.io/badge/Syscalls-native%2C%20unintercepted-0F766E?style=flat-square">
</p>

**Husk 是一个 Android 运行时：让完整的 Linux 发行版（glibc 用户态）以普通应用进程的身份，
直接被 Android 内核执行——不需要 root、不需要解 Bootloader、不需要硬件虚拟化、不做 ptrace。**

它不拦截系统调用。路径虚拟化在 loader 里一次完成（改写 GOT），之后程序自己的 `svc` 直接进内核，
因此性能与原生 Android 应用同量级。

---

## 当前状态（唯一目标机：RMX3357 / Android 13）

| 项 | 事实 |
|---|---|
| 设备 | RMX3357，MT6893（天玑 1200），Mali-G77，Android 13（SDK 33），kernel 4.19.191，4 KiB 页，arm64 |
| 运行域 | `untrusted_app`，无 root、无 Shizuku |
| 运行时 | loader 内嵌 path-shim（零 ptrace）；exec / PTY / 信号 / 生命周期已闭环 |
| 桌面（2.0 应用内，X11） | LinuxHub 2.0 的 Ubuntu 24.04 rootfs 起 GNOME Shell 46，**X11 会话**（mutter 46 X11 后端）；顶栏、壁纸、dock、通知全部在屏（evidence 147） |
| 桌面（Husk 原生壳，Wayland） | mutter 46 在 guest 内跑成 Wayland 合成器，GPU 走 PanVK；`ScreenCast` / `RemoteDesktop` 接口都在（evidence 96–99） |
| GPU（guest 直开） | guest 进程直开 `/dev/mali0`，PanVK（Mali-G77）logical device / compute / 离屏 graphics / Zink surfaceless 全通（evidence 97） |
| GPU（2.0 桌面） | **仍是软件渲染**：`MESA_LOADER_DRIVER_OVERRIDE=swrast`/`llvmpipe` 还钉着，`DRI3` 未通（`docs/STATUS.md` 第 4 条） |
| 显示（2.0 应用内） | 应用内 X server（Lorie）把桌面渲染上屏 |
| 显示（原生直通） | `AHardwareBuffer` → dma-buf → `SurfaceControl` 零拷贝上屏；3-buffer ring + 真实 `SYNC_FD`；10,000 帧 / 665 秒长稳，fd/RSS 无增长 |
| 视频 | H.264 1080p30 经 `c2.mtk.avc.decoder` + AHB，连续 600 秒，dropped=0 |
| 音频 | guest 枚举输出设备、控音量，PCM 经独立通道进 app AAudio；610 秒连续会话 |
| 相机 | guest 内 `/dev/videoN` V4L2，GStreamer 可取流、可拍照、可录像 |
| 输入（2.0 应用内） | 触摸 → `TouchInputHandler` → JNI → 同一个进程里的 X server → XI2 → mutter（evidence 148） |
| 输入（原生直通） | 经 mutter `RemoteDesktop` 注入到 guest 合成器（evidence 98） |

**进行中**：把 GNOME 的帧接到零拷贝直通路径（ScreenCast → PipeWire → AHB → SurfaceControl）。
2.0 里的桌面现在走应用内 X server，还没有并到这条链上。

**未开始**：WSI / swapchain、HEVC / VP9 / AV1 解码、硬件编码、相机零拷贝、传感器、GPS、
USB / 蓝牙 / NFC / 指纹，以及把 LinuxHub 2.0 的 proot 运行时整体替换成 Husk。

> 只写实测过的结论；未上机的一律标 UNKNOWN，不写 PASS。逐项证据在
> [`docs/design/evidence/`](docs/design/evidence/)，当前进度在 [`docs/STATUS.md`](docs/STATUS.md)。

---

## 它和容器 / 虚拟机 / PRoot 的区别

| | 传统容器 | 虚拟机 | PRoot 一类 | **Husk** |
|---|---|---|---|---|
| 内核 | 宿主内核（要 namespace/cgroup） | 自带内核 | 宿主内核 + ptrace | **宿主内核，零拦截** |
| 权限 | root / user namespace | 虚拟化 | 普通应用 + ptrace | **普通应用** |
| 每个 syscall 成本 | 原生 | 陷入虚拟机 | 一次 ptrace 往返 | **原生（0 额外）** |
| 路径隔离 | mount namespace | 完整隔离 | ptrace 逐条改写 | **loader 首次解析后改 GOT，一次到位** |

---

## 架构

```
 Android app 进程 (untrusted_app)
 ├─ 壳：NativeActivity（Rust）——窗口 / Surface / 生命周期
 ├─ 硬件后端（Rust）——GPU、显示、视频、音频、相机、输入、传感器……
 │    └─ 平台 API / HAL（NDK、AHardwareBuffer、AAudio、MediaCodec、Camera2）
 └─ guest：自研 loader（内嵌 path-shim）+ Ubuntu 24.04 用户态
      ├─ 路径虚拟化：装载时重写 GOT，运行期零拦截
      ├─ exec：自己解析 PT_INTERP 与 shebang，递归回到 guest 根
      └─ 设备：能直开的节点（/dev/mali0）直开；被沙箱拒绝的走 app 内硬件后端
```

- 设计取舍与接口形态：[`docs/design/device-interface-choices.md`](docs/design/device-interface-choices.md)
- 运行时分层与预算：[`docs/design/runtime.md`](docs/design/runtime.md)
- 硬件后端接口边界：[`docs/design/husk-backend-interface.md`](docs/design/husk-backend-interface.md)

---

## 仓库结构

| 路径 | 内容 |
|---|---|
| `runtime/` | 纯 Rust 运行时：loader 侧 shim、exec、PTY、信号、路径解析 |
| `app-rs/` | 纯 Rust 壳应用（NativeActivity），窗口 / Surface / 输入 |
| `tools/` | 构建工具（`nr-tools`）、设备测试脚本 |
| `docs/` | 设计、证据、验收记录 |
| `docs/design/evidence/` | 每一步的设备原始输出，结论都能指回这里 |
| [`QUICKSTART.md`](QUICKSTART.md) | 从零跑起来 |
| [`CHANGELOG.md`](CHANGELOG.md) | 版本与改动记录 |

---

## 许可

Apache License 2.0，见 [LICENSE](LICENSE)；第三方组件、glibc 构件与商标声明见
[NOTICE](NOTICE)。
