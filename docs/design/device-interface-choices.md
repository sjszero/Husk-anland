# 每类硬件用哪个"接口"（先找前人，再定方案）

> 目的：用户要求"动手前先看有没有人已经做出来了，看有没有更好解的接口"。
> 本文件是**接口选型结论**，逐类写清：标准 Linux 接口是什么、谁已经证明可行、Husk 现状、下一步。
> 参考来源、固定 commit 见 [`references.md`](references.md)、[`reference-audit-matrix.md`](reference-audit-matrix.md)。
> 每条"前人证据"都只写仓库里能读到的内容；没读到的不写。

## 0. 一句话结论

**最有价值的借鉴不是某个实现，而是 DroidNative 的接口选择**：它把手机硬件包装成
**Linux 程序本来就认识的标准接口**（`/dev/videoN` V4L2、IIO sysfs、`/dev/gps0` NMEA、
PulseAudio source），所以 Cheese、GStreamer、iio-sensor-proxy、gpsd 这类程序**一个都不用改**。
Husk 之前偏向"自定义 IPC + 自定义客户端"，那是次优解。下面的表按这个原则重排。

## 1. 逐类接口表

| 硬件 | 标准接口 | 前人证据（可读到的） | Husk 现状 | 下一步 |
|---|---|---|---|---|
| GPU | `open("/dev/mali0")` + kbase UAPI，Mesa PanVK 后端 | `FristOneRR-Admin/FristOneRR-Panvk-Source`、`faux123/panvk-...rp4pro`、`zenithblue-oss/panvk-kbase-android` | **可用**：PanVK logical device / compute / 离屏 graphics / Zink surfaceless（evidence 46–50） | 缺 WSI：把渲染面接到 Surface（第 3 条路径） |
| 显示 | dma-buf → `AHardwareBuffer` → `ANativeWindow` / `SurfaceControl`（consumer 持有 buffer） | `anland`（consumer-owned dmabuf + eventfd + shm 索引页，协议见 `common/protocol.h`）、`lfdevs/mesa-for-android-container` | **可用**：AHB→dma-buf→SurfaceControl，3-buffer ring（evidence 73–79） | 合成器侧需要一个 anland 式后端或 ScreenCast 导出；见下节 |
| 视频解码 | VA-API（`vaGetDisplayDRM`/`vaCreateSurfaces`）→ 驱动器转 MediaCodec | `lfdevs/termux-va`（同样用合成 CSD）、2.0 的 mesa tar | **可用**：`vaapih264dec` 出真实帧，150 帧交接（evidence 85–87） | EOS 后 guest 不退出的 bug；HEVC/VP9/AV1 |
| 视频编码 | VA-API encode → `c2.mtk.avc.encoder` | `lfdevs/termux-va` 有编码（README 声明 AVC/HEVC 编码） | **UNKNOWN／没路径**：只确认了编码器存在 | 先读 termux-va 的 encode 路径再设计 |
| 音频输出 | `module-pipe-sink`（guest PulseAudio 契约） | 2.0 `ContainerAudioBridge`、DroidNative `AudioOutput`、Oboe/AAudio | **可用**：610 秒长稳（evidence 63） | GStreamer/桌面播放器验收 |
| 音频输入 | `module-pipe-source` + 按需开关 | DroidNative `MicBridge.kt` + `session.sh mic_watch` | **可用**：声学回环 440 Hz（evidence 88） | 产品级权限、长稳、并发 |
| 相机 | **`/dev/videoN` V4L2**（QUERYCAP / ENUM_FMT / REQBUFS / QBUF / DQBUF / poll） | DroidNative `CameraBridge.kt` + `docs/device-passthrough.md` + `tests/host/v4l2/feed.c` | **没做** | 在 shim 里做 V4L2 语义层（他们做在 proot extension，我们做在 libc 层） |
| 传感器 | **IIO sysfs**（`/sys/bus/iio/devices/iio:deviceN` + `in_*_raw`） | DroidNative `SensorBridge.kt` / 文档 §Sensors | **没做** | 同左，纯文件语义，比 ioctl 简单 |
| 定位 | **`/dev/gps0` NMEA 0183**（`$GPGGA`/`$GPRMC`） | DroidNative `LocationBridge.kt` / 文档 §Location | **没做** | 同左 |
| 输入 | **合成器级注入**：X11 → `XTEST FakeInput`；Wayland/mutter → `RemoteDesktop`+EIS(libei) | `anland`（Clutter 虚拟设备）、2.0 的 Lorie（server 侧 inject）、DroidNative（App 内合成器） | 部分：app 侧手势已做，注入端未定 | 见 [`anland-deep-read.md`](anland-deep-read.md)；**不走 /dev/input+libinput+udev** |
| USB / 蓝牙 / NFC / 指纹 | Android 普通应用 API（无标准 Linux 节点） | —— | **没做** | 逐项探测 + 写成平台限制或 Rust 桥 |

