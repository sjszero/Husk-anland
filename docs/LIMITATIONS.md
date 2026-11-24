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
   `gl-probe es3`（`tools/device-tests/es3-workload.sh` 一条命令重跑）。
   `MESA_GLES_VERSION_OVERRIDE=3.2` 现在由安装器写进 `/etc/profile.d/husk-gpu.sh`（evidence 167），
   **进发布配置**，桌面合成路径（mutter/cogl，`cogl=gles2 override=3.2`）已经在 3.2 下出帧
   （evidence 165/168）。还没验的是**普通应用**：GTK4 的 ngl 渲染器这类真实客户端没跑过，
   遇到花屏/黑窗口先怀疑这里，回退开关是 `HUSK_GPU=llvmpipe`。
   另外这棵树的整块 `glReadPixels` 只写第一行+第一列（evidence 162），
   截图类工具会拿到全零而不报错——那是读回缺陷，别当成渲染失败。
2. **窗口呈现能跑，但不是零拷贝**（evidence 165/167）：X11 窗口化 EGL 已经通了
   （重编 PanVK 打开 `-Dplatforms=x11` 后 `VK_KHR_xcb_surface` 才进表），`gl-probe window`
   `stage 7→10`、60 次 swap 全过。但客户端的帧是**经 SHM 拷贝**进 X server 的：
   `DRI3 error: Could not get DRI3 device` 仍在，dmabuf 直传没接上。
   再往后 X server 的合成结果仍走 `AHardwareBuffer` → dma-buf → `SurfaceControl`
   （evidence 59，那条是真零拷贝）。所以整条链是
   **GPU 渲染 → SHM 拷贝 → X 合成 → AHB 零拷贝 → 屏幕**，中间那一次拷贝没有消掉。
3. **桌面合成器已经由 GPU 出帧（FACT），但 fps 在轻负载上不占优**：gnome-shell 46 跑在
   zink → PanVK 上，X 根窗口抓到完整桌面、非黑像素 100%（evidence 165/168）。而 640×480
   clear+swap 的帧率是 zink 131 fps vs llvmpipe 300 fps —— 这个负载的瓶颈就是上面那次
   SHM 拷贝，不是 GPU 算力（evidence 168）。重负载的 GPU/CPU 对比**未测**。

## 8 · 分区很细：这一版不是「什么都跑得起来」

| 能力 | 状态 | 证据 |
|---|---|---|
| 命令行用户态（bash、coreutils、apt、dpkg、gcc、make、python3+numpy、git） | 可用 | evidence 154、155 |
| 会话服务（polkit、at-spi、gvfs、RandR） | 可用 | evidence 151、152 |
| GPU：Vulkan compute / 离屏 graphics | 可用 | evidence 156 |
| GPU：GL/EGL 经 Zink 落到 GPU（默认上限 GLES 3.2，靠 override；真实应用未验） | 可用 | evidence 156、161、167 |
| GPU：X11 窗口呈现（EGL window surface + swap） | 可用，呈现走 SHM 拷贝（DRI3 未通） | evidence 165 |
| GPU：桌面合成器（gnome-shell 46 on zink/PanVK） | 可用，出帧；轻负载 fps 不如 llvmpipe | evidence 165、168 |
| 硬件视频解码（平台 MediaCodec，应用侧） | 有实测链路，未产品化 | evidence 80 |
| guest 内 VA-API 全链（播放器直接用） | 部分 | evidence 81–87 |
| 相机（V4L2 前端 + 平台编码） | 有实测链路 | evidence 91–93 |
| 音频（PulseAudio → AAudio） | 可用，10 秒级，非长稳 | evidence 62 |
| 传感器、GPS、USB、蓝牙、NFC、指纹 | 未开始 | — |
| 浏览器 | 随附 rootfs 无浏览器包，未跑过 | — |

「有实测链路」不等于「产品可用」：这四类都还是「跑通过一次」的程度，稳定性与错误路径没做完。
