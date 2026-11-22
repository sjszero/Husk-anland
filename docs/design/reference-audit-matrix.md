# 参考仓库逐条审计矩阵（120/120）

> 审计日期：2026-11-23  
> 范围：用户原始清单 12 类、120 条，**保留重复项，逐条有结论**。  
> 原始资料：[`reference-request-20261122.md`](reference-request-20261122.md)  
> 机器可读记录：[`reference-audit-20261123.json`](reference-audit-20261123.json)

## 口径

- `✅ GitHub`：用已登录的 `gh api repos/<owner>/<repo>` 核对；★ 与 push 日期是审计时快照。
- `✅ 上游`：官方 Git/GitLab/源码站；用代理实测 HTTP 200。
- `✅ 组件`：不是独立仓库，明确指向它所属的上游组件。
- `❌ UNKNOWN`：清单名称过于泛化或找不到与描述匹配的可信唯一仓库；不猜 URL、不把同名项目冒充原项目。
- `A` = 直接影响 Husk 当前实现，应先读/先复用；`B` = 重要备选或对照；`C` = 外围、产品/生态参考，不阻塞主线。
- 本矩阵完成的是**身份、真实来源、可用性、相关性审计**；不等于每个仓库都已逐行读源码。源码级深读另见 `mali-kbase-prior-art.md` 与证据文件。

## 汇总

| 项 | 数量 |
|---|---:|
| 原始条目 | 120 |
| GitHub 仓库 | 69 |
| 官方上游/组件 | 45 |
| 无法唯一确认 | 6 |
| A 级优先 | 28 |
| B 级备选 | 44 |
| C 级外围 | 48 |

### 结论

1. 清单不是依赖清单。可直接复用的重点是 Mali kbase/PanVK、Wayland/Android Surface、AAudio/MediaCodec 三条线。
2. `proroot` 确实存在，但当前仓库是**闭源二进制 + 限制分发**，只能做架构旁证，不能当 Husk 依赖，也不能抄实现。
3. 未定位到可信来源的 6 项是 `elf_loader`、`mesa-patches`、`Local Desktop`、`ARM Mali GPU Tools`、`usix-termux`、`pr`；继续用名字搜索容易误采同名项目。
4. `mesa-tu8`（实际 `Droid-VM/turnip`）和 `libtar` 镜像已归档；只能读历史实现，不作为活跃上游。
5. 本机当前路线继续以 `FristOneRR-Admin/FristOneRR-Panvk-Source` / `zenithblue-oss/panvk-kbase-android` / `Noysz/panvk-g99-jm` / `faux123/panvk-faux123-driver-rp4pro` 为一级参考。

