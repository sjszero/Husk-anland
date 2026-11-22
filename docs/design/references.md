# 参考仓库审计（Husk 长期参考库）

> 用途：**先找前人现成实现，再决定要不要自己写**。每开一个新模块，先在这张表里找；
> 找不到再自己实现，实现完把结论补回本表。
>
> 来源：用户提供的一份第三方清单（12 类、~70 项）。清单本身是别的 AI 生成的，
> **名字不等于仓库**，所以本表逐条核对真实路径与 star（核对时间 2026-11-22，`gh api`）。
> 核对状态：✅ 存在且已核对 · 🔎 未核对（上游在 gitlab/git.kernel.org 等非 GitHub 源） · ❌ 查无此仓库
>
> 约束提醒：这些是**参考**，不是依赖清单。有的要 root / 虚拟化 / 特权，只能借思路。

## 0. 结论先行（对本项目影响最大的三条）

1. **Mali kbase 直连 Mesa 已经有人做成开源项目，而且有 Mali-G77 的专门构建**
   （`faux123/panvk-faux123-driver-rp4pro`，25★；`zenithblue-oss/panvk-kbase-android` 174★）。
   ⇒ 我们自己手搓的「DRM 语义层」不再是唯一路线，必须先把这两条读透再决定（见 §3）。
2. **容器里跑 Mali GPU 已有第二代方案**：`ravindu644/Droidspaces-Mali`（libhybris + Mesa Zink）、
   `lfdevs/mesa-for-android-container`（392★，2.0 里那个 mesa tar 就是它的产物）。
3. **免 root 跑完整 Linux 桌面**最接近我们产品形态的是 `DroidNative/DroidNativeApp`（84★）、
   `ExTV/Podroid`（3906★，但走 VM，路线不符）、`Shelnutt2/wldroid`（Wayland 合成器嵌 Android）。

## 1. Linux Runtime / proot 替代

| 清单名 | 真实仓库 | ★ | 借鉴点 / 采纳 |
|---|---|---|---|
| proot | `proot-me/proot` 🔎 | ~6k | 反面教材：ptrace 拦截的全部代价清单（我们就是要去掉它） |
| proot-rs | `proot-me/proot-rs` ✅ | 166 | Rust 版 ptrace 沙箱；看它怎么组织 syscall 分派，但**架构不采纳** |
| Podroid | `ExTV/Podroid` ✅ | 3906 | 无 root 起 Alpine + 容器；**VM 路线，不符合约束**，只看产品化包装 |
| AndroLinux Runtime | `Meapri/androlinux-runtime-lab` ✅ | 0 | 低开销 runtime + GPU bridge 实验，规模小，当索引看 |
| Shroot | — | — | ❌ 未找到；待用别的关键词再核 |
| proroot | — | — | ❌ 未找到 |
| zproot（Zig 重写 proot） | — | — | ❌ 未找到 |
| elf_loader | — | — | ❌ 未找到（"Bionic 进程里加载 glibc ELF" 这个题很关键，值得单独再找） |
| UserLAnd / PRoot Distro / AnLinux / Andronix | 上游 | 🔎 | 产品化/安装器 UX 参考，技术无新东西 |

## 2. 高通 Adreno（本机不是高通，留作第二条 GPU 家族）

| 清单名 | 真实仓库 | ★ | 借鉴点 / 采纳 |
|---|---|---|---|
| cccodroid-gpu-driver-snapdragon | `greyoak111/cccodroid-gpu-driver-snapdragon` ✅ | 0 | 免 root 让**命令行程序**用 Adreno（Turnip + Zink + 厂商 OpenCL）——和我们的形态最像，读它的加载方式 |
| Adreno-Tools-Drivers | `StevenMXZ/Adreno-Tools-Drivers` ✅ / `K11MCH1/AdrenoToolsDrivers` ✅ | 1342 / 5207 | 驱动包格式与构建流水线 |
| mesa-tu8 | `Droid-VM/turnip` ✅ | 0 | A8xx KGSL Turnip fork |
| droidspaces-gpu | → 见 `lfdevs/mesa-for-android-container` ✅ 392 | | 容器内 GPU 的通用做法 |

## 3. ARM Mali / 天玑（**本机就是这条线，最高优先级**）

