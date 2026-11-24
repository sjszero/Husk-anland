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
报错是一句误导性的 `/bin/sh: 1: Cannot fork`（evidence 154）。修法：`make libc-patch`（`nr-tools libc-patch`，Rust，跑完会断言：slot 表逐个对上、
`clone3` 站点归零、`set_robust_list` 被改写、4K/16K/64K 页都对齐），或直接重装 APK。
会话启动前 `husk-session.sh` 还会比一次哈希：被动过就换回随包那份，换不回来就拒绝启动。（evidence 170）

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
   **2026-11-25 已补真实客户端**：GTK 4.14 的 `gnome-text-editor`（`GSK_RENDERER=ngl`）
   在 zink/PanVK 的 X11 上开窗、画出完整界面，X 根窗口抓图 28 565 种颜色、不是黑窗口
   （evidence 176）；`gnome-control-center`（GTK 4 + libadwaita）的侧栏与状态页也完整出画。
   **反例是文件管理器**：`nautilus` 46 能开窗、能枚举目录（日志逐条打出 `/etc/*`），但内容区
   整块空白、连列头都不出现；`GSK_RENDERER=cairo` 与 `ngl` 一样、`list-view` 与 `icon-view`
   一样，**同一份 rootfs 换 proot 起也照样空白**，同一会话里最小 `GtkColumnView` 程序却正常
   ——所以不是 path-shim 造成的，根因 UNKNOWN（evidence 178）。WebKit/Electron 类**未测**。
   键盘输入**已测**：XTEST 敲进 `gnome-text-editor`，标题与正文都跟着更新、折行正常
   （evidence 180）；**没量延迟**，也没测 IME/组合键，指针注入另见 evidence 148。
   另外这棵树的整块 `glReadPixels` 是坏的：目的行距恒为 **16 字节**（与宽度无关），
   每行只有前 4 个像素落在该去的地方，写入量 = `H×16 + (W×4 − 16)` 字节
   （evidence 179 把地址学量出来了，evidence 162 是最初的现象）。截图类工具会拿到错位画面
   而 `glGetError()` 仍是 0。**逐行读（`W×1` 调 H 次）与逐像素读实测 100% 正确**，
   单列读（`1×H`）同样坏、proot 下还会写到缓冲区外。llvmpipe、zink、proot 三处完全一致，
   所以不在 path-shim、也不在 Mali/PanVK 后端，落在这版 Mesa 的公共读回层（具体文件 UNKNOWN）。
2. **窗口呈现能跑，但不是零拷贝**（evidence 165/167）：X11 窗口化 EGL 已经通了
   （重编 PanVK 打开 `-Dplatforms=x11` 后 `VK_KHR_xcb_surface` 才进表），`gl-probe window`
   `stage 7→10`、60 次 swap 全过。但客户端的帧是**经 SHM 拷贝**进 X server 的：
   `DRI3 error: Could not get DRI3 device` 仍在，dmabuf 直传没接上。
   再往后 X server 的合成结果仍走 `AHardwareBuffer` → dma-buf → `SurfaceControl`
   （evidence 59，那条是真零拷贝）。所以整条链是
   **GPU 渲染 → SHM 拷贝 → X 合成 → AHB 零拷贝 → 屏幕**，中间那一次拷贝没有消掉。
3. **桌面合成器已经由 GPU 出帧（FACT），但 fps 在轻负载上不占优**：gnome-shell 46 跑在
   zink → PanVK 上，X 根窗口抓到完整桌面、非黑像素 100%（evidence 165/168）。而 640×480
   clear+swap 的帧率是 zink 131 fps vs llvmpipe 300 fps。**不是 GPU 算力不够**：2026-11-25
   把窗口 swap 拆成「渲染」和「呈现」两段量了一遍——GPU 渲染在 160×120 到 1080×1080 之间
   都只要 1.2–1.7 ms，而呈现这一段按窗口面积线性涨：zink 约 **24.8 ms/兆像素**（≈161 MB/s
   的有效带宽），llvmpipe 约 11 ms/兆像素（evidence 174）。即 640×480 那 8.9 ms 里 GPU 只
   占 1.3 ms，其余是 GPU→CPU 回读，再加挂进 X server 的那次拷贝。重负载的 GPU/CPU 对比
   **未测**。
   要消掉这一段只能走 DRI3/dma-buf 直传，而它在这台机上**不在 shim 能力范围内**：X server 是
   应用进程里的 bionic Xorg（X.Org 21.1.99），`untrusted_app` 打不开 `/dev/dri`、
   `/dev/dma_heap` 不存在、`/dev/ion` EACCES，所以「DRI3 有扩展但没有设备」，只能停在
   `Could not get DRI3 device`。要么 shell 侧给服务器一条能 import dma-buf 的路，要么接受
   这次拷贝（已实测代价见 evidence 174）。

