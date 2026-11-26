# 硬件接口地图：直连 / 中转 / 不需要 / 权限或厂商限制

> 唯一目标机：RMX3357（Android 13 / kernel 4.19.191 / 4 KiB 页 / Mali-G77 MC9 /
> `untrusted_app`，无 root、无 Shizuku）。状态列只写这台机器的实测。
> 逐类的取舍理由在 [`device-interface-choices.md`](device-interface-choices.md)，
> 后端协议在 [`husk-backend-interface.md`](husk-backend-interface.md)。

## 1. 四类口径

每个接口必须落进且只落进其中一类，没有第五类，也不允许写「未做」：

| 分类 | 定义 |
|---|---|
| **直连** | guest 能安全打开的真实设备节点或标准内核接口，中间没有我们的代码 |
| **中转** | 必须经过 Android 服务 / NDK 的标准接口；guest 看到的是标准 Linux 接口，后端是宿主 |
| **不需要** | 对 Linux 用户态本来就无意义，或实现出来只能和 Android 抢同一份硬件 |
| **权限或厂商限制** | 想做但被宿主挡住；必须贴具体错误串，不许写「未做」 |

下面 1.1 是实测的接法，1.2 是能映射但还没做的，1.3 是明确不需要的，1.4 是被挡住的。

### 1.1 已接上的（直连 / 中转）

| 设备类 | guest 看到的接口 | 接法 | 实测 | 证据 |
|---|---|---|---|---|
| GPU | Vulkan / EGL（`/dev/mali0` + kbase，PanVK 后端） | **直连**：guest 进程自己开设备节点，应用侧一行不参与 | `Mali-G77 MC9` 枚举、compute、离屏回读、Zink、X11 窗口呈现；canonical ICD 与 loader 级 `env -i` 默认值已复跑；0.0.34 当前 GNOME 与 Nautilus 进程均为 zink 环境、持有两个 `/dev/mali0`、maps 已加载 PanVK；`make gpu-audit` 又通过嵌套 GTK 应用验证继承链。GTK4 仍默认 `GSK_RENDERER=cairo`，不能把 UI 绘制说成硬件加速；当前仍不是窗口内容到 X server 的零拷贝（DRI3 未通） | 156、165、167、241、242、243、244、245、278 |
| 相机 | V4L2 `/dev/videoN` facade | **中转**：guest 看到标准 V4L2 ioctl/read，后端经 socket 到 Camera2 NDK；`open()` 是 memfd，不是真节点 | 前后摄自动启动并取流；NV12/YUYV 的 `read`、`readv`、mmap 均通过；拍照 / 30 秒录像，硬件 H.264 编码。无 DMABUF / controls | 93、240、282 |
| 音频 | PulseAudio（`module-pipe-sink` / `module-pipe-source`） | **中转**：Rust `audio-relay` → AAudio | 48 kHz 双声道 610 秒；反证：guest 里 `/dev/snd` 是 `Permission denied`，根本没有声卡节点 | 62 |
| 视频解码 | VA-API（`vaGetDisplayDRM` / `vaCreateSurfaces`） | **中转**：Rust VA driver + `video-bridge` ↔ MediaCodec，AHB 交接 | H.264 1080p30 连续 600 秒，`dropped=0` | 80–87 |

### 1.2 有标准接口可以映射、还没做的

| 接口 | 分类 | 宿主侧接口 | 现在卡在哪 |
|---|---|---|---|
| 传感器 | **中转（可做，未做）** | NDK 纯 C：`ASensorManager_*` / `ASensorEventQueue_*` 由 `libandroid.so` 导出（本机 `libandroid.so` 导出表已核）；`pm list features` 有 accel / compass / gyro / light / proximity / stepcounter | 没有实现。`untrusted_app` 读不到 `/sys/bus/iio/devices`（`Permission denied`），但这不影响走 NDK。前人做法已读完：伪造一棵 IIO sysfs 树，值文件原地覆盖写 |
| 定位 | 中转（受限） | 无 NDK API；`LocationManager` 是平台类，JNI 可以取 `getLastKnownLocation()` | 持续定位需要 `LocationListener` 实现类，`hasCode=false` 造不出来。要么接受「只给 last-known」，要么单独设计一个极小的 Java 类 |
| USB | 中转 + 权限对话框 | `UsbManager`（Java）拿到 `UsbDeviceConnection` 的 fd 后，可以交给 guest 的 `libusb` | 首次插拔必须弹系统授权，需要 `Intent` + 广播接收器，同样卡在 `hasCode=false` |
| 蓝牙 | 中转 + 运行时权限 | 只有 Java `BluetoothAdapter`，NDK 没有 API | 监听器 / 接收器同上；Android 12+ 还要 `BLUETOOTH_CONNECT`、`BLUETOOTH_SCAN` |

### 1.3 不需要的

| 接口 | 理由 |
|---|---|
| NFC | Linux 用户态没有标准 POSIX NFC 接口。Android 的 NFC 只在 Java 侧由前台 Activity 派发 tag，guest 没有能承接一次刷卡交互的可信窗口。硬做只能和 Android 抢同一张卡。 |
| 指纹 | 认证语义绑定「可见 Activity 上的用户交互」，`BiometricPrompt` 要求真实窗口。guest 能拿到的只有「已认证」这个结论，拿不到可用的凭据流。 |

### 1.4 被宿主挡住的（贴原始结果）

| 节点 / 能力 | 实际结果 |
|---|---|
| `/dev/snd` | `Permission denied`（`untrusted_app`）—— 所以音频只能中转 |
| `/sys/bus/iio/devices` | `Permission denied` —— 所以传感器只能走 NDK，不能直读 sysfs |
| `/dev/gps*` | `No such file or directory` —— 没有字符设备，只能走定位服务 |
| `/dev/dri/renderD128` | 不存在（只有 `card0`，`226,0`，`system:system 0660`）—— DRI3 因此不通，呈现只能回读加拷贝 |

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
