# 参考仓库审计（Husk 长期参考库）

> 用途：**先找前人现成实现，再决定要不要自己写**。每开一个新模块，先在这张表里找；
> 找不到再自己实现，实现完把结论补回本表。
>
> 来源：用户提供的一份第三方清单（12 类、120 个条目，含重复项；逐字归档于
> [`reference-request-20261122.md`](reference-request-20261122.md)）。清单本身是别的 AI 生成的，
> **名字不等于仓库**。本文件按功能合并，保留面向实现的结论；逐条 120/120 的最终审计矩阵在
> [`reference-audit-matrix.md`](reference-audit-matrix.md)，机器可读快照在
> [`reference-audit-20261123.json`](reference-audit-20261123.json)。
> 核对状态：✅ GitHub/官方上游已核对 · 🔎 仅作外围参考 · ❌ 无法唯一确认（不猜链接）
>
> 约束提醒：这些是**参考**，不是依赖清单。有的要 root / 虚拟化 / 特权，只能借思路。

> **强制开工门禁（2026-11-23）**：每个新模块或新子路线在写代码前，必须先检索本文件、
> `reference-audit-matrix.md` 和用户保存的原始清单；涉及 LinuxHub 2.0 已有能力时，还必须只读检查
> 2.0 对应源码。模块设计文档要记录“读过的仓库 + 固定 commit + 可复用点 + 不采用理由”。
> 若已有成熟实现，优先复用其协议/边界/测试，不自造同名轮子。音频的首份深读记录见
> [`audio-prior-art.md`](audio-prior-art.md)。

## 0. 结论先行（对本项目影响最大的三条）

1. **Mali kbase 直连 Mesa 已经有人做成开源项目，而且有 Mali-G77 的专门构建**
   （`faux123/panvk-faux123-driver-rp4pro`，25★；`zenithblue-oss/panvk-kbase-android` 174★）。
   ⇒ 我们自己手搓的「DRM 语义层」不再是唯一路线，必须先把这两条读透再决定（见 §3）。
2. **容器里跑 Mali GPU 已有第二代方案**：`ravindu644/Droidspaces-Mali`（libhybris + Mesa Zink）、
   `lfdevs/mesa-for-android-container`（392★，2.0 里那个 mesa tar 就是它的产物）。
3. **免 root 跑完整 Linux 桌面**最接近我们产品形态的是 `DroidNative/DroidNativeApp`（84★）、
   `ExTV/Podroid`（3906★，但走 VM，路线不符）、`Shelnutt2/wldroid`（Wayland 合成器嵌 Android）。

## 0.5 深读记录

源码级读过、并且会直接影响实现的：见 [`mali-kbase-prior-art.md`](mali-kbase-prior-art.md)
（`mesa-kbase-j4lte` 的 libdrm-kbase 1785 行、`panvk-mtk-driver` 的 MTK 补丁、`panvk-kbase-android` 的补丁面）。

## 1. Linux Runtime / proot 替代