## 1. Linux Runtime / PRoot 替代方案

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 1 | Shroot | [`mekhontsev/shroot`](https://github.com/mekhontsev/shroot) | ✅ GitHub · ★7 · 2026-11-22 | A | loader/rootfs/C API layering |
| 1 | proroot | [`coderredlab/proroot`](https://github.com/coderredlab/proroot) | ✅ GitHub · ★98 · 2026-06-17 | A | binary-only zero-ptrace runtime；只对照分层，不采用二进制 |
| 1 | zproot | [`zproot/zproot`](https://github.com/zproot/zproot) | ✅ GitHub · ★3 · 2026-11-21 | B | Zig ptrace core；反面/压缩拦截面参考 |
| 1 | proot | [`proot-me/proot`](https://github.com/proot-me/proot) | ✅ GitHub · ★2668 · 2026-10-02 | B | ptrace 成本与路径语义反面基线 |
| 1 | proot-rs | [`proot-me/proot-rs`](https://github.com/proot-me/proot-rs) | ✅ GitHub · ★166 · 2026-06-08 | B | Rust syscall 分派组织方式；架构不采纳 |
| 1 | elf_loader | — | ❌ UNKNOWN | C | 同名仓库均非所述项目；不能伪造链接 |
| 1 | AndroLinux Runtime | [`Meapri/androlinux-runtime-lab`](https://github.com/Meapri/androlinux-runtime-lab) | ✅ GitHub · ★0 · 2026-05-15 | B | 低开销 runtime/GPU bridge 实验索引 |
| 1 | UserLAnd | [`CypherpunkArmory/UserLAnd`](https://github.com/CypherpunkArmory/UserLAnd) | ✅ GitHub · ★4339 · 2026-11-18 | C | 发行版安装/生命周期产品参考 |
| 1 | PRoot Distro | [`termux/proot-distro`](https://github.com/termux/proot-distro) | ✅ GitHub · ★3659 · 2026-11-21 | C | RootFS 发行版管理与启动脚本 |
| 1 | DroidNative | [`DroidNative/DroidNativeApp`](https://github.com/DroidNative/DroidNativeApp) | ✅ GitHub · ★84 · 2026-10-24 | A | 免 root Linux 桌面产品与窗口架构 |
| 1 | Podroid | [`ExTV/Podroid`](https://github.com/ExTV/Podroid) | ✅ GitHub · ★3906 · 2026-11-09 | C | 产品化参考；VM 路线不符合约束 |

## 2. Qualcomm 高通 Adreno GPU

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 2 | Mesa | [官方上游](https://gitlab.freedesktop.org/mesa/mesa) | ✅ 上游 | A | Turnip/Freedreno/Zink/Panfrost/PanVK 上游 |
| 2 | Qualcomm KGSL | [官方上游](https://git.codelinaro.org/clo/la/kernel/msm-4.19) | ✅ 上游 | B | 厂商 KGSL 内核 UAPI；按 SoC 树核对 |
| 2 | Turnip / Freedreno | [所属组件](https://gitlab.freedesktop.org/mesa/mesa/-/tree/main/src/freedreno) | ✅ 组件 | B | Mesa 内 Adreno 驱动组件 |
| 2 | cccodroid-gpu-driver-snapdragon | [`greyoak111/cccodroid-gpu-driver-snapdragon`](https://github.com/greyoak111/cccodroid-gpu-driver-snapdragon) | ✅ GitHub · ★0 · 2026-11-16 | B | 免 root glibc Turnip+Zink 加载形态 |
| 2 | Droid-Deck Drivers | [`Droid-Deck/Drivers`](https://github.com/Droid-Deck/Drivers) | ✅ GitHub · ★42 · 2026-11-21 | B | Android+glibc Turnip/驱动包流水线 |
| 2 | Adreno-Tools-Drivers | [`StevenMXZ/Adreno-Tools-Drivers`](https://github.com/StevenMXZ/Adreno-Tools-Drivers) | ✅ GitHub · ★1344 · 2026-11-19 | B | AdrenoTools 驱动包构建流水线 |
| 2 | mesa-tu8 | [`Droid-VM/turnip`](https://github.com/Droid-VM/turnip) | ✅ GitHub · ★0 · 2026-08-31 · archived | C | A8xx KGSL Turnip fork |
| 2 | mesa-patches | — | ❌ UNKNOWN | C | 泛化名，无法唯一对应可信仓库 |
| 2 | Turnip Faux123 | [`faux123/turnip-faux123-driver`](https://github.com/faux123/turnip-faux123-driver) | ✅ GitHub · ★44 · 2026-11-10 | B | Adreno 8xx Turnip/KGSL 修复 |
| 2 | droidspaces-gpu | [`lfdevs/mesa-for-android-container`](https://github.com/lfdevs/mesa-for-android-container) | ✅ GitHub · ★392 · 2026-11-22 | B | 容器内 glibc GPU 打包与运行基线 |
| 2 | Local Desktop | — | ❌ UNKNOWN | C | 泛化名；搜到的是产品/页面，不是可信驱动源码 |
| 2 | Adreno Tools | [`K11MCH1/AdrenoToolsDrivers`](https://github.com/K11MCH1/AdrenoToolsDrivers) | ✅ GitHub · ★5207 · 2026-11-08 | B | Adreno 驱动分发/加载格式 |

## 3. ARM Mali / 天玑 GPU

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 3 | Panfrost | [所属组件](https://gitlab.freedesktop.org/mesa/mesa/-/tree/main/src/panfrost) | ✅ 组件 | A | 重复项：Mesa Panfrost |
| 3 | PanVK kbase | [`zenithblue-oss/panvk-kbase-android`](https://github.com/zenithblue-oss/panvk-kbase-android) | ✅ GitHub · ★174 · 2026-11-20 | A | PanVK 直连 mali_kbase，核心先例 |
| 3 | panvk-g99-jm | [`Noysz/panvk-g99-jm`](https://github.com/Noysz/panvk-g99-jm) | ✅ GitHub · ★25 · 2026-11-13 | A | Valhall v9 JM，同代路线 |
| 3 | mesa-panvk-g57 | [`apexspan-svg/mesa-panvk-mali-g57`](https://github.com/apexspan-svg/mesa-panvk-mali-g57) | ✅ GitHub · ★3 · 2026-10-01 | A | G57 kbase+X11 窗口参考 |
| 3 | panvk-g720-kbase-csf | [`wonderkast02/panvk-g720-kbase-csf`](https://github.com/wonderkast02/panvk-g720-kbase-csf) | ✅ GitHub · ★54 · 2026-10-10 | C | CSF 路线；本机不适用 |
| 3 | panvk-Android | [`zenithblue-oss/panvk-kbase-android`](https://github.com/zenithblue-oss/panvk-kbase-android) | ✅ GitHub · ★174 · 2026-11-20 | B | 多个 Android Mali 设备的 kbase 构建 |
| 3 | panvk-faux123-driver-rp4pro | [`faux123/panvk-faux123-driver-rp4pro`](https://github.com/faux123/panvk-faux123-driver-rp4pro) | ✅ GitHub · ★25 · 2026-11-16 | A | G77/r32p1/Android13/4.19 最贴近先例 |
| 3 | Panfrost | [所属组件](https://gitlab.freedesktop.org/mesa/mesa/-/tree/main/src/panfrost) | ✅ 组件 | A | 重复项：Mesa Panfrost |
| 3 | ARM Mali GPU Tools | — | ❌ UNKNOWN | C | 未定位到与描述匹配的唯一官方仓库 |

## 4. MediaTek / 天玑平台相关

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 4 | Linux MediaTek | [官方上游](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git) | ✅ 上游 | B | Linux 上游 MediaTek 平台支持 |
| 4 | MTK Linux Kernel | [官方上游](https://git.kernel.org/pub/scm/linux/kernel/git/mediatek/linux.git) | ✅ 上游 | B | MediaTek 内核树；设备具体版本另取厂商源码 |
| 4 | Panfrost | [所属组件](https://gitlab.freedesktop.org/mesa/mesa/-/tree/main/src/panfrost) | ✅ 组件 | A | 重复项：Mesa Panfrost |
| 4 | PanVK kbase | [`zenithblue-oss/panvk-kbase-android`](https://github.com/zenithblue-oss/panvk-kbase-android) | ✅ GitHub · ★174 · 2026-11-20 | A | 重复项：PanVK kbase |
| 4 | ARM Compute Library | [`ARM-software/ComputeLibrary`](https://github.com/ARM-software/ComputeLibrary) | ✅ GitHub · ★3213 · 2026-11-18 | C | Mali 计算/异构优化参考 |
| 4 | Arm NN | [`ARM-software/armnn`](https://github.com/ARM-software/armnn) | ✅ GitHub · ★1318 · 2026-11-10 | C | ARM CPU/GPU 推理后端参考 |
| 4 | clvk | [`kpet/clvk`](https://github.com/kpet/clvk) | ✅ GitHub · ★456 · 2026-11-20 | C | OpenCL 上实现 Vulkan compute；本阶段外围 |

## 5. Linux 桌面、GNOME、Wayland、X11

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 5 | GNOME Shell | [官方上游](https://gitlab.gnome.org/GNOME/gnome-shell) | ✅ 上游 | B | 2.0 桌面实际 Shell |
| 5 | Mutter | [官方上游](https://gitlab.gnome.org/GNOME/mutter) | ✅ 上游 | B | GNOME Wayland/X11 合成器 |
| 5 | Wayland | [官方上游](https://gitlab.freedesktop.org/wayland/wayland) | ✅ 上游 | B | 显示协议核心 |
| 5 | Wayland Protocols | [官方上游](https://gitlab.freedesktop.org/wayland/wayland-protocols) | ✅ 上游 | B | 协议扩展 |
| 5 | wlroots | [官方上游](https://gitlab.freedesktop.org/wlroots/wlroots) | ✅ 上游 | B | 可嵌入合成器库 |
| 5 | Weston | [官方上游](https://gitlab.freedesktop.org/wayland/weston) | ✅ 上游 | C | 参考合成器 |
| 5 | labwc | [`labwc/labwc`](https://github.com/labwc/labwc) | ✅ GitHub · ★2927 · 2026-11-21 | C | 轻量 wlroots 合成器 |
| 5 | KWin | [官方上游](https://invent.kde.org/plasma/kwin) | ✅ 上游 | C | KDE 合成器参考 |
| 5 | Xwayland | [官方上游](https://gitlab.freedesktop.org/xorg/xserver) | ✅ 上游 | B | X11 兼容路径 |
| 5 | Termux:X11 | [`termux/termux-x11`](https://github.com/termux/termux-x11) | ✅ GitHub · ★5194 · 2026-11-16 | A | Android Surface/共享内存/输入成熟基线 |
| 5 | Anland | [`SuperTurtleDev/anland`](https://github.com/SuperTurtleDev/anland) | ✅ GitHub · ★362 · 2026-11-21 | A | Android Wayland 输出，2.0/容器实际采用 |
| 5 | labwc-android | [`adittaya/labwc-android`](https://github.com/adittaya/labwc-android) | ✅ GitHub · ★0 · 2026-07-09 | B | Android 原生 Wayland 桌面整合 |
| 5 | Piperella Wayland Compositor | [`Piperella/Piperella-Wayland-Compositor`](https://github.com/Piperella/Piperella-Wayland-Compositor) | ✅ GitHub · ★6 · 2026-08-07 | A | 合成器作为 Gradle 库嵌入 APK |
| 5 | wldroid | [`Shelnutt2/wldroid`](https://github.com/Shelnutt2/wldroid) | ✅ GitHub · ★4 · 2026-05-26 | A | wlroots 嵌 Android + AHardwareBuffer |
| 5 | DroidDesk | [`orailnoor/DroidDesk`](https://github.com/orailnoor/DroidDesk) | ✅ GitHub · ★4875 · 2026-10-27 | C | 成品桌面产品参考 |
| 5 | usix-termux | — | ❌ UNKNOWN | C | 未定位到可信唯一仓库 |

## 6. Android 原生显示与 GPU 缓冲区

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 6 | AOSP | [官方上游](https://android.googlesource.com/platform/frameworks/native/) | ✅ 上游 | A | SurfaceFlinger/BufferQueue/NativeWindow 权威源 |
| 6 | Android NDK | [官方上游](https://github.com/android/ndk) | ✅ 上游 | B | Native API 与 ABI 权威入口 |
| 6 | libdrm | [官方上游](https://gitlab.freedesktop.org/mesa/drm) | ✅ 上游 | B | DRM UAPI 用户态封装 |
| 6 | libgbm | [所属组件](https://gitlab.freedesktop.org/mesa/mesa/-/tree/main/src/gbm) | ✅ 组件 | B | Mesa 内 GBM 组件 |
| 6 | libwayland | [所属组件](https://gitlab.freedesktop.org/wayland/wayland) | ✅ 组件 | B | Wayland 核心库 |
| 6 | libseat | [官方上游](https://git.sr.ht/~kennylevinsen/seatd) | ✅ 上游 | C | seat 设备访问管理 |
| 6 | libinput | [官方上游](https://gitlab.freedesktop.org/libinput/libinput) | ✅ 上游 | B | Linux 输入设备处理 |
| 6 | libxkbcommon | [`xkbcommon/libxkbcommon`](https://github.com/xkbcommon/libxkbcommon) | ✅ GitHub · ★381 · 2026-11-11 | B | 键盘布局处理 |
| 6 | libhybris | [`libhybris/libhybris`](https://github.com/libhybris/libhybris) | ✅ GitHub · ★866 · 2026-06-24 | A | glibc 调 bionic HAL/EGL 的第二路线 |

## 7. 视频、编解码和媒体硬件

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 7 | FFmpeg | [官方上游](https://git.ffmpeg.org/ffmpeg.git) | ✅ 上游 | B | guest 解复用/同步权威实现 |
| 7 | GStreamer | [官方上游](https://gitlab.freedesktop.org/gstreamer/gstreamer) | ✅ 上游 | A | guest 媒体管线与 VA 后端接入点 |
| 7 | Android NDK Media | [官方上游](https://android.googlesource.com/platform/frameworks/av/) | ✅ 上游 | A | AMediaCodec/MediaCodec 权威 ABI |
| 7 | libav | [所属组件](https://git.ffmpeg.org/ffmpeg.git) | ✅ 组件 | C | FFmpeg 的库组件，不是独立项目 |
| 7 | libyuv | [官方上游](https://chromium.googlesource.com/libyuv/libyuv/) | ✅ 上游 | B | 像素格式转换/缩放 |
| 7 | libjpeg-turbo | [`libjpeg-turbo/libjpeg-turbo`](https://github.com/libjpeg-turbo/libjpeg-turbo) | ✅ GitHub · ★4472 · 2026-11-21 | C | JPEG 软解/图像处理 |
| 7 | dav1d | [`videolan/dav1d`](https://github.com/videolan/dav1d) | ✅ GitHub · ★830 · 2026-11-20 | C | AV1 软解兜底 |
| 7 | libvpx | [`webmproject/libvpx`](https://github.com/webmproject/libvpx) | ✅ GitHub · ★980 · 2026-11-20 | C | VP8/VP9 编解码兜底 |

## 8. 音频、麦克风和 Linux 声音系统

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 8 | Oboe | [`google/oboe`](https://github.com/google/oboe) | ✅ GitHub · ★4139 · 2026-11-21 | A | AAudio 低延迟实践与封装 |
| 8 | AAudio | [所属组件](https://android.googlesource.com/platform/frameworks/av/) | ✅ 组件 | A | bionic 侧 AAudio 权威 ABI |
| 8 | PulseAudio | [官方上游](https://gitlab.freedesktop.org/pulseaudio/pulseaudio) | ✅ 上游 | C | guest 声音栈参考 |
| 8 | PipeWire | [官方上游](https://gitlab.freedesktop.org/pipewire/pipewire) | ✅ 上游 | C | guest 音视频设备框架参考 |
| 8 | ALSA | [官方上游](https://github.com/alsa-project/alsa-lib) | ✅ 上游 | C | Linux 音频 ABI/用户态参考 |
| 8 | TinyALSA | [所属组件](https://android.googlesource.com/platform/external/tinyalsa/) | ✅ 组件 | C | bionic 侧轻量 ALSA 参考 |
| 8 | SDL | [`libsdl-org/SDL`](https://github.com/libsdl-org/SDL) | ✅ GitHub · ★17053 · 2026-11-22 | C | 音频/窗口兜底参考 |
| 8 | miniaudio | [`mackron/miniaudio`](https://github.com/mackron/miniaudio) | ✅ GitHub · ★7692 · 2026-11-19 | C | 轻量跨平台音频后端参考 |

## 9. 文件系统、存储、挂载与 RootFS

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 9 | libfuse | [`libfuse/libfuse`](https://github.com/libfuse/libfuse) | ✅ GitHub · ★6182 · 2026-11-22 | B | 用户态文件系统参考 |
| 9 | fuse-overlayfs | [`containers/fuse-overlayfs`](https://github.com/containers/fuse-overlayfs) | ✅ GitHub · ★707 · 2026-09-11 | B | 无特权 overlay/写时复制参考 |
| 9 | squashfs-tools | [`plougher/squashfs-tools`](https://github.com/plougher/squashfs-tools) | ✅ GitHub · ★939 · 2026-11-08 | C | 只读镜像制作/解包 |
| 9 | E2fsprogs | [`tytso/e2fsprogs`](https://github.com/tytso/e2fsprogs) | ✅ GitHub · ★489 · 2026-11-22 | C | ext 文件系统工具 |
| 9 | libarchive | [`libarchive/libarchive`](https://github.com/libarchive/libarchive) | ✅ GitHub · ★3677 · 2026-11-22 | B | RootFS 归档解包 Rust 替代的语义基线 |
| 9 | rclone | [`rclone/rclone`](https://github.com/rclone/rclone) | ✅ GitHub · ★61183 · 2026-11-22 | C | 文件同步，外围 |
| 9 | libtar | [`tklauser/libtar`](https://github.com/tklauser/libtar) | ✅ GitHub · ★47 · 2021-06-20 · archived | C | TAR 管理参考 |

## 10. 进程、系统调用、Linux ABI 和兼容性

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 10 | Linux Kernel | [官方上游](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git) | ✅ 上游 | A | syscall/进程/内存/设备 UAPI 权威源 |
| 10 | Android Common Kernel | [官方上游](https://android.googlesource.com/kernel/common/) | ✅ 上游 | A | Android 内核差异与 vendor hooks |
| 10 | Bionic | [官方上游](https://android.googlesource.com/platform/bionic/) | ✅ 上游 | A | Android libc/loader/syscall 权威源 |
| 10 | glibc | [官方上游](https://sourceware.org/git/glibc.git) | ✅ 上游 | A | guest libc/loader 权威源 |
| 10 | musl | [官方上游](https://git.musl-libc.org/cgit/musl) | ✅ 上游 | B | 轻量 libc/ABI 对照 |
| 10 | QEMU | [官方上游](https://gitlab.com/qemu-project/qemu) | ✅ 上游 | B | ELF/用户态 ABI 转换参考 |
| 10 | Box64 | [`ptitSeb/box64`](https://github.com/ptitSeb/box64) | ✅ GitHub · ★5723 · 2026-11-22 | C | 跨架构用户态翻译参考 |
| 10 | FEX | [`FEX-Emu/FEX`](https://github.com/FEX-Emu/FEX) | ✅ GitHub · ★8442 · 2026-11-22 | C | 跨架构用户态翻译参考 |
| 10 | gVisor | [`google/gvisor`](https://github.com/google/gvisor) | ✅ GitHub · ★20083 · 2026-11-22 | B | 用户态内核/syscall 设计参考 |
| 10 | bubblewrap | [`containers/bubblewrap`](https://github.com/containers/bubblewrap) | ✅ GitHub · ★9588 · 2026-10-14 | C | 无特权命名空间参考；不是本机主路线 |
| 10 | nsjail | [`google/nsjail`](https://github.com/google/nsjail) | ✅ GitHub · ★4201 · 2026-11-20 | C | 进程隔离/namespace/seccomp 参考 |
| 10 | strace | [`strace/strace`](https://github.com/strace/strace) | ✅ GitHub · ★2727 · 2026-11-18 | B | 观测工具；非产品依赖 |
| 10 | libseccomp | [`seccomp/libseccomp`](https://github.com/seccomp/libseccomp) | ✅ GitHub · ★939 · 2026-11-20 | C | syscall 过滤参考 |

## 11. 网络、USB、输入和其他 Android 硬件

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 11 | Android NDK | [官方上游](https://github.com/android/ndk) | ✅ 上游 | B | 重复项：Native API 入口 |
| 11 | Termux:API | [`termux/termux-api`](https://github.com/termux/termux-api) | ✅ GitHub · ★4692 · 2026-09-17 | B | Android↔Linux 能力桥接参考 |
| 11 | libusb | [`libusb/libusb`](https://github.com/libusb/libusb) | ✅ GitHub · ★6201 · 2026-11-17 | C | USB 访问参考 |
| 11 | libinput | [官方上游](https://gitlab.freedesktop.org/libinput/libinput) | ✅ 上游 | B | 重复项：输入处理 |
| 11 | libevdev | [官方上游](https://gitlab.freedesktop.org/libevdev/libevdev) | ✅ 上游 | C | 输入事件接口 |
| 11 | BlueZ | [官方上游](https://git.kernel.org/pub/scm/bluetooth/bluez.git) | ✅ 上游 | C | 蓝牙协议栈参考 |
| 11 | NetworkManager | [`NetworkManager/NetworkManager`](https://github.com/NetworkManager/NetworkManager) | ✅ GitHub · ★519 · 2026-11-20 | C | 网络管理参考 |
| 11 | ConnMan | [官方上游](https://git.kernel.org/pub/scm/network/connman/connman.git) | ✅ 上游 | C | 网络管理参考 |
| 11 | crosvm | [官方上游](https://chromium.googlesource.com/chromiumos/platform/crosvm/) | ✅ 上游 | C | 设备模型参考；VM 不符合约束 |
| 11 | scrcpy | [`Genymobile/scrcpy`](https://github.com/Genymobile/scrcpy) | ✅ GitHub · ★155401 · 2026-11-22 | B | 显示/输入/编码链路参考 |

## 12. Android 应用与整体产品架构

| # | 清单名 | 真实来源 | 状态 | 级别 | Husk 用法 |
|---:|---|---|---|---|---|
| 12 | DroidNative | [`DroidNative/DroidNativeApp`](https://github.com/DroidNative/DroidNativeApp) | ✅ GitHub · ★84 · 2026-10-24 | A | 重复项：免 root Linux 桌面产品 |
| 12 | MagicDesk | [`mekhontsev/magicdesk`](https://github.com/mekhontsev/magicdesk) | ✅ GitHub · ★243 · 2026-11-22 | B | 原生多窗口工作站产品参考 |
| 12 | Linuxify | [`Bilal140202/linuxify`](https://github.com/Bilal140202/linuxify) | ✅ GitHub · ★2 · 2026-11-07 | C | Kotlin/Compose+proot 产品化壳 |
| 12 | pr | — | ❌ UNKNOWN | C | 名字过泛，无法确认所述 Android Library |
| 12 | wldroid | [`Shelnutt2/wldroid`](https://github.com/Shelnutt2/wldroid) | ✅ GitHub · ★4 · 2026-05-26 | A | 重复项：wlroots + AHardwareBuffer |
| 12 | DroidVM | [`Droid-VM/DroidVM`](https://github.com/Droid-VM/DroidVM) | ✅ GitHub · ★1110 · 2026-11-21 | C | VM/GPU 设备通路参考；路线不符 |
| 12 | Termux | [`termux/termux-app`](https://github.com/termux/termux-app) | ✅ GitHub · ★65467 · 2026-11-05 | B | 生命周期/终端服务成熟样本 |
| 12 | Termux Packages | [`termux/termux-packages`](https://github.com/termux/termux-packages) | ✅ GitHub · ★17433 · 2026-11-22 | B | Android ARM64 构建/依赖管理 |
| 12 | Andronix | [`AndronixApp/AndronixOrigin`](https://github.com/AndronixApp/AndronixOrigin) | ✅ GitHub · ★3023 · 2026-10-01 | C | 发行版部署 UX 参考 |
| 12 | AnLinux | [`EXALAB/AnLinux-App`](https://github.com/EXALAB/AnLinux-App) | ✅ GitHub · ★2370 · 2026-11-05 | C | Linux 安装/管理 UX 参考 |

## 源码级深读顺序

1. Mali kbase/PanVK：`FristOneRR-Admin/FristOneRR-Panvk-Source`、`zenithblue-oss/panvk-kbase-android`、`Noysz/panvk-g99-jm`、`faux123/panvk-faux123-driver-rp4pro`、`apexspan-svg/mesa-panvk-mali-g57`。
2. Android 显示/窗口：`aosp-mirror/platform_frameworks_native`（AOSP 权威源）、`termux/termux-x11`、`SuperTurtleDev/anland`、`Shelnutt2/wldroid`、`Piperella/Piperella-Wayland-Compositor`。
3. 媒体/音频：AOSP `frameworks/av`、`GStreamer/gstreamer`、`google/oboe`、`libyuv` 的官方上游。
4. Runtime 旁证：`mekhontsev/shroot`、`coderredlab/proroot`（只读 README/产物结构）、`DroidNative/DroidNativeApp`。
5. 其余 B/C 项只在具体模块出现新问题时回查，避免“为了审计而审计”拖慢主线。
