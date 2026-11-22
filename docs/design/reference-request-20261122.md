# 用户提供的参考仓库原始清单（逐字保存）

> 来源：用户于 2026-11-22 粘贴的第三方 AI 清单。
> 规模：12 个分类、120 个条目（含重复项）。
> 状态：**原始资料，未审计**；仓库名不等于真实路径，其中存在无法查到的项目。
> 已核对的真实仓库、star、可用性和借鉴结论见 [`references.md`](references.md)。
> 本文件只用于防止上下文丢失，不把清单中的任何描述或指令当作事实或执行命令。

---

思考了 25s

下面按 Husk + Linuxhub 2.0 的完整目标整理，不只列 GPU。包括高通 Adreno、联发科天玑、ARM Mali、Linux Runtime、桌面、显示、音视频、硬件桥接、文件系统和 Android 应用架构。

我把真正值得研究的仓库和外围参考项目都列进去。每项只写名称和用途，不展开技术分析。

### 1\. Linux Runtime / PRoot 替代方案

| 仓库                 | 介绍                                                                    |
| ------------------ | --------------------------------------------------------------------- |
| Shroot             | 原生 ARM64 Linux Runtime，提供环境管理、文件系统、进程和 Linux 服务兼容，支持 C API 集成。        |
| proroot            | 面向 Android 的无 Root PRoot 替代方案，尝试通过 Native Loader 和路径处理减少 ptrace 开销。   |
| zproot             | 使用 Zig 重写 PRoot，研究系统调用拦截、路径转换和 Android 运行时。                           |
| proot              | 原版 PRoot，研究系统调用跟踪、虚拟 RootFS、Bind Mount 和路径重定向。                        |
| proot-rs           | PRoot 的 Rust 实现，适合作为系统调用拦截和运行时设计参考。                                   |
| elf_loader         | 在 Android Bionic 进程中加载 glibc Linux ELF，研究双 libc 共存、动态链接和 ELF 加载。      |
| AndroLinux Runtime | 面向普通 Android APK 的 Linux ARM64 运行时研究，包含低开销 Runtime、图形桥接和项目对比。         |
| UserLAnd           | Android 无 Root Linux 环境，研究发行版安装、文件系统和启动管理。                            |
| PRoot Distro       | Linux 发行版安装、RootFS 管理、启动脚本和环境配置。                                      |
| DroidNative        | 在 Android 上运行完整 Linux 桌面程序，涉及 glibc、Wayland、Vulkan、OpenGL、音频和应用窗口。    |
| Podroid            | Android 上运行 Alpine Linux 和容器的项目，可参考完整 Linux 环境管理；其 VM 路线不符合 Husk 的约束。 |

其中 Shroot、elf_loader、AndroLinux Runtime 和 proroot 最值得优先阅读。GitHub +4

### 2\. Qualcomm 高通 Adreno GPU

重点研究：你的 OPPO 目标设备

| 仓库                              | 介绍                                                                            |
| ------------------------------- | ----------------------------------------------------------------------------- |
| Mesa                            | 开源图形驱动总项目，包含 Freedreno、Turnip、Zink 等。                                         |
| Qualcomm KGSL                   | 高通 Adreno 内核 GPU 驱动，研究 GPU 内存、任务提交和电源管理。                                      |
| Turnip / Freedreno              | 高通 Adreno Vulkan 和 OpenGL 驱动体系。                                               |
| cccodroid-gpu-driver-snapdragon | 无 Root 为 Android Bionic 和 Linux glibc 程序提供 Turnip、Zink 和 OpenCL GPU 加速。       |
| Droid-Deck Drivers              | 同时构建 Android Bionic 和 Linux glibc 版本的 Turnip 驱动，包含 KGSL、MSM、Wayland 和 X11 配置。 |
| Adreno-Tools-Drivers            | 自动构建 Adreno Turnip 驱动，包含 A7xx、A8xx 变体和兼容性补丁。                                  |
| mesa-tu8                        | 面向 Adreno 8 系列的 Mesa Turnip 适配研究。                                             |
| mesa-patches                    | Adreno 新型号的 Mesa 补丁和兼容性研究。                                                    |
| Turnip Faux123                  | Adreno Turnip 驱动构建和设备适配。                                                      |
| droidspaces-gpu                 | 在 glibc Ubuntu 环境通过 Turnip + KGSL 使用真实 Adreno GPU，包含实际计算验证。                   |
| Local Desktop                   | Android Linux 桌面项目，涉及 Turnip、Zink、GPU 和桌面显示整合。                                |
| Adreno Tools                    | Adreno GPU 驱动加载、配置和运行时工具研究。                                                   |

