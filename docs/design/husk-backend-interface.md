# Husk Backend Interface（HBI）可行性与边界

> 状态：设计定案，未实现。  
> 结论：**可行度高，但不能做成“一个 ioctl 适配所有 SoC”**。正确形态是：
> 上层继续使用成熟的 VA-API / Vulkan / EGL / Wayland / X11 / PulseAudio；
> Husk 内部只定义一套统一的 Rust 后端接口，芯片差异收敛到很薄的 adapter。

## 1. 三层结构

```text
应用 / GNOME / Chromium / GStreamer / Totem / mpv
        │
        │ 标准接口，不改应用
        ▼
VA-API   Vulkan/EGL   Wayland/X11   PulseAudio
        │
        ▼
Husk Backend Interface（HBI，Rust trait + 稳定对象模型）
        │
        ├── Mali kbase adapter
        ├── Adreno KGSL adapter
        ├── DRM/V4L2 adapter
        └── MediaCodec adapter（bionic 侧）
```

**FACT：** 现有代码已经分别有 MediaCodec、PanVK、SurfaceControl、AAudio 的实测路径；
问题是这些路径各自带着自己的 buffer、fd、fence 和生命周期。

**JUDGMENT：** 把它们收进 HBI 能显著减少重复逻辑，且不会要求应用重写。

## 2. 一个接口应该统一什么

HBI 不按“GPU 接口 / 视频接口 / 音频接口”分成三套互不相关的 API，而是统一资源模型：

| 对象 | 语义 |
|---|---|
| `Device` | SoC/后端能力探测、设备节点、驱动版本 |
| `Buffer` | dma-buf/AHardwareBuffer 的 fd、尺寸、stride、format、modifier、usage |
| `Fence` | acquire/release 同步；必须能跨 CPU、GPU、显示和视频使用 |
| `GraphicsQueue` | Vulkan/EGL/DRM 提交与完成 |
| `VideoSession` | codec、profile、分辨率、压缩包输入、解码帧输出 |
| `DisplayTarget` | Android Surface、Wayland surface 或 X11 drawable |
| `AudioStream` | PCM 格式、时钟、输出设备 |

统一的关键不是“函数名一样”，而是**同一块 Buffer 和同一个 Fence 能在视频解码、GPU 转换、显示和音频时钟之间移动**。这正好对应 Husk 已有的 AHB + dma-buf + sync_file 实测基础。

## 3. 为什么放在系统层

**JUDGMENT：** 正确位置是 Husk 的系统运行层，而不是每个应用、每个播放器各写一套。

- guest 内把 HBI 装成系统库：VA-API driver、Vulkan ICD、EGL/GL 入口、显示后端；
- app 进程内只保留一个 bionic 侧 `MediaCodec`/`AAudio`/Surface bridge；
- Chromium、GStreamer、Totem、Mesa 看到的是标准系统接口，不需要知道 MTK、Mali 或 Adreno；
- 新芯片只替换 HBI 后面的 adapter，不改 Chromium、GStreamer、GNOME 或业务代码。

**FACT：** 这不等于“没有适配”。设备节点、ioctl、buffer modifier、fence、MediaCodec 名字和显示目标仍然由 adapter 处理；
只是适配从“每个应用各做一次”降为“系统层每个芯片族做一次”。

## 4. 可行性判断

| 能力 | 可行度 | 原因 |
|---|---:|---|
| GPU 统一入口 | 高 | Vulkan/EGL 已是跨厂商标准；Mali/Adreno 只需 backend adapter |
| 视频统一入口 | 中高 | VA-API 是标准，但 codec、surface 导出和 fence 语义因后端而异 |
| 显示统一入口 | 高 | Android 走 AHardwareBuffer/SurfaceControl；Linux 桌面走 Wayland/X11 |
| 音频统一入口 | 高 | guest 侧 PulseAudio/ALSA，app 侧 AAudio；时钟模型可独立 |
| 应用零修改 | 高 | 让 GStreamer/Chromium 用 VA-API，让 Mesa 用 Vulkan/EGL |
| SoC 零适配 | 低 | 设备节点、ioctl、格式、同步和固件永远有差异 |
| 代码复用 | 高 | 以现有 evidence 73–80 的 buffer/fence 路径为基础，估计可复用大部分控制面 |

**结论（JUDGMENT）：** 统一接口的架构可行度约 80–90%；“任何芯片拿过来完全不动代码”可行度接近 0。
应把目标改成：**应用不改，Husk 的上层不改；新增芯片只写 adapter 和探测表。**

## 5. 视频为什么仍需要一个薄层

**FACT：** 本机 guest 内没有可直连的 `/dev/video*` 硬解节点；硬解入口在 bionic 侧
`AMediaCodec`，而 Chromium/GStreamer 在 glibc guest 内。

所以不能把 MediaCodec 直接当成 Linux 的 `/dev/video`。HBI 的视频层必须提供：

```text
VA-API 语义（H.264/HEVC profile、surface、render、sync、export）
        ↓
HBI VideoSession
        ↓
app 内 AMediaCodec（c2.mtk.*）
        ↓
AHB / dma-buf
        ↓
同一个 HBI Buffer 回到 guest / GPU / 显示
```

这不是重写播放器，也不是重写解码器；它是把 Android 的硬解能力包装成 Linux 媒体栈认识的接口。

## 6. 不应采用的方案

- 不定义一套只给 Husk 应用使用的新播放器 API；这样 GStreamer、Chromium、Totem 都要改。
- 不假设所有 SoC 都支持同一组 kbase/DRM ioctl。
- 不把视频帧永远走 CPU 拷贝；HBI 必须把 dma-buf/fence 作为一等对象。
- 不为每个机型写一套完整路径；只允许机型/probe 表决定选哪个 adapter。
- 不把“统一接口”误解为“统一驱动”；底层仍然是 Mali kbase、Adreno KGSL、DRM 等不同实现。

## 7. 落地顺序

1. 先定义 HBI 的 `Buffer`、`Fence`、`DisplayTarget`、`VideoSession` 四个对象。
2. 把现有 SurfaceControl/PanVK buffer 路径迁移到 `Buffer`/`Fence`。
3. 实现 `husk` VA-API driver，让 `vainfo` 枚举 H.264/HEVC profile。
4. 让 GStreamer `vah264dec/vah265dec` 通过 HBI 调用 `c2.mtk.*`。
5. 让 Chromium 使用同一 VA-API 后端；用真实 H.264/HEVC 文件验证。
6. 最后再接 GNOME/Totem、显示 WSI 和 AAudio 时钟，补 AV 同步。

## 8. 当前能力事实（2026-11-23）

- 2.0 资产里有 `google-chrome-stable_..._arm64.deb`、`totem_..._arm64.deb`，不需要重造浏览器或播放器。
- 当前已解开的 `rootfs-full` 有 GStreamer 运行库，但缺少 `gst-launch-1.0`/`gst-inspect-1.0`、VA-API 插件和 Chrome 可执行文件。
- 已准备官方 Ubuntu 24.04 的 `gstreamer1.0-tools`、`gstreamer1.0-vaapi`、`vainfo` 包，下一步是装入 Husk 私有 rootfs 并验证加载。
- 还没有 `husk` VA-API driver；这是视频统一入口当前最关键的缺口（UNKNOWN 未验证）。
