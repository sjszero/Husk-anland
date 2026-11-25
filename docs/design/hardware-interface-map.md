# 硬件接口地图：直连、中转、未做

> 唯一目标机：RMX3357（Android 13 / kernel 4.19.191 / 4 KiB 页 / Mali-G77 MC9 /
> `untrusted_app`，无 root、无 Shizuku）。状态列只写这台机器的实测。
> 逐类的取舍理由在 [`device-interface-choices.md`](device-interface-choices.md)，
> 后端协议在 [`husk-backend-interface.md`](husk-backend-interface.md)。

## 1. 只有一根轴：直连 / 中转

| 设备类 | guest 看到的接口 | 接法 | 实测 | 证据 |
|---|---|---|---|---|
| GPU | Vulkan / EGL（`/dev/mali0` + kbase，PanVK 后端） | **直连**：guest 进程自己开设备节点，应用侧一行不参与 | `Mali-G77 MC9` 枚举、compute、离屏回读、Zink、X11 窗口呈现 | 156、165、167 |
| 相机 | V4L2 `/dev/videoN` | **直连**：真节点 + 标准 ioctl | 取流 / 拍照 / 30 秒录像，硬件 H.264 编码 | 93 |
| 音频 | PulseAudio（`module-pipe-sink` / `module-pipe-source`） | **中转**：Rust `audio-relay` → AAudio | 48 kHz 双声道 610 秒；反证：guest 里 `/dev/snd` 是 `Permission denied`，根本没有声卡节点 | 62 |
| 视频解码 | VA-API（`vaGetDisplayDRM` / `vaCreateSurfaces`） | **中转**：Rust VA driver + `video-bridge` ↔ MediaCodec，AHB 交接 | H.264 1080p30 连续 600 秒，`dropped=0` | 80–87 |

两类不在这一根轴上，单列免得被算错：

| 项 | 说明 |
|---|---|
| 显示 | `dma-buf → AHardwareBuffer → SurfaceControl`，**这一段是零拷贝**（3-buffer ring + 真实 `SYNC_FD`，submitted=10 000 / completed=10 000，evidence 59） |
| 输入 | 走窗口层注入（mutter → `RemoteDesktop`/EIS），不是设备参数；`/dev/input` + libinput + udev 那条路需要设备节点和 udev，本机不通（evidence 148、180） |

未做 / 不做的：

| 项 | 状态 | 为什么 |
|---|---|---|
| 视频编码 | 未做 | 编码器存在已确认，VA-API encode 后端没写 |
| 传感器 | 未做 | 要合成 `/sys/bus/iio/devices/iio:deviceN/*` 树（`untrusted_app` 读真 sysfs 被 SELinux 拒） |
| 定位 | 未做 | 要 `/dev/gps0` 输出 NMEA 0183，gpsd 一族只认这个 |
| USB | 未做 | 要 `/dev/bus/usb/00X/00Y` + `USBDEVFS_*` ioctl 语义（libusb 的 Linux 后端） |
| 蓝牙 | 未做 | 本机没有 HCI 节点 / VHCI，内核路径不通 |
| NFC | 未做 | 本机无 `/dev/nfc*`，Linux NFC 栈不适用 |
| 指纹 | 不做 | 认证必须绑定可见 Activity 的用户交互，没有设备节点也没有 NDK API |

## 2. 为什么「直连」只对两类成立

`untrusted_app` 能开的节点是有限的，这一条决定了接法，不是设计偏好。2026-11-25 在应用域
（`u:r:runas_app`）里实测的布局：

| 节点 | 结果 |
|---|---|
| `/dev/mali0` | 存在，`crw-rw-rw- root root 10,33`，能打开——**这就是 GPU 直连的物理基础** |
| `/dev/video0` / `/dev/video1` | 存在，`81,0` / `81,1`——相机直连 |
| `/dev/dri/card0` | 存在，`system:system 0660`，应用打不开；`renderD128` **不存在** |
| `/dev/dma_heap` | 不存在 |
| `/dev/snd` | 打不开（`Permission denied`）——所以音频只能中转 |
| `/dev/kgsl-3d0` | 不存在（那是高通的节点） |

**结论**：能直开的（GPU、相机）直开，节点就是标准 Linux 节点，程序看到的是真 fd；
开不了的（声音、编解码）走一条**明确的 socket 协议**，数据在应用侧过一次；
既开不了又没有平台 API 的（传感器、GPS、蓝牙、NFC、USB）这一版没有。

## 3. 默认值也属于接口的一部分

驱动只部署一份、配置只写一次，是这套接法的前提。默认值不放在 `/etc/profile.d`
（那只对登录 shell 生效），而是由**加载器**在程序自己没设置时补：

- 数据来自 `/etc/husk/gpu-env`（安装器写，ICD 文件名在安装时解析）；
- 程序自己 `export` 的永远优先；
- 逃逸开关 `HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF=1`；
- 实测（evidence 182）：`env -i` 起 `gl-probe es3` → `zink Vulkan 1.3(Mali-G77 MC9)`。

## 4. 能说什么、不能说什么

可以说：

- 硬件按 **Linux 标准接口**整块接进来，不是给某个应用单独开洞；
- 应用侧一行代码不改，驱动只部署一份，配置只写一次；
- 直连的两类（GPU、相机）**绕过中间层**，应用身份那一侧不参与。

不能说：

- 「全部直连」——音频、视频是明确的中转，中间多一次应用侧协议；
- 「零拷贝」不加限定——显示那一段是零拷贝，**窗口内容 → X server 那一段不是**
  （代价与原因见 [`../LIMITATIONS.md`](../LIMITATIONS.md) §7）。