| 清单名 | 真实仓库 | ★ | 借鉴点 / 采纳 |
|---|---|---|---|
| proot | `proot-me/proot` 🔎 | ~6k | 反面教材：ptrace 拦截的全部代价清单（我们就是要去掉它） |
| proot-rs | `proot-me/proot-rs` ✅ | 166 | Rust 版 ptrace 沙箱；看它怎么组织 syscall 分派，但**架构不采纳** |
| Podroid | `ExTV/Podroid` ✅ | 3906 | 无 root 起 Alpine + 容器；**VM 路线，不符合约束**，只看产品化包装 |
| AndroLinux Runtime | `Meapri/androlinux-runtime-lab` ✅ | 0 | 低开销 runtime + GPU bridge 实验，规模小，当索引看 |
| Shroot | `mekhontsev/shroot` ✅ | 7 | **原生 ARM64 Linux runtime**（持久虚拟文件系统、C API），同一作者的 MagicDesk 是同族产品 |
| zproot（Zig 重写 proot） | `zproot/zproot` ✅ + `zproot/zproot-android` ✅ | 3 | Zig 写的 ptrace 核心；看它怎样把拦截面压到最小 |
| elf_loader | 未定位到对应项目（同名仓库均无关） | — | ❌ 清单此项不可靠；"Bionic 进程里加载 glibc ELF" 题由我们自己解（已有 loader 方案） |
| proroot | — | — | ❌ 未找到（GitHub 搜索无对应项目） |
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
| panvk-g99-jm | `Noysz/panvk-g99-jm` ✅ | 25 | **Valhall v9 + Job Manager**（Helio G99 = Mali-G57 MC2），和本机 G77 同为 v9 JM——**同代最贴近的实现** |
| mesa-panvk-g57（Termux + X11） | `apexspan-svg/mesa-panvk-mali-g57` ✅ | 3 | kbase + **X11** 在 Termux 里跑 panvk：WSI/窗口侧的做法可直接借 |
| mesa-gs101（G78/G77） | `m8l8th814n-eng/mesa-gs101` ✅ | 3 | 明确写了支持 **Mali G77** 的 panvk |
| panvk-JM-kirin | `MineGraph-bilibili/panvk-JM-kirin` ✅ | 1 | JM 后端、kbase UAPI 11.x（与本机 11.38 同档） |
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
| Piperella | `Piperella/Piperella-Wayland-Compositor` ✅ | 6 | Wayland 合成器做成 **Gradle 库**直接嵌 Android App |
| Anland | `SuperTurtleDev/anland` ✅ / `lfdevs/anland-termux` ✅ | 361 / 165 | **Android 上的 Wayland 协议实现**，Termux/PRoot/Chroot/LXC 都有现成接法；`Droidspaces-Mali` 的桌面就是靠它显示 |
| DroidDesk | `orailnoor/DroidDesk` ✅ | 4872 | Termux + X11 + TUR 的成品桌面（产品化参考） |
| MagicDesk | `mekhontsev/magicdesk` ✅ | 241 | 原生多窗口工作站（X11/Wayland + Termux + MCP），同作者 Shroot |
| Linuxify | `Bilal140202/linuxify` ✅ | 2 | Kotlin/Compose 壳 + proot 的产品化界面 |
| usix-termux | — | — | ❌ 未找到 |
| Mutter / GNOME Shell / wlroots / Weston / KWin / Xwayland | 上游 | 🔎 | 桌面本体，我们只负责载体，不改它们 |

## 6. Android 原生显示与缓冲

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| libhybris | `libhybris/libhybris` ✅ | 866 | **glibc 进程里用 bionic 的 HAL/EGL**：`Droidspaces-Mali` 就是靠它跑 Mali。与我们的「kbase 直连」并列的第二方案 |
| android-vulkan-bridge | `afeimod/android-vulkan-bridge` ✅(镜像 2★) | | LXC 里用 libhybris 接 Android Vulkan |
| AOSP / NDK / libdrm / libgbm / libwayland / libseat / libinput / libxkbcommon | 上游 | 🔎 | 接口与语义权威来源 |

## 7. 视频 / 编解码

> 2026-11-23 追加：**这条线已经有完整开源实现（termux-va）**，结论写在 [`prior-art-video.md`](prior-art-video.md)：合成 CSD 是标准做法（我们做对了）、流水线深度 6 是流畅的前提、gst-vaapi 一帧一结所以只能 18 fps、**Chromium/Firefox 硬解在非骁龙（含 MTK）上是已知不可用**。