| 清单名 | 真实仓库 | ★ | 借鉴点 / 采纳状态 |
|---|---|---|---|
| PanVK kbase | `zenithblue-oss/panvk-kbase-android` ✅ | 174 | **Mesa PanVK 直连厂商 mali_kbase**，arch v6–v14，含构建与加载脚本。要读：它怎么绕过 DRM、怎么喂 gpuprops、怎么提交 atom |
| panvk-faux123-driver-**rp4pro** | `faux123/panvk-faux123-driver-rp4pro` ✅ | 25 | **Mali-G77 + kbase + Android**：和本机同代 GPU，JM 路线。第一优先读 |
| panvk-faux123-driver-rg556 | `faux123/panvk-faux123-driver-rg556` ✅ | 34 | G57（JM），比 G77 更早一代，可对照 |
| panvk-faux123-driver-ace / pixel7 | `faux123/...-ace` ✅ 25 / `...-pixel7` ✅ 10 | | G610 / G710（CSF），本机不适用，当差异对照 |
| panvk-g720-kbase-csf | `wonderkast02/panvk-g720-kbase-csf` ✅ | 54 | CSF kbase；本机 ENOTTY（evidence 41），只做参照 |
| panvk-mtk-driver | `BkaNeko/panvk-mtk-driver` ✅ | 11 | **联发科** Mali 的 panvk kbase 构建 |
| mali-panvk | `JimVulkan/mali-panvk` ✅ | 126 | PanVK 移植主线之一；有 JM 分支衍生（`MineGraph-bilibili/panvk-JM-kirin` ✅ 1，kbase UAPI 11.x —— 与本机 11.38 同档） |
| PanPlay | `zenithblue-oss/panplay` ✅ | 30 | 把 panvk kbase 打包进 Wine/FEX 启动器，看它怎么随包分发驱动 |
| mesa-kbase-j4lte | `manoedinata/mesa-kbase-j4lte` ✅ | 0 | 提到 **libdrm-kbase**（把 libdrm 架在 kbase 上）+ gralloc mapper，Exynos 7570 小设备，代码量小好读 |
| Droidspaces-Mali | `ravindu644/Droidspaces-Mali` ✅ | 10 | **容器内 Mali GPU + KDE**：libhybris + Mesa Zink 路线（与 kbase 直连并列的第二方案） |
| panvk-g99-jm / mesa-panvk-g57 | 未定位 | — | ❌ 名字不准，语义已由上面 faux123 系列覆盖 |
| ARM Compute Library / Arm NN / clvk | 上游 | 🔎 | 计算栈，与本阶段无关 |

## 4. 联发科 / 天玑平台

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| Linux MediaTek / MTK Linux Kernel | git.kernel.org / 厂商开源站 🔎 | — | 本机内核 4.19.191 r32p1 源码已在本地 `/tmp/r32_*`（先前取得） |
| Mesa / Panfrost | `Mesa3D/mesa` 🔎(gitlab 主源) | — | panfrost/panvk 上游 |

## 5. Linux 桌面 / Wayland / X11

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| wldroid | `Shelnutt2/wldroid` ✅ | 4 | **wlroots 合成器嵌 Android**，库形态，AHardwareBuffer 输出 |
| labwc-android | `adittaya/labwc-android` ✅ | 0 | Android 上的原生 Wayland 桌面 + GPU |
| Termux:X11 | `termux/termux-x11` ✅ | 5194 | Android 显示 ↔ X11 的成熟做法（LDS/共享内存/输入），我们显示层的对照基线 |
| Piperella / Anland / DroidDesk / usix-termux | 未定位 | — | ❌ 名字不准，待再核 |
| Mutter / GNOME Shell / wlroots / Weston / KWin / Xwayland | 上游 | 🔎 | 桌面本体，我们只负责载体，不改它们 |

## 6. Android 原生显示与缓冲

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| libhybris | `libhybris/libhybris` ✅ | 866 | **glibc 进程里用 bionic 的 HAL/EGL**：`Droidspaces-Mali` 就是靠它跑 Mali。与我们的「kbase 直连」并列的第二方案 |
| android-vulkan-bridge | `afeimod/android-vulkan-bridge` ✅(镜像 2★) | | LXC 里用 libhybris 接 Android Vulkan |
| AOSP / NDK / libdrm / libgbm / libwayland / libseat / libinput / libxkbcommon | 上游 | 🔎 | 接口与语义权威来源 |

