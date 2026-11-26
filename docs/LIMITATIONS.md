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

## 7 · GPU：桌面出帧已通，fixed-function GS 有默认规避，但呈现仍拷贝

**2026-11-25 的现状**：gnome-shell 46 跑在 zink → PanVK 上，X 根窗口抓到完整桌面、非黑像素
100%（evidence 165/168）；GTK4 应用因为圆角裁剪缺口默认走 `GSK_RENDERER=cairo`（evidence 187）；
`nautilus` 46 的内容区实测完整（evidence 207）。还没消掉的是「窗口内容 → X server」那一次
回读 + 拷贝，以及这颗 SoC 上没有 DRI3 要的 render 节点。

**已经是 FACT（evidence 156，2026-11-24，当前版本的树）**：guest 进程直开 `/dev/mali0`；
kbase 后端的 PanVK（FristOneRR，Mesa 26.3）枚举出 `Mali-G77 MC9`，compute 与离屏 graphics
都能跑并回读 GPU 写入的值；开 Zink 后 EGL/GL 的 renderer 是
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，不开就是 `llvmpipe`。

**2026-11-26 ICD 路径修正（FACT，evidence 241）**：0.0.15 在独立验证树上看到的
`vulkan-probe` / `gl-probe` SIGSEGV，不是“PanVK 驱动旧”导致的。根因是 ICD JSON 的
`library_path` 写成宿主 `/data/data/...`；路径 shim 会把它当 guest 路径再翻译，Vulkan loader
于是拿到空驱动。App 现在生成 canonical guest 相对 manifest
`/usr/share/vulkan/icd.d/husk_panvk_guest.json`，安装器、登录 shell、会话脚本使用同一路径；
同一驱动的 A/B 结果是：旧 manifest `rc=139`，新 manifest 下 `vulkan-probe` 枚举
`Mali-G77 MC9` 并通过 compute/graphics，`gl-probe es3` 报 `RESULT PASS`。这只修正驱动定位，
不改变下面的 SHM 呈现限制，也不能写成 `glxgears` 已经长跑通过。