| 清单名 | 真实仓库 | ★ | 借鉴点 |
|---|---|---|---|
| GStreamer | `GStreamer/gstreamer` 🔎 | ~2k | 媒体管线；我们要在 guest 内提供 VA-API 后端给它 |
| FFmpeg / libav | `FFmpeg/FFmpeg` 🔎 | ~50k | 解复用/同步留在 guest |
| Android NDK Media | `aosp-mirror/platform_frameworks_av` 🔎 | — | AMediaCodec 的 ABI 权威来源 |
| dav1d / libvpx / libyuv / libjpeg-turbo | 上游 | 🔎 | 软解兜底 |
| termux-va | `lfdevs/termux-va` ✅ | 20 | **最直接的对照**：daemon(MediaCodec) + Mesa VA 桥，AVC/HEVC/VP9/AV1 解码 + AVC/HEVC/VP9 编码，免 root、支持 proot。也用**合成 CSD**；HEVC 合成实现是蓝本。见 `prior-art-video.md` |
| droidspaces-media-decode | `Re-s/droidspaces-media-decode` ✅ | | termux-va 的上游（协议 DMD v3 的来源） |
| mesa-for-android-container (dev 分支) | `lfdevs/mesa-for-android-container` `dev/adreno-main` ✅ | 392 | `src/gallium/frontends/va/tva_*` 就是这套 VA 桥；也是 2.0 里那个 mesa tar 的来源 |
| droidmedia | `sailfishos/droidmedia` ✅ | 19 | glibc 上包 MediaCodec / **Camera** / AudioRecorder 的样板（Droidian 有 port），相机/录音的接口形状参考 |

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
| DroidDeck | `Droid-Deck/DroidDeck` ✅ | 21306 | SteamOS 体验搬到 Android（gamescope + proot），是目前最成熟的一批 |
| pr / DroidVM / Termux Packages / Andronix / AnLinux | 未逐个核 | — | Termux 本身（`termux/termux-app`）是生命周期/服务的成熟样本 |

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

- 2026-11-22 第一遍：逐条核对真实路径（`gh api` + 搜索）。当时只对上 **41** 项，未对上项集中在 proot 替代类。
- 2026-11-22 第二遍（走代理补查）：`Shroot→mekhontsev/shroot`、`zproot→zproot/zproot`、
  `Piperella/Piperella-Wayland-Compositor`、`Anland→SuperTurtleDev/anland`、`MagicDesk→mekhontsev/magicdesk`、
  `DroidDesk→orailnoor/DroidDesk`、`DroidDeck→Droid-Deck/DroidDeck`；
  并补到三份**同代 Mali 实现**：`Noysz/panvk-g99-jm`（v9 JM）、`apexspan-svg/mesa-panvk-mali-g57`（kbase+X11）、
  `m8l8th814n-eng/mesa-gs101`（写明 G77）。
- 当时仍 ❌ 未找到：`proroot`、`elf_loader`（同名仓库无关）、`usix-termux`、`mesa-tu8` 原名。
- 2026-11-23 第四遍（最终逐条矩阵）：`proroot→coderredlab/proroot`、`mesa-tu8→Droid-VM/turnip`、
  `Droid-Deck Drivers→Droid-Deck/Drivers`、`Adreno Tools→K11MCH1/AdrenoToolsDrivers` 均已定位。
- **已真读过源码的**（不只是看 README）：`manoedinata/mesa-kbase-j4lte`（libdrm-kbase 1785 行 C + Mesa 补丁）、
  `BkaNeko/panvk-mtk-driver`（903 行 MTK panvk 补丁）、`zenithblue-oss/panvk-kbase-android`（README + 目录树）。

## 拉取规则（怎么"拉下来"）

```sh
# 只读参考，不进本仓库（避免 1GB 级仓库污染 git）
mkdir -p /tmp/refs && cd /tmp/refs
gh api repos/<owner>/<repo>/tarball/HEAD | tar xz --strip-components=1 -C <owner>_<repo>
# 记录审计用的 commit
gh api repos/<owner>/<repo> --jq '.full_name+" @"+.default_branch+" "+(.pushed_at)'
```

注意：`zenithblue-oss/panvk-kbase-android` 仓库体积 ~1.09 GB（含预编译二进制），拉取前先看 `size`。

---

## 原始清单归档

用户 2026-11-22 提供的第三方清单已逐字保存在
[`reference-request-20261122.md`](reference-request-20261122.md)（12 类、120 个条目，含重复项）。
本文件中的表格是对它做的去重、真实路径核对和可用性判断，不是对原清单的替代。
原始清单是**未审计资料**，其中的项目名和描述不能直接当事实。

## 已拉取到本机的工作副本（2026-11-23）

只拉当前会用到的仓库；放在 `/tmp/refs2`，不进 Husk git。记录 commit 是为了可复现：