## 8 · 分区很细：这一版不是「什么都跑得起来」

| 能力 | 状态 | 证据 |
|---|---|---|
| 命令行用户态（bash、coreutils、apt、dpkg、gcc、make、python3+numpy、git） | 可用 | evidence 154、155 |
| 会话服务（polkit、at-spi、gvfs、RandR） | 可用 | evidence 151、152 |
| 文件管理器（Nautilus 46） | 能开窗、能枚举；**内容区空白**（cairo/ngl、list/icon 都一样，proot 同样空白） | evidence 178 |
| GPU：Vulkan compute / 离屏 graphics | 可用 | evidence 156 |
| GPU：GL/EGL 经 Zink 落到 GPU（默认上限 GLES 3.2，靠 override；真实应用未验） | 可用 | evidence 156、161、167 |
| GPU：X11 窗口呈现（EGL window surface + swap） | 可用，呈现走回读+拷贝（DRI3 未通，代价 24.8 ms/MP） | evidence 165、174 |
| GL 整块读回（`glReadPixels` W×H） | **坏**：目的行距恒 16 字节；逐行/逐像素读正确 | evidence 179 |
| GPU：桌面合成器（gnome-shell 46 on zink/PanVK） | 可用，出帧；轻负载 fps 不如 llvmpipe | evidence 165、168 |
| 硬件视频解码（平台 MediaCodec，应用侧） | 有实测链路，未产品化 | evidence 80 |
| guest 内 VA-API 全链（播放器直接用） | 部分 | evidence 81–87 |
| 相机（V4L2 前端 + 平台编码） | 有实测链路 | evidence 91–93 |
| 音频（PulseAudio → AAudio） | 可用，10 秒级，非长稳 | evidence 62 |
| 传感器、GPS、USB、蓝牙、NFC、指纹 | 未开始（按定位不做：这类节点由机主自己穿透） | — |
| 输入注入（触摸/键盘） | 能到应用（evidence 148）；指针默认是 trackpad 语义，2.0 侧的 `touchMode` 默认值未在本仓库改 | evidence 148 |
| 浏览器 | 随附 rootfs 无浏览器包，未跑过 | — |

「有实测链路」不等于「产品可用」：这四类都还是「跑通过一次」的程度，稳定性与错误路径没做完。

## 9 · 路径改写的已知边界（四条已证、一条未证）

libc 里有 5 个函数打不上入口补丁，钉在 `tools/loader/libc-interpose.expected`：
`__readlink_chk`、`__readlinkat_chk`、`__realpath_chk`、`__mq_open_2`、`__openat64_nocancel`。
2026-11-25 用同一份 stock libc 反汇编核过（evidence 175）：前四个是 24 字节的 fortify 包装，
唯一「跳进补丁窗口」的是它们自己的 `b.hi`/`tbnz` 跳到 +0xc，真正干活是尾调用
`readlink` / `readlinkat` / `realpath` / `mq_open`——这四个名字都在表里、入口已被接管，
所以补包装函数没有收益，不构成路径逃逸。第五个在这个 libc 构建里**没有导出符号**，
slot 页定位不到，影响标 **UNKNOWN**：它的调用者是 libc 内部、外层入口（`open`/`open64`/
`openat`/`openat64`）都已接管，但这是论证，不是实测。
