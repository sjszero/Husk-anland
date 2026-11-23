# 每类硬件的 guest 接口形态

> 原则：**guest 里看到的东西必须是 Linux 程序本来就认识的标准接口**。
> 这样 Cheese、GStreamer、gpsd、iio-sensor-proxy 这类程序一个都不用改。
> 自定义 IPC + 自定义客户端是次优解，只在没有标准接口时才用。
>
> 状态列只反映本机（RMX3357 / Android 13 / untrusted_app）实测。

## 1. 逐类形态

| 硬件 | guest 侧接口 | 为什么是这个 | 状态 |
|---|---|---|---|
| GPU | `open("/dev/mali0")` + kbase UAPI，PanVK 后端 | 本机唯一可直开的 GPU 节点，Mesa 有 kbase 后端 | 可用（compute / 离屏 / Zink surfaceless） |
| 显示 | dma-buf → `AHardwareBuffer` → `SurfaceControl` | 渲染面必须由 consumer 持有，才能零拷贝上屏 | 可用（3-buffer ring + 真实 SYNC_FD，10,000 帧） |
| 视频解码 | VA-API（`vaGetDisplayDRM` / `vaCreateSurfaces`），后端接 MediaCodec | 播放器与 GStreamer 只认 VA-API | 可用（H.264 1080p30 600 秒） |
| 视频编码 | VA-API encode，后端接 MediaCodec 编码器 | 同上 | 未做（编码器存在已确认） |
| 音频输出 | `module-pipe-sink`（guest PulseAudio 契约） | 桌面与播放器统一走 PulseAudio | 可用（610 秒） |
| 音频输入 | `module-pipe-source` + 按需开关 | 同上；只有真录音时才开 Android 麦克风 | 可用（声学回环） |
| 相机 | `/dev/videoN` V4L2（QUERYCAP / ENUM_FMT / G/S_FMT / REQBUFS / QBUF / DQBUF / poll） | GStreamer / Cheese / OpenCV 全走 V4L2 | 可用（取流 / 拍照 / 录像） |
| 输入 | 合成器级注入：mutter → `RemoteDesktop`(+EIS) | 不走 `/dev/input` + libinput + udev：那需要设备节点与 udev | 可用 |
| 传感器 | `/sys/bus/iio/devices/iio:deviceN/*` 合成树 | `iio-sensor-proxy` 与直接读 sysfs 的程序都认；guest 读真 sysfs 会被 SELinux 拒 | 未做 |
| 定位 | `/dev/gps0` 输出 NMEA 0183 | gpsd 一族只认串口 NMEA | 未做 |
| USB | `/dev/bus/usb/00X/00Y` + `USBDEVFS_*` ioctl 语义 | libusb 的 Linux 后端就长这样 | 未做 |
| 蓝牙 | org.bluez D-Bus 接口 | 本机没有 HCI 节点 / VHCI，内核路径不通；`bluetoothctl` 是 D-Bus 客户端 | 未做 |
| NFC | 自有守护进程 + D-Bus（读卡器姿态） | 本机无 `/dev/nfc*`，Linux NFC 栈不适用 | 未做 |
| 指纹 | 无（平台限制） | 认证必须绑定可见 Activity 的用户交互，没有可用的设备节点或 NDK API | 不做 |

## 2. 被实测否决的路线

1. **厂商 GPU 用户态库直接进 glibc guest**：25 个 bionic `DT_NEEDED`，加载不了（FACT）。
2. **app 侧设备 socket 放在 rootfs 外**：guest 路径解析会把 socket 路径搬进 rootfs，导致
   `No such file or directory`（FACT，evidence 88）。**所有设备桥 socket 必须建在 rootfs 内。**
3. **namespace / mount / chroot / FUSE**：本机 `CONFIG_USER_NS` 未开，`mount` 与 `chroot`
   在 app 域直接被杀进程（见 `capability-baseline.md`）。

## 3. 实现层原则

| 项 | Husk |
|---|---|
| 路径/设备拦截 | loader 内嵌 shim（libc 层 interpose，零 ptrace） |
| app 语言 | 纯 Rust；只有内核 ABI 强制的裸汇编 stub 例外 |
| 平台 API 的例外 | USB / 蓝牙 / NFC / 定位在 NDK 里没有平台 API，需要最小 JNI 垫片（只取句柄，不做传输） |

## 4. 相机为什么必须是 V4L2

- guest 侧要能直接用 `v4l2src` / Cheese / OpenCV，这些只认 `/dev/videoN`。
- 需要的 ioctl 集合：`QUERYCAP`、`ENUM_FMT`、`ENUM_FRAMESIZES`、`G/S/TRY_FMT`、
  `REQBUFS`、`QBUF`、`DQBUF`、`poll`，`MMAP` 与 `USERPTR` 两种内存模式。
- udev/GStreamer 会 glob 设备节点，Android 不允许列目录，因此还要合成
  `/sys/class/video4linux/videoN` 与 `/run/udev/data/c81:N`。

## 5. 桌面路线

目标形态是 **2.0 的 GNOME 桌面**，不是自研合成器，因此：

- 不在 APK 里塞第二个合成器（等于把 gnome-shell 换掉，验收项全变）。
- 不给 mutter 写编译期后端（等于把上游组件的版本绑到自己身上）。
- 走 mutter 自带的 `org.gnome.Mutter.ScreenCast` 协议，只做消费端：
  PipeWire 节点 → dma-buf → AHardwareBuffer → SurfaceControl。