| 仓库 | 本地目录 | commit | 当前用途 |
|---|---|---|---|
| `faux123/panvk-faux123-driver-rp4pro` | `/tmp/refs2/faux123_panvk-faux123-driver-rp4pro` | `cb72a0ba2ec7f160b818917be0c826bf4d57750b` | Mali-G77 / r32p1 / Android 13 实机先例；PanVK kbase + Zink 的最强旁证 |
| `BkaNeko/panvk-mtk-driver` | `/tmp/refs2/BkaNeko_panvk-mtk-driver` | `594efbf6d2038df81ce0a75603cce72f4c866879` | MTK 的 `MEM_ALLOC_EX` 回退、AFBC、CSF 调度补丁 |
| `manoedinata/mesa-kbase-j4lte` | `/tmp/refs2/manoedinata_mesa-kbase-j4lte` | `3fd6fc3f0271d2538f93bc5a51a19cedf267b8dd` | libdrm-kbase 1785 行规格；panfrost 语义与 gralloc 参考 |
| `FristOneRR/FristOneRR-Panvk-Driver` | `/tmp/refs2/FristOneRR_FristOneRR-Panvk-Driver` | `e62405dbe3c9bea626a4afe534746180cea06586` | JM / Valhall v9 提交模型旁证 |
| `zenithblue-oss/panvk-kbase-android` | `/tmp/refs2/zenithblue-oss_panvk-kbase-android` | `cf70e02b918e1864cbcd746a7fefa57b96c673ce` | 通用 Mali kbase PanVK（v6–v14）、profile、构建与验证；仓库约 1.09 GB，使用 sparse checkout |
| `funnymdzz/mesa` tag `kbase-v0.1.2` | `/tmp/mesa-kb012` | `4e5323ef35dc580a6140f8d433a6d13ae3724c07` | Mesa 26.3-devel + `src/panfrost/lib/kmod/kbase_kmod.c`；已实测 `drmSyncobjCreate()` 失败，保留为对照 |

拉取规则不变：仅浅克隆/稀疏检出；记录 URL 和 commit；不把第三方源码提交进 Husk。

## 第三遍审计（2026-11-23，补上遗漏与最新状态）

本轮把原始清单重新对齐到真实仓库，重点补上昨天清单里没有定位到的项目。状态仍按证据分级，
不把“搜到同名仓库”当成“就是这个项目”。

### 0.5.1 新确认：proroot 不是空名字

- **FACT**：`coderredlab/proroot` 真实存在，默认分支 `main`，审计时 commit
  `c01039ab017448263a4ddbd9221ecb88e737553c`，98★（2026-11-23 查询）。
- **FACT**：README 自述 “Rootless Linux runtime for Android. Drop-in replacement for proot with zero ptrace overhead.”
  并要求 Android 8.0+ / arm64-v8a / glibc rootfs；发布物是 5 个 arm64 `.so`
  （`libproroot.so`、`libproroot-runtime.so`、`libproroot-linker.so`、
  `libproroot-bridge.so`、`libproroot-stub-loader.so`）。
- **FACT**：仓库当前只有二进制和 README，**源码未公开**；LICENSE 为
  `NOASSERTION`，README 明确写 proprietary、禁止分发修改后的二进制。
- **JUDGMENT**：可作为“零 ptrace 原生 runtime 已经有人做成产品”的强旁证，
  也可对照它的 loader/runtime/stub 分层；**不能把它的二进制当依赖或直接抄代码**。
- **FACT**：`DSH-APP/DSHA`（3651★）在 README 中把 proroot 作为 Ubuntu 环境运行时，
  说明它已经有真实产品集成案例；该消费者仓库本身不作为运行时依赖。

### 0.5.2 新确认：FristOneRR 是当前最值得实测的 G77 源码路线

- **FACT**：源码仓库 `FristOneRR-Admin/FristOneRR-Panvk-Source`，当前审计 commit
  `0c9dce35094dbe89c412b79bf57c6067c7b97e59`，Mesa `26.3.0-devel`。
- **FACT**：源码 `src/panfrost/model/pan_model.c:88` 有
  `VALHALL_MODEL(PAN_PROD_ID(9, 0, 0), 0, "G77", ...)`；
  本机 GPU_ID `0x90800011` 正好解码为 `PAN_PROD_ID(9,0,0)`。