**2026-11-26 loader 级默认值（FACT，evidence 242）**：App 现在只在 `/dev/mali0` 真实可打开
且 PanVK 驱动存在时生成 guest `/etc/husk/gpu-env`；经 Husk loader 启动的进程即使 `env -i`，
也会从该文件得到 canonical ICD、EGL vendor、Zink 和 GLES 3.2 默认值。真机无 GPU 变量的
`env -i` 下 `gl-probe es3` 仍报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` 并
`RESULT PASS`；显式 `HUSK_GPU=llvmpipe` 正确回退。这只说明默认值层级和设备门控，不改变
窗口呈现/长跑限制。

**2026-11-26 GPU 默认值 crate 与 Adreno 启动分支（FACT/UNKNOWN，evidence 243）**：默认值
生成已抽成纯 Rust `gpu-env` crate，App 用它生成初始环境和 loader 读取的 Mali/Adreno 配置。Mali 真机
回归通过；Adreno 的 `/dev/kgsl-3d0`、Freedreno/Turnip、ioctl/mmap、桌面和长跑没有真机证据，
仍是 UNKNOWN。本版还让软件回退显式携带 `HUSK_GPU=llvmpipe`，避免 loader 重新注入硬件默认值。

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
   **2026-11-25 更正，并已给修法**（evidence 186/187）：`GSK_RENDERER=ngl` 下 GTK4 的窗口
   其实不是"完整出画"——CSD 的**圆角裁剪层**在 zink/PanVK 上算错，窗口里会出现直边三角/菱形
   缺口（关掉 `box-shadow` 只减 15%，`border-radius: 0` 能去掉其中 84%；逐帧一致；`llvmpipe`
   与 `cairo` 都不复现）。所以默认值改成 **`GSK_RENDERER=cairo`**：安装器同时写进
   `/etc/profile.d/husk-gpu.sh` 与 `/etc/husk/gpu-env`，后者由加载器在程序没设过时注入
   （实测缺口像素 10 850 → 9）。代价是 GTK4 的界面绘制改吃 CPU，Vulkan/GL 不受影响；
   `GSK_RENDERER=ngl` 可切回 GPU 并复现缺口。根因（PanVK 的模板/裁剪路径）**UNKNOWN**。
   **反例是文件管理器**：`nautilus` 46 能开窗、能枚举目录（日志逐条打出 `/etc/*`），但内容区
   整块空白、连列头都不出现；`GSK_RENDERER=cairo` 与 `ngl` 一样、`list-view` 与 `icon-view`
   一样，**同一份 rootfs 换 proot 起也照样空白**，同一会话里最小 `GtkColumnView` 程序却正常
   ——所以当时判断不是 path-shim 造成的（evidence 178）。
   **2026-11-25 复查：没能复现**（evidence 188）。同一条 `nautilus --new-window /etc`，
   `cairo`+zink、`ngl`+zink、`cairo`+llvmpipe 三种组合内容区都完整（面包屑、侧栏、图标网格
   全在），`ngl` 只多出 187 那种三角缺口。178 那次的可疑条件（`xdotool windowkill` 后重建
   窗口、1200x540 时代的旧几何、实验版加载器）当时同时成立，所以按**会话态问题**记，
   不再当成渲染路径的已知 bug。
   **2026-11-25 定案**（evidence 207）：清掉旧实例后 `nautilus --new-window /root/vtest`，
   15 秒出窗（`281,142 1012x672`），窗口内 26 286 种颜色、内容区 17 223 种、白底占比 0.757、
   672 行里有 44 行带文字边——内容区不空白。「点了没反应」的真实原因是单实例被上一轮会话的
   残留进程劫持：`--new-window` 把请求转给一个 X 连接已死的实例，连等 60 秒一个新窗口都不出；
   先 `kill -9` 再起就正常。`husk-desktop.sh` 起会话时清残留那一步就是治这个的。WebKit/Electron 类**未测**。
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

**2026-11-26 增补：`glxgears` 不是长跑通过（FACT，evidence 232）**。当前 loader 和 pre-229
loader 下，`glxgears -info` 都在 45 秒超时前自行退出 `RC=139`；崩溃处理器给出 `fa=0x21`，
相对地址落在 `libvulkan_panfrost_x11.so` 的 `emit_cf_list.isra.0+0x120`，对应指令为
`ldr x28, [x28]`。同一轮里 llvmpipe 跑到预期的 20 秒超时（约 348–371 FPS）、
`gl-probe window`、ES3 workload 和 `vulkan-probe` 都通过。结论是 PanVK/Bifrost 的固定
着色器编译路径有已知崩溃，不是“Mali 完全没有起来”；具体 NIR 破坏和其他应用命中率仍是
**UNKNOWN**。不能把默认渲染器改成 llvmpipe 来掩盖问题，也不能据此宣称 GPU 长稳通过。

**2026-11-26 fixed-function 默认规避（FACT/UNKNOWN，evidence 244）**：不启动 X11 的
`gl-probe fixed` 证明崩溃发生在 fixed-function 转译后的内部 geometry shader，不是窗口/呈现层。
PanVK 的 DXVK 兼容开关默认暴露 `geometryShader`；0.0.19 在 Mali 的运行时默认值中设为
`PANVK_DXVK_COMPAT=0`，并让 loader 把这个默认值物化到真实 `setenv`/`execve` 环境。默认配置下
fixed-function 探针 600 帧 `PASS`（588.5 fps），ES3 和 Vulkan 回归也通过；显式
`PANVK_DXVK_COMPAT=1` 仍在 `fa=0x21` SIGSEGV。**这不是 PanVK 编译器修复**；依赖 geometryShader
的 DXVK 程序必须自己显式开启，当前默认不保证 DXVK 可用。0.0.20 已在真实 X11 窗口里把
`glxgears` 连续跑满 600 秒，119 个 5 秒窗口、147872 帧、215.779–299.303 FPS、平均
248.352 FPS；进程实际持有 `/dev/mali0` 并映射 PanVK/GLX 驱动
（evidence 245）。但这只证明 **Husk 的 guest GLX 路径**，当前 2.0 拉起的 GNOME 仍显式设置
`GALLIUM_DRIVER=llvmpipe`、`MESA_LOADER_DRIVER_OVERRIDE=swrast`、
`LIBGL_ALWAYS_SOFTWARE=1`，约 1.0–1.2 FPS。硬件 GNOME 桌面、多应用并发和其他
fixed-function 程序仍是 UNKNOWN，单程序回退仍是 `HUSK_GPU=llvmpipe`。

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
| 文件管理器（Nautilus 46） | — | 能开窗、能枚举、**能渲染**（内容区不空白，evidence 207）；首次出窗约 15 秒；旧会话残留实例会让 `--new-window` 不出窗，单一入口已清 | evidence 178、207 |
| GPU：Vulkan compute / 离屏 graphics（`/dev/mali0` + kbase） | 直连 | 可用 | evidence 156 |
| GPU：GL/EGL 经 Zink 落到 GPU（默认上限 GLES 3.2，靠 override；真实应用未验） | 直连 | 可用；默认值由加载器注入，非登录执行也吃得到 | evidence 156、161、167、182 |
| GPU：X11 窗口呈现（EGL window surface + swap） | 直连（呈现这一跳不是） | 可用，呈现走回读+拷贝（DRI3 未通，代价 24.8 ms/MP） | evidence 165、174 |
| GL 整块读回（`glReadPixels` W×H） | — | **坏**：目的行距恒 16 字节；逐行/逐像素读正确 | evidence 179 |
| GPU：桌面合成器（gnome-shell 46 on zink/PanVK） | 直连 | 可用，出帧；轻负载 fps 不如 llvmpipe | evidence 165、168 |
| 显示上屏（dma-buf → AHB → `SurfaceControl`） | 应用侧，零拷贝 | 可用（10 000 帧 ring） | evidence 59 |
| 硬件视频解码（平台 MediaCodec，应用侧） | 中转 | 有实测链路，未产品化 | evidence 80 |
| guest 内 VA-API 全链（播放器直接用） | 中转 | 部分 | evidence 81–87 |
| 视频编码（VA-API encode） | 中转（未做） | 未做 | — |
| 相机（V4L2 `/dev/videoN` facade + Camera2 NDK + 平台编码） | 中转 | 有实测链路；`open()` 是 memfd，不支持 DMABUF / read / controls，约 15–17 fps | evidence 91–93、240 |
| 音频（PulseAudio → `audio-relay` → AAudio） | 中转 | 可用，非长稳；本轮播放 1 次 xrun、平均 121.667 ms，采集协议通过但未做声学质量测试；guest 里 `/dev/snd` 打不开 | evidence 62、231 |
| 输入注入（触摸/键盘） | 窗口层 | 全屏点击与双客户端已验证；`adb input swipe` 的 XI 事件仍是 UNKNOWN；指针默认是 trackpad 语义，2.0 侧的 `touchMode` 默认值未在本仓库改；**拖动窗口失效**（见 §10） | evidence 148、180、203、230 |
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

## 10 · 拖动窗口：只有会话里的第一次会失效，已用「输入预热」绕掉

**结论先说**：拖动、对齐、边缘平铺**都是好的**。唯一的问题是**每个会话里的第一次**指针拖动
不搬窗，第二次起就正常；`husk-desktop.sh` 现在会先做一次空点击 + 8 px 空拖动把这一次消耗掉
（evidence 206）。

**实测**（FACT）：

| 场景 | 第一次拖 | 第二次拖 |
|---|---|---|
| 起会话后立刻拖 | `236,188` 不动 | `536,388`（+300,+200） |
| 两次连着做 | `236,188` 不动 | 紧接着就 `536,388` |
| 加了预热之后 | `536,388`（第一次就动） | — |

拖到左边缘松手 → `46 12 1207 1088`，即 2400×1080 的左半屏平铺，吸附正常。

**第一次为什么失效（UNKNOWN）**：mutter 日志里这一次 grab op 是成功的、指针位置正确、
连"Computing edges to resist-movement or snap-to"都跑了，但位置一次都没更新（evidence 206 §2）。
是 mutter 首次算边缘时 work area 还没建好，还是首次 `ConfigureWindow` 之前 server 侧没准备好，
没有证据。预热**不是**修根因，是把一次已知会失效的操作提前消耗掉。

**事件层已全部排除**（evidence 205/206）：核心抓取（窗口级与 root 级）、`XISelectEvents`、
`XIGrabDevice` + XI2 事件、XI2 motion 的 `root_x/root_y` 坐标——全部正常。

> 历史更正：197 / 198 / 203 / 205 都写着"拖动完全失效"，那四份测的全是
> **新起会话里的第一次拖动**。205 初版还把原因归到“XI2 选择/枚举坏了”，那是我们自己的探针
> 把请求构造错了。这两条都在各自文件里留痕并改掉了。

**还剩一条不归我们管**：指针默认是 trackpad 语义（1:1 相对移动，不是触摸的绝对定位），
那个默认值在 2.0 应用的 `touchMode` 里，本仓库没改。

## 11 · 屏幕一灭，桌面「像死了一样」：那是 Android 冻结，不是崩

`dumpsys power` 显示 `mWakefulness=Asleep` 时，应用转 cached，Android 的
Cached Apps Freezer 会把它的全部线程 SIGSTOP；X server 住在应用进程里，于是
**根窗口还在、但新连接一律卡住**。判据：`/proc/<x11 pid>/task/*/wchan` 全是 `get_signal`，
而 `echo`/`id` 这类不碰 X 的 guest 命令秒回。唤醒（`input keyevent KEYCODE_WAKEUP`）
或把应用切回前台即恢复，**不需要重启应用**（FACT，evidence 203）。

所以：真机上「锁屏一会儿回来桌面卡住」先看这一条，别当崩溃查。

## 12 · 中文：locale、字体、输入法、界面翻译都通了，繁體没有

**已经做到的（FACT，evidence 210 / 211）**：默认 `LANG=zh_CN.UTF-8`
（`/etc/profile.d/husk-locale.sh`，程序自己设了就不覆盖）；`fc-match "sans-serif:lang=zh-cn"`
落到 Noto Sans CJK SC，不再回落到日文字形；会话起来拉起 fcitx5 并打 `CHECK im=fcitx5`；
`ctrl+space` 切拼音后在 `gnome-text-editor` 里能打出「你好」并落盘；界面翻译由树包里附带的
语言包提供（413 个 `.mo` 装到 `/usr/share/locale-langpack/zh_CN/LC_MESSAGES/`），
`gnome-calculator --help` 在中文下输出「用法：/帮助选项：」。

**没做到的**：只有 `zh_CN`，`zh_TW` / `zh_HK`（繁體）没打；翻译版本绑在
`24.04+20260905`，换 Ubuntu 版本要重下（步骤见 `tools/langpack/README.md`）。

**其它未测**：输入法只在 GTK4 上验过；XIM 前端、Qt 应用、候选窗翻页与数字选词没测。

**判断输入法是否正常的正确判据**（踩过一次）：没有焦点窗口时 `fcitx5-remote -o` 与 `-n`
都返回空、状态保持 0，看上去像"没起来"，其实是"没有输入上下文可切"。必须给一个焦点窗口，
看到状态从 0 变 2、再把提交的字符从应用里读回来，才算验过。

## 13 · 硬链接模拟与挂载表的边界（2026-11-26 更新）

**硬链接只做「复制 + 登记」。** app 数据目录里 `link(2)` 被 SELinux 拒绝，shim 的回退是
复制一份并把源 inode 记进别名表，源 fd `close()` 时再重刷一次。程序自己 `write()` +
`close()` 的刷新生效；`fopen` / `fputs` / `fclose` 的 stdio 路径在 0.0.21 起也会在真实
`fclose` 前先 `fflush`，再用 `fileno` 刷新别名（evidence 246）。这修掉了旧版
`echo AAA > a; ln a b; echo BBB >> a; cat b` 只看到 `AAA` 的常见缺口。**仍然不要把硬链接
模拟当完全等价**：link count、共享 inode、跨进程即时共享本来就丢了；绕过导出 `fclose`
的 glibc 内部关闭路径仍是 UNKNOWN。

**挂载表默认每次重读，`NR_MNT_REFRESH=0` 可关。** 表是
`rootfs/run/husk/mounts` 这个跨进程共享的文件；以前每进程只读一次，于是另一个进程刚做的
mount，已经在跑的进程看不到——同一个路径在两个进程里指向两个目录，写进去的字节会「消失」
（evidence 221 §1）。现在默认每次重读，实测代价 +0.5%（`find /usr` 23 万次路径解析）。
关掉它是允许的，但等于接受上面那种静默写错位置。

## 14 · 会话形态：umask 是入口定的，`/proc` 魔术链接只能读不能走（2026-11-25 实测）

**umask 由会话入口设成 022，不跟 guest 的 `/etc/login.defs` 走。** Android 给应用的 umask
是 0000，shim 不改它（那是进程属性，不经过任何被钩的 libc 函数）。要按发行版那套
UMASK/USERGROUPS 规则算，就得在 shim 里重写 `open`/`mkdir`/`creat` 的 mode 参数——那等于
在每个创建点上多一层推断，代价和风险都不划算。所以两个入口（`husk-session.sh`、
`husk-desktop.sh`）统一 `umask 022`，需要别的值的程序自己调 `umask(2)`（会被正常执行）。
evidence 222 §1。

**`/proc/self/cwd` 和 `/proc/self/root` 可以读，不能拿来做路径前缀。** `readlink` 现在答的是
guest 视图（`/root`、`/`），但 `open("/proc/self/cwd/x")`、`ls -l /proc/self/cwd` 仍然 ENOENT：
这两个魔术链接解析出来是宿主路径，放行等于给了跳出 rootfs 的一条路。真正需要「当前目录的
绝对路径」的代码应该用 `getcwd(3)`（已改写）。evidence 222 §2。

**信号处置是从 Android 父进程继承的。** 实测 guest 侧 `SigIgn=0x2000000000`（一位 RT 信号被
忽略）、`SigCgt=0x400004c8`，与同一台机器上 `adb shell` 直接 `cat /proc/self/status` 逐位相同
——也就是说 loader 既不添也不清。会话不是 `login(1)` 起的，别指望信号表是干净的。
evidence 222 §5。

## 15 · `close_range`：用户态兜底可用，但不模拟 fd-table unshare（2026-11-26）

Android 13 的 app seccomp 会拦截 syscall 436；glibc 的 `close_range()` 不会在
`ENOSYS` 后自动回退。0.0.22 起由 path-shim 枚举 `/proc/self/fd`，按真实最大 fd 执行原始
`close(57)`，所以普通关闭与 `CLOSE_RANGE_CLOEXEC` 可用，`3..UINT_MAX` 也不会退化成超大循环
（evidence 248）。

**不支持的是 `CLOSE_RANGE_UNSHARE`**：fd table 的 unshare 是内核语义，用户态假装成功会让另一个
线程仍共享同一张表。当前明确返回 `EOPNOTSUPP`。多线程并发 open/close 与 V4L2、Adreno 真机上的
批量关闭仍未压测，UNKNOWN。

## 16 · 假 netlink fd 的复制与跨进程边界（2026-11-26）

0.0.23 起，假 netlink socket 用 client / peer 的 socket inode 识别，`dup`、`dup2`、
`fcntl(F_DUPFD_CLOEXEC)` 都算同一 socket 的引用；关闭原 fd 不会拆掉仍存活的复制 fd，
最后一个引用关闭时才清理私有 peer（evidence 249）。

**仍未支持的是 `SCM_RIGHTS` 跨进程传递假 netlink fd**：接收进程没有对应的 `NL_SLOTS` 状态，
不能保证继续模拟 netlink。多线程同时 dup/close 的极端竞态也没有专项压测，UNKNOWN。

## 17 · V4L2 假相机 fd：高位复制已修，跨进程仍是边界（2026-11-26）

0.0.24 起，假 V4L2 fd 的引用计数枚举 `/proc/self/fd`，不再只看 0..1023；`dup()` 到高位后
关闭原 fd 不会提前拆除 camera socket。path-shim 与 preload 对 `V4L2_CTX` 的访问也已加锁
（evidence 250）。

**仍未支持的是 `SCM_RIGHTS` 跨进程传递假 V4L2 fd**：memfd 本身能传，但接收进程没有对应的
设备、socket 和 buffer 表，不能保证继续模拟 V4L2。真实 Camera2 后端在高 fd 下的长时间
STREAMON/DQBUF 与多线程压力仍未验证，UNKNOWN。

## 18 · DRM / Adreno 假 fd 的覆盖写（2026-11-26）

0.0.25 起，`dup2`/`dup3` 覆盖假 DRM fd 时，path-shim 会先清掉目标号码的 `FAKE_FDS` 表项；
只有源 fd 仍是假 DRM 时才重新登记。因此普通文件覆盖旧假 fd 后，DRM ioctl 不会再被错误送进
`DRM_CTX`，而假 fd 复制到新号码后仍保持 DRM 语义（evidence 251）。`ADRENO_FDS` 同步采用
相同的目标清理顺序。

**仍未验证的是 Adreno/KGSL 真机**：当前设备只有 Mali-G77，Adreno 分支只有源码静态审计。
通过 `SCM_RIGHTS` 把假 DRM fd 交给另一进程后的表身份恢复仍未实现；多线程同时
`dup2`/`dup3`/`close` 的长时压力也仍是 UNKNOWN。

## 24 · V4L2 假节点的 `O_CLOEXEC` / `O_NONBLOCK`（2026-11-26）

0.0.31 起，path-shim 对 `/dev/video0` 返回的 memfd 会保留调用方请求的 `O_CLOEXEC` 和
`O_NONBLOCK`。旧 0.0.30 loader 两项都会丢：相机 fd 可泄漏进 `exec` 子进程，非阻塞采集也会
退化成普通 fd 语义；修后真机为 `fdflags=1 fl=133122`（evidence 258）。

netlink 是同类接口里的控制项，不是本轮缺陷：带 `SOCK_CLOEXEC | SOCK_NONBLOCK` 的
`socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE)` 在真机返回 `fdflags=1 fl=2050`，现已加入
fd identity 门禁。跨进程 `SCM_RIGHTS`、真实 Camera2 长时间采集和多线程压力仍是 UNKNOWN。

## 23 · DRM 假节点的 O_CLOEXEC 与初始化失败（2026-11-26）

0.0.30 起，path-shim 和 preload 返回的 DRM 假 fd 会保留调用方请求的 `O_CLOEXEC`；旧实现
固定返回没有该标志的 fd，导致 `exec` 后意外继承。真机负对照为 `fdflags=0`，修后为
`fdflags=1`（evidence 257）。

kbase 初始化失败路径也会关闭临时 mali fd，并把负 errno 返回给 open 调用方。该分支没有在
真机上人为制造失败，因此只能算源码审计修复，不能写成设备已验证。

独立 `libdrm_preload.so` 仍是内部/实验组件，当前发布 tar 不包含它；path-shim 的同项修复
随 `libld.so` 发布。

## 22 · DRM preload 的 mmap / ioctl 失败语义（2026-11-26）

0.0.29 起，`libdrm_preload.so` 的 `mmap` 失败按 ABI 返回 `MAP_FAILED`，并保留内核给出的
`EBADF`、`EINVAL` 等 errno；普通 fd 的 `ioctl` 失败也不再被改写成 `EPERM`。DRM 回调改用原始
系统调用，`DrmCtx` 仍能收到负 errno（evidence 256）。

**仍未证明的是**：preload 在路径 shim 之外直接处理非 DRM 路径时的完整翻译与隔离，以及
Adreno/KGSL 真机上的失败路径。

## 21 · DRM preload 的 open ABI（2026-11-26）

0.0.28 起，`libdrm_preload.so` 的 `openat` 失败会保留 glibc 给出的真实 errno；`open`、
`open64`、`openat`、`openat64` 也都会把创建 mode 交给内核。旧实现的 `ENOENT → EPERM`
改写和 `O_CREAT` 文件 mode=000 已在真机负对照里复现并修掉（evidence 255）。

**仍需单独审计的是**：当这个 preload 在路径 shim 之外直接处理非 DRM 路径时，绝对路径翻译和
跨进程路径语义没有在这三项里证明；不能把 open ABI 通过写成完整路径隔离已经成立。

## 20 · DRM preload 假 fd 的覆盖写（2026-11-26）

0.0.27 起，`libdrm_preload.so` 不再保存 fd 号码身份表。每次判定都通过 `/proc/self/fd/<n>`
读取内核目标，只有 `memfd:husk-*` 或真实 `mali` 节点才进入 DRM 模拟；普通文件通过
`dup2`/`dup3` 覆盖旧号码后不会再被误认（evidence 254）。假 fd 复制到新号码、源关闭后的
识别仍然成立。

**仍未验证的是 Adreno/KGSL 真机**：当前设备只有 Mali-G77。通过 `SCM_RIGHTS` 把假 DRM fd
交给另一进程后的身份恢复未实现；多线程同时 `dup2`/`dup3`/`close` 的长时压力仍是 UNKNOWN。

## 19 · V4L2 假 fd 的覆盖写与跨进程边界（2026-11-26）

0.0.26 起，V4L2 不再把 fd 号码当设备身份。每次 ioctl / mmap / close 都通过
`/proc/self/fd` 的 `memfd:husk-v4l2-N` 名称复核；`dup2`/`dup3` 覆盖假 fd 后，path-shim
会清核目标号码的旧设备，零 live memfd 引用时关闭 camera.sock（evidence 253）。旧 loader
在普通文件覆盖假 `/dev/video0` 后会把 `VIDIOC_QUERYCAP` 伪报成 `driver=husk`，现已用真机
负对照和修后探针钉住。

**仍未支持的是 `SCM_RIGHTS` 跨进程传递假 V4L2 fd**：memfd 本身能传，但接收进程没有对应的
设备、socket 和 buffer 表，不能保证继续模拟 V4L2。多线程同时对同一 fd 做
`dup2`/`dup3`/`close` 的长时压力仍未验证，UNKNOWN。