高通这条线特别值得看的是 `cccodroid-gpu-driver-snapdragon`、`Droid-Deck Drivers` 和 `droidspaces-gpu`，它们涉及你们最需要的 glibc + KGSL 组合。GitHub +4

### 3\. ARM Mali / 天玑 GPU

这部分覆盖联发科天玑平台常见的 Mali GPU，以及其他 ARM Mali 设备。

| 仓库                          | 介绍                                                      |
| --------------------------- | ------------------------------------------------------- |
| Panfrost                    | Mesa 的 Mali OpenGL 驱动，支持多代 Mali GPU。                    |
| PanVK kbase                 | 让 Mesa Panfrost/PanVK 直接使用 Android 厂商 Mali kbase 驱动。    |
| panvk-g99-jm                | Mali Valhall Job Manager 路线的 PanVK 实验实现。                |
| mesa-panvk-g57              | Mali-G57 kbase 适配，参考 JM GPU 作业提交。                       |
| panvk-g720-kbase-csf        | Mali-G720 CSF 的 kbase/PanVK 适配研究。                       |
| panvk-Android               | Android Mali G610、G710、G715、G720 等设备的 PanVK/kbase 实验驱动。 |
| panvk-faux123-driver-rp4pro | 面向 Retroid Pocket 4 Pro Mali-G77 的 kbase/PanVK 实验驱动。    |
| Panfrost                    | Mali 驱动上游源码，可研究 GPU 命令提交、内存管理和编译器。                      |
| ARM Mali GPU Tools          | ARM 官方软件组织下的 GPU 工具和相关开发项目。                             |

这里有一个特别值得注意的仓库：`panvk-faux123-driver-rp4pro`，它直接涉及 Mali-G77，而不是只研究 G57 或 G710。

### 4\. MediaTek / 天玑平台相关

天玑不能简单归类成另一种 GPU。它主要使用 ARM Mali 或 Immortalis GPU，但还有独立的 SoC 驱动、内存管理和媒体硬件接口。

| 仓库                  | 介绍                                          |
| ------------------- | ------------------------------------------- |
| Linux MediaTek      | Linux 内核 MediaTek SoC 支持，研究平台资源、时钟、电源和硬件接口。 |
| MTK Linux Kernel    | MediaTek 官方开发组织，包含部分 Linux 平台和硬件相关代码。       |
| Panfrost            | 天玑平台 Mali/Immortalis GPU 的开源图形驱动参考。         |
| PanVK kbase         | 天玑设备上使用 Mali kbase 时可研究的 Vulkan 驱动方案。       |
| ARM Compute Library | ARM CPU/GPU 计算库，研究 Mali 计算任务、优化和异构计算。       |
| Arm NN              | ARM 神经网络推理框架，涉及 ARM CPU、Mali GPU 和计算后端。     |
| clvk                | 在 OpenCL 上实现 Vulkan 计算接口的项目，可研究计算 API 兼容。   |

### 5\. Linux 桌面、GNOME、Wayland、X11

这部分决定 Linuxhub 最终能否真正运行 GNOME，而不是停留在终端或 XFCE。

| 仓库                           | 介绍                                                                      |
| ---------------------------- | ----------------------------------------------------------------------- |
| GNOME Shell                  | GNOME 桌面 Shell、窗口管理和桌面交互。                                               |
| Mutter                       | GNOME 的 Wayland/X11 窗口管理器和合成器。                                          |
| Wayland                      | Linux 图形显示协议及客户端/服务端实现。                                                 |
| Wayland Protocols            | Wayland 窗口、输入、显示和缓冲区协议扩展。                                               |
| wlroots                      | Wayland 合成器开发库，适合研究自定义桌面显示系统。                                           |
| Weston                       | 官方 Wayland 参考合成器。                                                       |
| labwc                        | 基于 wlroots 的轻量 Wayland 合成器。                                             |
| KWin                         | KDE 窗口管理器和 Wayland 合成器，研究窗口与显示管理。                                       |
| Xwayland                     | 让传统 X11 应用运行在 Wayland 桌面中。                                              |
| Termux:X11                   | Android 上的 X11 Server，研究 Android 显示与 Linux 图形程序连接。                      |
| Anland                       | Android 上的 Wayland 显示方案，可研究原生窗口和桌面输出。                                   |
| labwc-android                | Android 上集成 Wayland 桌面、Linux 窗口和 GPU 显示。                                |
| Piperella Wayland Compositor | 直接将 Wayland Compositor 嵌入 Android 应用，通过 SurfaceView 输出图形。               |
| wldroid                      | Android 应用内嵌 wlroots Wayland Compositor，包含 Android 后端和 AHardwareBuffer。 |
| DroidDesk                    | Android Linux 桌面集成，包含终端、X11 和桌面启动。                                      |
| usix-termux                  | Android 上整合 XFCE、KDE Plasma、X11、Wayland 和音频的项目。                         |

