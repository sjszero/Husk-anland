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
| 显示 | dma-buf → `AHardwareBuffer` → `ANativeWindow` / `SurfaceControl` | 2.0 的 `anland`、`lfdevs/mesa-for-android-container` | **可用**：AHB→dma-buf→SurfaceControl，3-buffer ring（evidence 73–79） | 与 GPU 渲染面合成 |
| 视频解码 | VA-API（`vaGetDisplayDRM`/`vaCreateSurfaces`）→ 驱动器转 MediaCodec | `lfdevs/termux-va`（同样用合成 CSD）、2.0 的 mesa tar | **可用**：`vaapih264dec` 出真实帧，150 帧交接（evidence 85–87） | EOS 后 guest 不退出的 bug；HEVC/VP9/AV1 |
| 视频编码 | VA-API encode → `c2.mtk.avc.encoder` | `lfdevs/termux-va` 有编码（README 声明 AVC/HEVC 编码） | **UNKNOWN／没路径**：只确认了编码器存在 | 先读 termux-va 的 encode 路径再设计 |
| 音频输出 | `module-pipe-sink`（guest PulseAudio 契约） | 2.0 `ContainerAudioBridge`、DroidNative `AudioOutput`、Oboe/AAudio | **可用**：610 秒长稳（evidence 63） | GStreamer/桌面播放器验收 |
| 音频输入 | `module-pipe-source` + 按需开关 | DroidNative `MicBridge.kt` + `session.sh mic_watch` | **可用**：声学回环 440 Hz（evidence 88） | 产品级权限、长稳、并发 |
| 相机 | **`/dev/videoN` V4L2**（QUERYCAP / ENUM_FMT / REQBUFS / QBUF / DQBUF / poll） | DroidNative `CameraBridge.kt` + `docs/device-passthrough.md` + `tests/host/v4l2/feed.c` | **没做** | 在 shim 里做 V4L2 语义层（他们做在 proot extension，我们做在 libc 层） |
| 传感器 | **IIO sysfs**（`/sys/bus/iio/devices/iio:deviceN` + `in_*_raw`） | DroidNative `SensorBridge.kt` / 文档 §Sensors | **没做** | 同左，纯文件语义，比 ioctl 简单 |
| 定位 | **`/dev/gps0` NMEA 0183**（`$GPGGA`/`$GPRMC`） | DroidNative `LocationBridge.kt` / 文档 §Location | **没做** | 同左 |
| 输入 | app `InputQueue` → guest 合成器（Wayland/xkb） | `anland`、`termux-x11`（成熟基线） | 部分（`app-rs/src/term.rs`） | 补齐鼠标/键盘/触摸到 guest |
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