## 2. 两条被实测否决/约束的路线

1. **厂商 `libGLES_mali.so` 直接进 glibc guest**：25 个 bionic `DT_NEEDED`，加载不了。
   FACT，已排除（见 `mali-kbase-prior-art.md`）。
2. **app 侧设备 socket 放在 rootfs 外**：guest 路径解析会把它搬进 rootfs，导致
   `No such file or directory`。FACT，evidence 88。**所有设备桥 socket 必须建在 rootfs 内**。

## 3. Husk 与 DroidNative 的形态差异（为什么不能照抄代码）

| | DroidNative | Husk |
|---|---|---|
| 路径/设备拦截层 | proot fork 的 C extension（ptrace） | loader 内嵌 shim（libc 层 interpose，零 ptrace） |
| app 语言 | Kotlin + 少量 C | 纯 Rust（只有内核 ABI 裸汇编例外） |
| 现状 | 相机/传感器/定位/麦克风都有 | 只有麦克风（evidence 88），相机/传感器/定位没有 |

⇒ **可以借协议与语义（ioctl 序列、sysfs 布局、NMEA 句子、`mic.active` 语义），不能借实现层。**

## 4. 相机为什么选 V4L2 而不是自定义桥

- FACT：DroidNative 的相机对 guest 是 `/dev/videoN`，支持 `QUERYCAP` / `ENUM_FMT` /
  `ENUM_FRAMESIZES` / `G/S/TRY_FMT` / `REQBUFS` / `QBUF` / `DQBUF` / `poll`，
  格式 `YUYV` 与 `NV12`，`V4L2_MEMORY_MMAP` 与 `USERPTR`。
- FACT：他们还补了 udev 发现所需的三处（`/sys/class/video4linux/videoN`、
  `/sys/subsystem/video4linux/devices/videoN`、`/run/udev/data/c81:N`），
  因为 Android 禁止列目录，udev/GStreamer 靠 glob 会看不到设备。
- JUDGMENT：这套语义值得照搬，否则我们又要写一个只有自己认识的客户端，
  和"用标准接口"的初衷相反。

## 5. 桌面（GNOME）路线：三种候选，先 spike 再定（2026-11-23）

anland 深读给出的事实：

- **FACT**：anland 的 Mutter 后端只支持 mutter 49/50/51；配方是 Debian 13 与 Ubuntu 26.04。
  我们 2.0 的 24.04 rootfs 是 **mutter 46**（guest 内 `libmutter-14.so.0` 已核实）。
- **FACT**：mutter 46 自带 `--headless`（`src/core/meta-context-main.c:468 create_headless_backend`）、
  `MetaRemoteDesktop` 的 EIS 输入注入（`src/backends/meta-eis.c`，`meta-remote-desktop-session.c:1933`）。
- **FACT**：LinuxHub 2.0 的 GNOME 是 **X11**：App 内 Lorie X server + GNOME Shell 46
  （`GnomeDesktopCompatibility.kt:4-9`），输入在 X server 侧注入。
- **FACT**：guest rootfs 里有 `Xwayland`、`mutter`、`xfce4`、`xterm`，没有 `Xvfb` / `Xorg`。

| 路线 | 显示 | 输入 | 代价 | 风险 |
|---|---|---|---|---|
| A. GNOME on X11（学 2.0） | guest 内 X server 的帧 → App AHB | XTEST | 需装 `xvfb`/Xorg-dummy（不在离线 deb 集里，需单独取包）；GL 走 llvmpipe 或 zink | Xvfb 上的 GL 质量 UNKNOWN |
| B. mutter `--headless` | ScreenCast 虚拟监视器 → PipeWire dmabuf | RemoteDesktop EIS | guest 需要 PipeWire 客户端（Rust 侧实现 spa 较陡） | EGL 需要 render node（我们有 PanVK surfaceless，FACT） |
| C. mutter + anland 后端 | anland 协议（最省事、最省带宽） | Clutter 虚拟设备 | 需 backport 到 46 或换 rootfs（违反"只用 2.0 的 24.04 链"） | 许可：anland GPL-3.0，只能协议层对接 |

**JUDGMENT**：先做 B 的 15 分钟 spike（`mutter --headless` 能不能在 guest 起来 + 有没有 ei fd），
因为它零补丁、零新增发行版依赖、GL 走硬件；A 作为并行备选（2.0 已证明 GNOME Shell 46 在 X11 下可用）。
C 只在 A/B 都不行时考虑，并且必须同时解决许可边界。