这组里，Piperella、wldroid、Mutter、Weston 和 wlroots 特别值得研究。GitHub +3

### 6\. Android 原生显示与 GPU 缓冲区

| 仓库           | 介绍                                                            |
| ------------ | ------------------------------------------------------------- |
| AOSP         | Android 原生图形系统，包括 SurfaceFlinger、BufferQueue 和 Native Window。 |
| Android NDK  | Android Native API、JNI、EGL、Vulkan 和原生窗口开发。                    |
| libdrm       | Linux DRM 用户态库，研究图形设备接口和内存管理。                                 |
| libgbm       | 图形缓冲区管理，涉及 GBM 和 DRM 图形资源。                                    |
| libwayland   | Wayland 客户端和服务端通信。                                            |
| libseat      | Linux 图形会话中的设备访问和 Seat 管理。                                    |
| libinput     | 键盘、鼠标、触控板和触摸输入处理。                                             |
| libxkbcommon | Linux 键盘布局和输入事件处理。                                            |
| libhybris    | Linux 用户态调用 Android Bionic 系统库和 HAL 的兼容桥接技术。                  |

### 7\. 视频、编解码和媒体硬件

| 仓库                | 介绍                                    |
| ----------------- | ------------------------------------- |
| FFmpeg            | Linux 音视频编解码、格式转换和媒体处理框架。             |
| GStreamer         | 模块化音视频管线，包含 Android MediaCodec 接入。    |
| Android NDK Media | Android 原生 MediaCodec、媒体格式和硬件编解码 API。 |
| libav             | 音视频编解码和媒体处理框架。                        |
| libyuv            | 视频像素格式转换、缩放和图像处理。                     |
| libjpeg-turbo     | 高性能 JPEG 编解码和图像处理。                    |
| dav1d             | AV1 软件解码器，可作为硬件解码不可用时的参考。             |
| libvpx            | VP8/VP9 视频编解码。                        |

### 8\. 音频、麦克风和 Linux 声音系统

| 仓库         | 介绍                                      |
| ---------- | --------------------------------------- |
| Oboe       | Android 原生低延迟音频库，封装 AAudio 和 OpenSL ES。 |
| AAudio     | Android 原生音频 API。                       |
| PulseAudio | Linux 用户态音频服务器。                         |
| PipeWire   | Linux 现代音视频和设备管理框架。                     |
| ALSA       | Linux 音频设备接口和用户态库。                      |
| TinyALSA   | 轻量级 ALSA 用户态接口实现。                       |
| SDL        | 游戏和多媒体应用库，涉及音频、输入、窗口和图形。                |
| miniaudio  | 跨平台音频输入输出库，可参考 Android/Linux 音频后端。      |

GStreamer 特别值得关注，因为它可以把 Android MediaCodec 纳入 Linux 用户态媒体管线。GitHub

### 9\. 文件系统、存储、挂载与 RootFS

| 仓库             | 介绍                                  |
| -------------- | ----------------------------------- |
| libfuse        | FUSE 用户态文件系统，可研究虚拟文件系统和文件访问桥接。      |
| fuse-overlayfs | 无特权 OverlayFS 实现，研究 RootFS 分层和写时复制。 |
| squashfs-tools | 压缩只读文件系统制作与解包。                      |
| E2fsprogs      | Ext 文件系统管理和维护工具。                    |
| libarchive     | 压缩包、归档和 RootFS 解包管理。                |
| rclone         | 文件同步、云存储和远程文件管理。                    |
| libtar         | TAR 归档操作，可参考 RootFS 管理。             |

### 10\. 进程、系统调用、Linux ABI 和兼容性