## 7. 视频 / 编解码

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| GStreamer | `GStreamer/gstreamer` 🔎 | ~2k | 媒体管线；我们要在 guest 内提供 VA-API 后端给它 |
| FFmpeg / libav | `FFmpeg/FFmpeg` 🔎 | ~50k | 解复用/同步留在 guest |
| Android NDK Media | `aosp-mirror/platform_frameworks_av` 🔎 | — | AMediaCodec 的 ABI 权威来源 |
| dav1d / libvpx / libyuv / libjpeg-turbo | 上游 | 🔎 | 软解兜底 |

## 8. 音频

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| Oboe | `google/oboe` 🔎 | ~2k | AAudio 之上的一层，含低延迟实践与坑位说明 |
| AAudio / TinyALSA | AOSP 🔎 | — | AAudio 直接 dlopen 即可，TinyALSA 是参考 |
| PulseAudio / PipeWire / ALSA | 上游 | 🔎 | guest 内声音栈；我们只提供「一块能读写的 PCM 设备」 |
| SDL / miniaudio | 上游 | 🔎 | 兜底参考 |

## 9. 文件系统 / RootFS

libfuse、fuse-overlayfs、squashfs-tools、e2fsprogs、libarchive、rclone、libtar —— 全部上游 🔎。
借鉴点：rootfs 分层与写时复制（未来 sparse image / overlay 用），本阶段不阻塞。

## 10. 进程 / syscall / Linux ABI

proot（反面）、gVisor（用户态内核设计）、bubblewrap / nsjail（命名空间，需特权，仅读设计）、
strace / libseccomp（观测与过滤）、Bionic / glibc / musl / QEMU / Box64 / FEX（ELF 与 ABI 权威）—— 上游 🔎。

## 11. 网络 / USB / 输入 / 其他硬件

Termux:API（Android↔Linux 能力桥）、libusb、libinput、libevdev、BlueZ、NetworkManager、ConnMan、
crosvm（设备模型）、scrcpy（编码链路）—— 上游 🔎。

## 12. Android 应用 / 产品架构

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| DroidNative | `DroidNative/DroidNativeApp` ✅ | 84 | **每个 Linux 桌面程序一个 Android 窗口**、免 root；产品形态最接近 2.0 的替代品 |
| MagicDesk / Linuxify / pr / DroidVM / Termux / Termux Packages / Andronix / AnLinux | 未定位或上游 | — | 待再核；Termux 本身（`termux/termux-app` ✅ 已知 ~40k）是生命周期/服务的成熟样本 |

---

## 网络：GitHub 连不上就开代理（用完关掉）

本机有代理，端口 `127.0.0.1:7897`（mixed，HTTP/SOCKS 同口）。实测（2026-11-22）：
直连 `api.github.com` 偶发超时、`git clone`/`ls-remote` 会 `Recv failure: Operation timed out`；
走代理 `200 / 0.24s`。

```sh
export ALL_PROXY=http://127.0.0.1:7897 https_proxy=http://127.0.0.1:7897
# 拉仓库/查 API 走这里
unset ALL_PROXY https_proxy   # 用完立刻关（用户要求：用完就关）
```

只用一条命令级的 `-x` 也行，不污染 shell：

```sh
curl -x http://127.0.0.1:7897 -sSL <url>
GIT_CONFIG_GLOBAL=/dev/null git -c http.proxy=http://127.0.0.1:7897 clone --depth 1 <url>
```

## 审计进度

- 2026-11-22：第一遍核对（`gh api` 逐条）。已确认存在 **24** 个；**7** 个名字查无此仓库（清单来自第三方 AI，存在编造/记错）。
- GitHub search API 配额用完，剩余 ~15 项待核（❌/未定位）。核对方式改为 `gh api repos/<path>` + `raw.githubusercontent.com` README。

## 拉取规则（怎么"拉下来"）

```sh
# 只读参考，不进本仓库（避免 1GB 级仓库污染 git）
mkdir -p /tmp/refs && cd /tmp/refs
gh api repos/<owner>/<repo>/tarball/HEAD | tar xz --strip-components=1 -C <owner>_<repo>
# 记录审计用的 commit
gh api repos/<owner>/<repo> --jq '.full_name+" @"+.default_branch+" "+(.pushed_at)'
```

注意：`zenithblue-oss/panvk-kbase-android` 仓库体积 ~1.09 GB（含预编译二进制），拉取前先看 `size`。