- **FACT**：源码 `src/panfrost/vulkan/panvk_physical_device.c` 的 kbase 路径使用
  `kbase_cpu_sync_type`，明确注释 “DRM syncobj is not available on a kbase fd”；
  这解决了另一种 G77 fork 在 `drmSyncobjCreate()` 处失败的问题。
- **FACT**：源码还有 `PANVK_ATOM_STRIDE` 自动探测和 JM kbase 提交路径；
  但 `panvk_bind_queue.c` 仍有 sparse-bind 专用的 `drmSyncobjCreate()`，
  因此“设备创建成功”与“所有 bind 路径成功”必须分开验收。
- **FACT**：其发布包是 Android/bionic 构建（DT_NEEDED 含 `libhardware.so`、`liblog.so`、
  `libnativewindow.so`、`libsync.so`），不能直接塞进 glibc guest；
  本轮必须用源码交叉编出 **glibc 版**。
- **FACT**：本轮已按上述源码编出 glibc 版，产物 sha256
  `fbfa3b43e69d3fd7f4088fe6b9e94a621e19ca3f95502265281f92ba908b6147`；
  guest 内实测 `vulkaninfo --summary` 与纯 Rust probe 都能报出 Mali-G77 MC9，并完成 logical
  device / queue / host-visible buffer map/readback（evidence 46）。
- **FACT**：初版在该进程里 `MEM_EXEC_INIT` 返回 EPERM；按 `jm-v9/004` 把 EXEC_VA 移到 JIT 前
  并设为 4G 后，警告消失。修复后产物 sha256
  `ead96201dc84b653f9b643db00e2af239b763c3ecf61f5cf2caa6f44c96de5ec`。
- **FACT**：纯 Rust probe 已提交 compute job，`vkQueueSubmit` + fence 后读回 GPU 写入
  `0x4855534b`；并完成真实顶点缓冲 + vertex/fragment shader + `vkCmdDraw` 的离屏 graphics
  管线，回读红色像素 `0xff0000ff`。仍没有 swapchain / WSI / renderer，不能写成完整 GPU 完成。
- **FACT**：FristOneRR 文档警告 kbase 在容器/chroot/proot 中可能触发厂商内核 bug，
  某些设备需要内核侧修复；因此在 RMX3357 上只先做 `vulkaninfo --summary` 级别的
  device-create 验证，跑通前不盲目提交 job。

### 0.5.3 funnymdzz fork 的实测结论（保留为失败证据）

- **FACT**：`funnymdzz/mesa` tag `kbase-v0.1.2`，commit
  `4e5323ef35dc580a6140f8d433a6d13ae3724c07`，在 RMX3357 上能打开 `/dev/mali0`、
  识别 GPU，但 `vkCreateDevice` 报 `VK_ERROR_OUT_OF_HOST_MEMORY`；
  根因不是 OOM，而是 `drmSyncobjCreate(device->drm_fd, ...)` 在 kbase fd 上失败。
- **JUDGMENT**：该 fork 不能作为本机唯一路线；保留其源码和产物作为对照，
  当前优先验证 FristOneRR 的 CPU-backed sync 路径。

### 0.5.4 其余清单的审计口径

- 原始清单 120 条全部逐条落盘，**保留重复项**：69 条 GitHub 仓库、45 条官方上游/组件、6 条无法唯一确认。
- 无法唯一确认的 6 条：`elf_loader`、`mesa-patches`、`Local Desktop`、`ARM Mali GPU Tools`、
  `usix-termux`、`pr`。同名或近似项目不作为原项目，避免伪造链接。
- 已确认但实用性受限：`proroot` 是闭源二进制；`mesa-tu8` 与 `libtar` 已归档。
- `Adreno Tools` 已在第四遍确认到 `K11MCH1/AdrenoToolsDrivers`；不再保持 UNKNOWN。
- 高优先级顺序不变：`proroot`（架构旁证，bin-only）→ `FristOneRR` / `faux123` / `zenithblue`
  （Mali kbase 源码）→ `mesa` 上游 → `libhybris` / Zink（容器 GPU 对照）→ 其余桌面/媒体/系统参考。