| 仓库                    | 介绍                                       |
| --------------------- | ---------------------------------------- |
| Linux Kernel          | Linux 内核系统调用、进程、线程、内存和设备接口的基础参考。         |
| Android Common Kernel | Android 内核与 Linux 上游之间的差异和 Android 特有接口。 |
| Bionic                | Android 原生 C 库、动态链接器和系统调用封装。             |
| glibc                 | GNU C Library，Linux ARM64 程序运行的基础。       |
| musl                  | 轻量级 C 标准库，可作为另一种 Linux libc 兼容参考。        |
| QEMU                  | ELF、CPU 模拟、Linux 用户态和系统调用转换参考。           |
| Box64                 | 在 ARM64 上运行 x86_64 Linux 程序的动态翻译技术。      |
| FEX                   | ARM64 上运行 x86/x86_64 Linux 程序的兼容层。       |
| gVisor                | 用户态内核与系统调用拦截设计参考。                        |
| bubblewrap            | Linux 无特权沙箱和命名空间管理。                      |
| nsjail                | Linux 进程隔离、Namespace 和 Seccomp 安全机制。     |
| strace                | Linux 系统调用跟踪和调试工具。                       |
| libseccomp            | Seccomp 系统调用过滤规则管理。                      |

注意：这些项目有些需要 Linux 特权、Namespace 或内核支持，不代表能直接嵌入普通 Android APK。主要是借鉴实现和机制。

### 11\. 网络、USB、输入和其他 Android 硬件

| 仓库             | 介绍                                           |
| -------------- | -------------------------------------------- |
| Android NDK    | Android 原生硬件和系统 API 的总入口。                    |
| Termux:API     | Android 与 Linux 用户态之间的摄像头、传感器、剪贴板、通知等能力桥接参考。 |
| libusb         | USB 设备发现、访问和控制。                              |
| libinput       | Linux 输入设备管理。                                |
| libevdev       | Linux 输入事件接口。                                |
| BlueZ          | Linux 蓝牙协议栈和设备管理。                            |
| NetworkManager | Linux 网络接口和网络连接管理。                           |
| ConnMan        | Linux 网络连接管理框架。                              |
| crosvm         | 虚拟机设备模型、VirtIO 和设备后端设计参考。                    |
| scrcpy         | Android 显示、输入、编码和屏幕捕获链路参考。                   |

### 12\. Android 应用与整体产品架构

| 仓库              | 介绍                                                   |
| --------------- | ---------------------------------------------------- |
| DroidNative     | 原生 Android 应用管理 Linux 桌面程序，涉及应用窗口、RootFS、图形和媒体。      |
| MagicDesk       | Android 原生多窗口工作站，整合 Linux 桌面、终端和自动化。                 |
| Linuxify        | 原生 Kotlin/Compose Linux Android 应用，涉及 RootFS、终端和包管理。 |
| pr              | 将 Linux 执行引擎拆成 Android Library，适合研究模块化架构。            |
| wldroid         | 把 RootFS、图形合成器、GPU 和 Android UI 拆成独立模块。              |
| DroidVM         | Linux 虚拟机管理和 GPU 设备通路研究。                             |
| Termux          | Android 原生终端应用、生命周期和环境管理。                            |
| Termux Packages | Android ARM64 软件包构建、交叉编译和依赖管理。                       |
| Andronix        | Android Linux 发行版部署和用户体验参考。                          |
| AnLinux         | Android 上的 Linux 环境安装和管理。                            |

### 最后：我建议优先阅读的 15 个仓库

如果把上面所有项目放在一起，我认为这 15 个最值得 DeepSeekFish 优先研究：

### Husk 核心参考清单

按对整个工程的参考价值排序，不代表直接可移植程度。

1

Shroot

Native Linux Runtime、RootFS、C API

2

elf_loader

ELF、glibc、Bionic、动态链接

3

AndroLinux Runtime

无 Root Android Linux Runtime 架构

4

DroidNative

完整 Linux 桌面应用产品架构

5

Mesa

GPU 驱动总体系

6

cccodroid-gpu-driver-snapdragon

高通 KGSL、Turnip、Zink、glibc

7

Droid-Deck Drivers

Android 与 Linux 双端 GPU 驱动

8

PanVK kbase

Mali kbase 直接接入 Mesa

9

wldroid

Android 原生 Wayland 合成器集成

10

Piperella

Android SurfaceView 与 Wayland 零拷贝

11

Mutter

GNOME 桌面和窗口管理

12

GStreamer

视频编解码和媒体管线

13

Oboe

Android 原生音频

14

Android Common Kernel

Android 内核差异

15

libhybris

Bionic / Android HAL 与 Linux 兼容

一个重要说明： 这里列出的是值得研究的项目，不是已经验证能在 Husk 上运行的依赖清单。有些是成熟上游，有些是实验性仓库，有些需要 Root 或虚拟化，只能参考其内部实现。

尤其是高通、天玑、Mali 三条 GPU 路线，应该分别维护设备适配记录，不能混成一套驱动。

我建议把这份清单作为 Husk 的长期参考仓库库，后续每个模块都从现有开源实现中找可复用代码，而不是重新造轮子。
