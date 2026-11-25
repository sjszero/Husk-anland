# 硬件接口地图：直连、中转、未做

> 唯一目标机：RMX3357（Android 13 / kernel 4.19.191 / 4 KiB 页 / Mali-G77 MC9 /
> `untrusted_app`，无 root、无 Shizuku）。状态列只写这台机器的实测。
> 逐类的取舍理由在 [`device-interface-choices.md`](device-interface-choices.md)，
> 后端协议在 [`husk-backend-interface.md`](husk-backend-interface.md)。

## 1. 只有一根轴：直连 / 中转

| 设备类 | guest 看到的接口 | 接法 | 实测 | 证据 |
|---|---|---|---|---|
| GPU | Vulkan / EGL（`/dev/mali0` + kbase，PanVK 后端） | **直连**：guest 进程自己开设备节点，应用侧一行不参与 | `Mali-G77 MC9` 枚举、compute、离屏回读、Zink、X11 窗口呈现；canonical ICD 与 loader 级 `env -i` 默认值已复跑；App 用 `gpu-env` crate 生成配置，loader 读取；0.0.19 默认关闭 DXVK 兼容 GS，fixed-function 600 帧通过，显式开启仍可复现 Bifrost 崩溃 | 156、165、167、241、242、243、244 |
| 相机 | V4L2 `/dev/videoN` facade | **中转**：guest 看到标准 V4L2 ioctl，后端经 socket 到 Camera2 NDK；`open()` 是 memfd，不是真节点 | 前后摄自动启动并取流；拍照 / 30 秒录像，硬件 H.264 编码。无 DMABUF / read / controls | 93、240 |
| 音频 | PulseAudio（`module-pipe-sink` / `module-pipe-source`） | **中转**：Rust `audio-relay` → AAudio | 48 kHz 双声道 610 秒；反证：guest 里 `/dev/snd` 是 `Permission denied`，根本没有声卡节点 | 62 |
| 视频解码 | VA-API（`vaGetDisplayDRM` / `vaCreateSurfaces`） | **中转**：Rust VA driver + `video-bridge` ↔ MediaCodec，AHB 交接 | H.264 1080p30 连续 600 秒，`dropped=0` | 80–87 |

> **跨 SoC 代码状态**：GPU 按节点分族。Mali/kbase 的本机实测见上表；Adreno/KGSL 的
> render-node → 真实 KGSL fd 适配器代码已落地（evidence 233），但本仓库没有 Adreno 真机，
> 所以 Adreno 的打开、Vulkan 枚举、ioctl/mmap、Turnip 和桌面全部是 **UNKNOWN**，不能算通过。

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

## 2. 为什么「直连」只对 GPU 成立

`untrusted_app` 能开的节点是有限的，这一条决定了接法，不是设计偏好。2026-11-25 在应用域
（`u:r:runas_app`）里实测的布局：

| 节点 | 结果 |
|---|---|
| `/dev/mali0` | 存在，`crw-rw-rw- root root 10,33`，能打开——**这就是 GPU 直连的物理基础** |
| `/dev/video0` / `/dev/video1` | guest 里由 V4L2 shim 报告为 `81,0` / `81,1`；实际帧来自 Camera2，不是把 App 的 V4L2 fd 交给 guest |
| `/dev/dri/card0` | 存在，`system:system 0660`，应用打不开；`renderD128` **不存在** |
| `/dev/dma_heap` | 不存在 |
| `/dev/snd` | 打不开（`Permission denied`）——所以音频只能中转 |
| `/dev/kgsl-3d0` | 不存在（那是高通的节点） |

**结论**：能直开的 GPU 使用真实节点和 fd；相机虽然给 guest 标准 V4L2 接口，但 `open()` 返回
shim 的 memfd，帧经 socket 由 Camera2 产生，所以归入中转；声音、编解码同样走明确的 socket
协议，数据在应用侧过一次；
既开不了又没有平台 API 的（传感器、GPS、蓝牙、NFC、USB）这一版没有。

## 3. 默认值也属于接口的一部分

驱动只部署一份、配置只写一次，是这套接法的前提。默认值不放在 `/etc/profile.d`
（那只对登录 shell 生效），而是由**加载器**在程序自己没设置时补：

- 数据来自 `/etc/husk/gpu-env`（安装器写，ICD 文件名在安装时解析）；第一行 `@GPU_FAMILY=`
  记录生成时的设备族，rootfs 搬到另一族设备时加载器不会注入错误家族的 ICD/驱动；
- 程序自己 `export` 的永远优先；
- 逃逸开关 `HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF=1`；
- 实测（evidence 182）：`env -i` 起 `gl-probe es3` → `zink Vulkan 1.3(Mali-G77 MC9)`；
  0.0.17 又在 App 管理的 rootfs 上复验了 canonical ICD + `/etc/husk/gpu-env` 的自动生成
  （evidence 242），并确认 `HUSK_GPU=llvmpipe` 的进程级回退仍优先；0.0.18 将生成逻辑抽成
  `gpu-env` crate，并补上 Adreno 启动分支和回退传递（evidence 243）；0.0.19 让默认值穿透
  真实 `setenv(..., 0)` 和显式 `execve(..., envp)`（evidence 244）。

## 4. 能说什么、不能说什么

可以说：

- 硬件按 **Linux 标准接口**整块接进来，不是给某个应用单独开洞；
- 应用侧一行代码不改，驱动只部署一份，配置只写一次；
- 直连的 GPU **绕过中间层**，应用身份那一侧不参与；相机是标准 V4L2 facade，后端明确走 Camera2。

不能说：

- 「全部直连」——相机、音频、视频都是明确的中转；相机更不是真实 V4L2 内核节点；
- 「零拷贝」不加限定——显示那一段是零拷贝，**窗口内容 → X server 那一段不是**
  （代价与原因见 [`../LIMITATIONS.md`](../LIMITATIONS.md) §7）。
- 「PanVK 编译器已修」——0.0.19 只是默认关闭会触发内部 GS 的 DXVK 兼容路径；显式开启仍会复现
  Bifrost 崩溃，依赖 GS 的 DXVK 程序需要自己 opt-in（evidence 244）。
