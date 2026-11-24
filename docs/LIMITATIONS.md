# 限制

这份文件写的是 Husk 做不到、或者还没做到的事。每条都来自目标机上的实测，或者明确标成
未验证。**没写在这里不等于能用**——那只是还没研究。

参考环境（实测条件）：arm64 Android 13、kernel 4.19.191、4 KiB 页，以 `untrusted_app`
运行，无 root。机型与 SoC 只是那一次实测的现场，不是运行时的要求。

## 1 · 一个 ABI、一个设备族（FACT）

只跑过 arm64 Android、4 KiB 页、glibc 2.39 的发行版。loader 的补丁是版本断言的
（`nr-tools scan-syscalls` 校验 glibc 版本与 `set_robust_list` / `rseq` / `clone3` 的
site 数量），所以换发行版或换 glibc 构建会**直接报错**，而不是半好不坏地跑起来。
16 KiB / 64 KiB 页只在 QEMU 上验证过（evidence 125），没在手机上验证。

## 2 · 没有 root、没有解 Bootloader、没有虚拟化（设计如此，永久）

Husk 跑在普通应用沙箱里。沙箱碰不到的东西就碰不到：内核模块、别的应用的数据、`hidepid`
隐藏的 `/proc` 项、裸块设备、系统级策略开关。

## 3 · Termux 不是交付形态（FACT，已实测）

用户态需要的二进制与库 Termux 的 `$PREFIX` 不提供，在那边跑会直接卡在缺依赖上。支持的
目标目录是能执行且可写的那种（设备上的 `/data/local/tmp` 与应用私有目录都满足，见
`docs/RUNNING.md`）。这不是脚本能补的缺口。

## 4 · `apt upgrade` 升 libc6 会拆掉运行时（FACT，已给修法）

`apt` 升级 `libc6` 会把打过补丁的 libc 换成 stock 版本，丢掉两处补丁：libc 内部调用的 slot
页、以及 `clone3 → ENOSYS`。之后 `getent shadow` 返回空、提权修复脚本失败、会话起不来，
报错是一句误导性的 `/bin/sh: 1: Cannot fork`（evidence 154）。重跑
`tools/guest-config/install-libc-patch.sh` 即可。

## 5 · rootfs 里不能有绝对符号链接（FACT，已给工具）

内核按 `/` 解析符号链接目标，所以 rootfs 里的绝对链接会逃到 Android 文件系统。Ubuntu
24.04 的树里有 552 个，`libblas.so.3` 之类经 `alternatives` 的链接全部受影响。
`husk-symlinks --root <rootfs> --apply` 改写成相对路径；这是 rootfs 准备步骤，
不是一次性修补（`docs/RUNNING.md`）。

## 6 · 取样走 `adb shell`，它的 stdout 是 socket（FACT）

guest 里 `/proc/self/fd/1` 是 socket。`make` 往它写完（309 字节就够）之后，同一会话后续
写入全部 `EACCES`。proot 下完全一样，所以不是 shim 的行为；但它限定了基准的取样方式：
构建类步骤统一重定向到 `/dev/null`，只记墙钟（evidence 155）。

## 7 · GPU：能算、GL 也能落到 GPU，但桌面还吃不到

**已经是 FACT（evidence 156，2026-11-24，当前版本的树）**：guest 进程直开 `/dev/mali0`；
kbase 后端的 PanVK（FristOneRR，Mesa 26.3）枚举出 `Mali-G77 MC9`，compute 与离屏 graphics
都能跑并回读 GPU 写入的值；开 Zink 后 EGL/GL 的 renderer 是
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，不开就是 `llvmpipe`。

**还差的三件事**：

1. **API 上限默认只有 GLES 2.0 / GL 2.1**。这是 Mesa 的 zink 报出来的上限，**不是**设备能力
   （同一棵树里 `GALLIUM_DRIVER=llvmpipe` 报 ES 3.2 / GL 4.6，evidence 160）。设
   `MESA_GLES_VERSION_OVERRIDE=3.2` 后，一个真 ES3 workload（VAO + 实例化 divisor +
   std140 UBO + 纹理采样 + MRT 两附件）在 zink/PanVK 上**逐像素画对**（evidence 161，
   `gl-probe es3`（`tools/device-tests/es3-workload.sh` 一条命令重跑）。还没验的是**真实应用**：
   GTK4 的 ngl 渲染器、mutter 的合成路径都还没在 ES3 下出过帧，所以 override 仍**不进发布
   配置**、只作实验开关。另外这棵树的整块 `glReadPixels` 只写第一行+第一列（evidence 162），
   截图类工具会拿到全零而不报错——那是读回缺陷，别当成渲染失败。
2. **WSI / swapchain 没做**：把窗口表面交给 GPU 直接呈现这条链还没接。呈现仍走
   `AHardwareBuffer` → dma-buf → `SurfaceControl`（evidence 59），GPU 直接把画面交给
   SurfaceFlinger 的那一步不存在。
3. **桌面合成器仍以软件渲染验收**：`docs/BENCHMARKS.md` 里图形那一维因此不进表——不是没测，
   是没有两个可比对象。

## 8 · 分区很细：这一版不是「什么都跑得起来」

| 能力 | 状态 | 证据 |
|---|---|---|
| 命令行用户态（bash、coreutils、apt、dpkg、gcc、make、python3+numpy、git） | 可用 | evidence 154、155 |
| 会话服务（polkit、at-spi、gvfs、RandR） | 可用 | evidence 151、152 |
| GPU：Vulkan compute / 离屏 graphics | 可用 | evidence 156 |
| GPU：GL/EGL 经 Zink 落到 GPU（GLES 2.0 上限） | 可用 | evidence 156 |
| GPU：窗口呈现（WSI）、桌面硬件加速 | 未做 | 本节 §7 |
| 硬件视频解码（平台 MediaCodec，应用侧） | 有实测链路，未产品化 | evidence 80 |
| guest 内 VA-API 全链（播放器直接用） | 部分 | evidence 81–87 |
| 相机（V4L2 前端 + 平台编码） | 有实测链路 | evidence 91–93 |
| 音频（PulseAudio → AAudio） | 可用，10 秒级，非长稳 | evidence 62 |
| 传感器、GPS、USB、蓝牙、NFC、指纹 | 未开始 | — |
| 浏览器 | 随附 rootfs 无浏览器包，未跑过 | — |

「有实测链路」不等于「产品可用」：这四类都还是「跑通过一次」的程度，稳定性与错误路径没做完。
