# 硬件接口地图

Husk 的目标不是给某个 Linux 应用单独开洞，而是把 Android 宿主能力映射成稳定的 Linux 接口。
每个接口只归入四类之一：

| 分类 | 定义 |
|---|---|
| **直连** | guest 进程直接打开真实设备节点，应用侧不参与数据路径 |
| **中转** | guest 看到标准 Linux 接口，后端经明确协议转到 Android 服务或 NDK |
| **不需要** | 对 Linux 用户态没有可复用的标准语义，或实现只会与 Android 抢夺同一交互 |
| **权限或厂商限制** | 接口方向成立，但当前应用权限或回调模型不够 |

参考验证环境是 arm64 Android 13、kernel 4.19.191、4 KiB 页、Mali-G77 MC9、
`untrusted_app`。机型只是证据条件，不是运行时产品绑定。

## 1. 当前矩阵

| 能力 | 分类 | guest 接口 | Android 后端 | 当前状态 |
|---|---|---|---|---|
| GPU | 直连 | `/dev/mali0` + Vulkan/EGL | PanVK + Zink（Mali/kbase） | Mali-G77 已真机验证；Adreno/KGSL 仅有适配代码，真机 UNKNOWN |
| 相机 | 中转 | V4L2 `/dev/videoN` facade | Camera2 NDK / `AImageReader` | 2026-11-27 复验：A-hw 11/0，前后摄各 3 帧 1280×720 NV12；fixture 的 V4L2/GStreamer 两档通过；DMABUF/controls 未支持 |
| 音频输出 | 中转 | PulseAudio | Rust relay → AAudio | 真实 PCM 输出已验证 |
| 音频采集 | 中转 | PulseAudio source | Rust relay → AAudio | 196,608 PCM bytes、peak=285 已验证 |
| 视频解码 | 中转 | VA-API | Rust VA driver → MediaCodec | H.264 1080p30 600 秒，dropped=0 |
| 传感器 | 中转 | IIO sysfs 影子树 | NDK `ASensorManager_*` | 加速度、磁力、陀螺仪、环境光、距离五类可读；D-Bus 消费端未验 |
| 显示 | 应用侧中转 | AHB / dma-buf | `SurfaceControl` | 3-buffer ring 与 `SYNC_FD` 10,000 帧；仅代表应用侧接管段 |
| 输入 | 窗口层 | 宿主 window/X11 事件 | Android 输入事件 | bridge 已实现；端到端依赖宿主窗口系统 |
| 定位 | 权限或厂商限制 | Location 接口（候选） | Java `LocationManager` | 持续定位需要 Java 回调类；可接受 last-known 子集或加最小垫片 |
| USB | 权限或厂商限制 | libusb（候选） | `UsbManager` + Intent | 首次授权需要广播接收器；需要最小 Java 垫片 |
| 蓝牙 | 权限或厂商限制 | BlueZ（候选） | `BluetoothAdapter` | Android 12+ 权限与回调模型需要 Java 垫片 |
| NFC | 不需要 | — | 前台 Activity tag dispatch | 没有可复用的标准 POSIX NFC 设备接口 |
| 指纹 | 不需要 | — | `BiometricPrompt` | 认证绑定可见 Activity，guest 没有可复用的凭据流 |

**直连只有 GPU。** 相机、音频、视频、传感器和显示都是中转。不能宣传成“全部直连”。

## 2. GPU 直连

参考机的真实节点：

```text
/dev/mali0  crw-rw-rw- root root 10,33
```

guest 进程直接打开该节点，PanVK 走 kbase 后端，Zink 提供 OpenGL/GLES。默认环境由 loader 从
`/etc/husk/gpu-env` 注入：

```text
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
```

实测：

```text
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1 -> llvmpipe
HUSK_GPU=llvmpipe -> llvmpipe
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

边界：

- `PANVK_DXVK_COMPAT=1` 的 geometry shader 崩溃是已知支持边界，默认规避不是修复。
- GLES 3.2 依赖 `MESA_GLES_VERSION_OVERRIDE`，真实应用覆盖不足。
- Adreno/KGSL 没有参考真机，状态 UNKNOWN。
- 窗口内容进入宿主 X server 仍不是零拷贝。

## 3. 相机、音频、视频和传感器

这些能力需要 Android 应用身份，因此不能把宿主 fd 直接冒充成内核设备：

- **相机**：guest `open("/dev/videoN")` 返回 shim 的 memfd；V4L2 ioctl 和 buffer 协议由 Rust
  shim 承接，帧由 Camera2 NDK 产生。`read`、`readv`、mmap 已验证。
- **音频**：`/dev/snd` 在应用域不可打开；guest 使用 PulseAudio，Rust relay 转成 AAudio。
- **视频**：guest 使用 VA-API；Rust driver 和 bridge 把帧转到 MediaCodec / AHardwareBuffer。
- **传感器**：NDK `ASensorManager_*` 产生事件，写入应用私有 IIO 影子树；guest 通过标准
  `/sys/bus/iio/devices/iio:deviceN/*` 读取。

这些接口的好处是应用侧无需改代码；代价是协议、同步和格式能力受 Android 后端限制。测试数字
分别见 evidence 309、310、302、80。

## 4. 显示与输入

显示链需要分成两段：

```text
guest 渲染 / X client -> 宿主 X server    # 可能发生回读与 SHM 拷贝
X 合成结果 -> AHB/dma-buf -> SurfaceControl # 已实现的应用侧零拷贝段
```

当前发行资产不包含 X server。GNOME/X11 桌面启动属于宿主 App 集成，不能把其他 App 或旧
PRoot 树的 socket 算进 Husk。证据见 evidence 312。

输入走窗口层：Android MotionEvent/KeyEvent 进入 App，再注入宿主窗口系统。它不是一个可直接
直开的 `/dev/input` 设备路径。

## 5. 定位、USB、蓝牙、NFC、指纹

- **定位、USB、蓝牙**方向上是中转，但 Android API 需要 Java listener、Intent 或广播接收器。
  当前 App 保持 `hasCode=false`，因此要么增加一个极小的 Java 类，要么明确只支持功能子集。
- **NFC、指纹**归类为不需要。它们依赖前台 Activity 的可见交互或安全认证语义，Linux 用户态
  没有可复用的标准设备接口。

## 6. 可以说什么

可以说：

- 能直开的设备节点使用真实 fd；
- 必须经过 Android 服务的能力，向 guest 暴露标准 Linux 接口；
- 驱动和 GPU 默认配置只部署一份，所有 guest 进程共享；
- 应用侧一行代码不改。

不能说：

- “所有硬件都是直连”；
- “整条显示链零拷贝”；
- “Adreno 已支持”；
- “GNOME 桌面由 Husk 独立启动”；
- “Husk 是恶意代码安全沙箱”。
