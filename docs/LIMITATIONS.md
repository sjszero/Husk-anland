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
   另外每一轮 PanVK 起上下文都会打 `kbase: KBASE_IOCTL_MEM_EXEC_INIT failed: EPERM`
   （FACT，evidence 176）。**INFERENCE**：它挡的是「CPU 可执行映射」那条路，GPU 侧着色器执行
   不受影响——同一棵树里 compute 与离屏图形回读的值都是对的（evidence 156、161）。
   内核为什么拒这一条（`untrusted_app` 策略、还是 kbase 的 flag 组合）：**UNKNOWN**。
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
   应用进程里的 bionic Xorg（X.Org 21.1.99）。2026-11-25 在应用域（`u:r:runas_app`）里把节点
   布局查清了：`/dev/dri` 下**只有 `card0`（226,0，`system:system` `0660`）**，**没有
   `renderD128`**；`/dev/dma_heap` 不存在；GPU 实际走厂商节点 `/dev/mali0`（`10,33`，
   `0666`，所以 PanVK 能直连）。也就是说这台机不是「权限不够」，而是**根本没有 DRI3 要的那个
   render 节点**。所以「DRI3 有扩展但没有设备」，只能停在
   `Could not get DRI3 device`。要么 shell 侧给服务器一条能 import dma-buf 的路，要么接受
   这次拷贝（已实测代价见 evidence 174）。

### 7.1 这三条路各自落在哪一层（能不能只改 Linux 运行层）

先说结论：**三条里没有一条能只靠 Linux 运行层（guest rootfs / shim / loader / Mesa）改完**，
因为它们要动的对象是 X server，而这一版的 X server 是**应用进程里的 bionic Xorg**，不归
shim 管。

| 路线 | 要改什么 | 落在哪一层 | 现在能不能做 |
|---|---|---|---|
| DRI3：server 自己拿 render 节点 + import dma-buf | fake `renderD128` 要支持 prime 导出/导入；server 还要拿到 `mali0` 的 fd | guest shim **+** Android 侧（server 在应用里） | 本机没有 render 节点；fake 节点只做到 alloc/mmap/读写（`gpu-drm-shim.md` M1），prime 没做。**不是纯运行层** |
| server 直接把像素写进应用给的共享段，应用侧再导入 GPU | server 侧新增一条呈现路径 + 应用侧 import | 主要是 Android 侧 | 没做。**不是纯运行层** |
| 合成器绕过 X，直接把 buffer 交给 `SurfaceControl` | 换掉呈现层 | Android 侧 | 已否决（不换合成器、保持 X11） |

**能在运行层单独做的，只有「把这一次拷贝的单价压低」，消不掉它**（都还没测，写在这里当待办）：

- 让 guest 客户端走 MIT-SHM / shared pixmap，而不是 `XPutImage` 请求流。现在量的 17.66 ms/帧
  是**请求流的上界**，不是 SHM 的真实代价——改的是 guest 里的库与驱动路径，属运行层。
- 让 zink 的 present 路径少一次 tiled→linear 转换（Mesa 编译选项 / WSI 参数，属运行层），
  能压「回读」那一格，压不到零。
- 唯一能把上面三条变成「运行层可做」的架构前提：**把 X server 也从 bionic 搬进 guest**
  （glibc Xorg + husk loader）。server 归 shim 管之后，fake render 节点才有对象可对。
  代价明确：上屏呈现要重做、mutter/zink 组合要重验，工作量按周计。
   **这次拷贝是架构共性，不是这台机慢**（分开标）：来源是「X server 跑在 Android 应用进程里、
   拿不到 render node」。**FACT**：本机 `untrusted_app` 打开 `/dev/dri/renderD128` 直接
   `EPERM`，`card0` 是 shim 合成的 `10,33`；**FACT**：guest 内纯 `memmove` 实测 10.05 GiB/s
   （10 MiB 一次 1.043 ms），2400×1080 一帧 10.37 MB 值拷贝下限约 1.0 ms，而实测回读是
   30.27 ms/帧，带宽只能解释 3.4%；**INFERENCE**：凡是不 root、把 X server 关在应用进程里的
   方案（Termux:X11、Xvfb 系）都受同一条 `untrusted_app` 约束，区别只在各自有没有把 DRI3
   /dma-buf 直传打通；别人在各自实机上到底省没省掉这一次，**我们没量，UNKNOWN**。

## 8 · 分区很细：这一版不是「什么都跑得起来」

接法只有一根轴：**直连**＝guest 进程自己开设备节点，应用侧一行不参与；**中转**＝一条明确的
socket 协议，数据在应用侧过一次。逐类理由见
[`docs/design/hardware-interface-map.md`](design/hardware-interface-map.md)。

| 能力 | 接法 | 状态 | 证据 |
|---|---|---|---|
| 命令行用户态（bash、coreutils、apt、dpkg、gcc、make、python3+numpy、git） | — | 可用 | evidence 154、155 |
| 会话服务（polkit、at-spi、gvfs、RandR） | — | 可用 | evidence 151、152 |
| 文件管理器（Nautilus 46） | — | 能开窗、能枚举；**内容区空白**（cairo/ngl、list/icon 都一样，proot 同样空白） | evidence 178 |
| GPU：Vulkan compute / 离屏 graphics（`/dev/mali0` + kbase） | 直连 | 可用 | evidence 156 |
| GPU：GL/EGL 经 Zink 落到 GPU（默认上限 GLES 3.2，靠 override；真实应用未验） | 直连 | 可用；默认值由加载器注入，非登录执行也吃得到 | evidence 156、161、167、182 |
| GPU：X11 窗口呈现（EGL window surface + swap） | 直连（呈现这一跳不是） | 可用，呈现走回读+拷贝（DRI3 未通，代价 24.8 ms/MP） | evidence 165、174 |
| GL 整块读回（`glReadPixels` W×H） | — | **坏**：目的行距恒 16 字节；逐行/逐像素读正确 | evidence 179 |
| GPU：桌面合成器（gnome-shell 46 on zink/PanVK） | 直连 | 可用，出帧；轻负载 fps 不如 llvmpipe | evidence 165、168 |
| 显示上屏（dma-buf → AHB → `SurfaceControl`） | 应用侧，零拷贝 | 可用（10 000 帧 ring） | evidence 59 |
| 硬件视频解码（平台 MediaCodec，应用侧） | 中转 | 有实测链路，未产品化 | evidence 80 |
| guest 内 VA-API 全链（播放器直接用） | 中转 | 部分 | evidence 81–87 |
| 视频编码（VA-API encode） | 中转（未做） | 未做 | — |
| 相机（V4L2 `/dev/videoN` + 平台编码） | 直连 | 有实测链路 | evidence 91–93 |
| 音频（PulseAudio → `audio-relay` → AAudio） | 中转 | 可用，10 秒级，非长稳；guest 里 `/dev/snd` 打不开 | evidence 62 |
| 输入注入（触摸/键盘） | 窗口层 | 能到应用；指针默认是 trackpad 语义，2.0 侧的 `touchMode` 默认值未在本仓库改 | evidence 148、180 |
| 传感器、GPS、USB、蓝牙、NFC | 未做 | 未开始：这几类既开不到节点，NDK 也没有对应平台 API | — |
| 指纹 | 不做 | 认证必须绑定可见 Activity 的用户交互 | — |
| 浏览器 | — | 随附 rootfs 无浏览器包，未跑过 | — |

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
