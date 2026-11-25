<!-- 首屏：标题一行，图标下一格。GitHub 只在空行处结束 HTML 块，两段必须分开，
     否则标题会退回左对齐。 -->
<h1 align="center">Husk</h1>

<p align="center">
  <img src="docs/assets/brand/husk-mark-hexagon-h-512.png" width="128" alt="Husk">
</p>

<p align="center">让 Linux 用户态以普通应用进程的身份，直接跑在 Android 内核上。</p>

<p align="center">
  <a href="https://github.com/xishanp/Husk/releases"><img alt="Release" src="https://img.shields.io/github/v/release/xishanp/Husk?style=flat-square&color=EA580C"></a>
  <a href="https://github.com/xishanp/Husk/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/xishanp/Husk/actions/workflows/ci.yml/badge.svg?branch=main&style=flat-square"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/xishanp/Husk?style=flat-square&color=2563EB"></a>
  <img alt="Platform: Android arm64" src="https://img.shields.io/badge/Platform-Android%20arm64-3DDC84?style=flat-square&logo=android&logoColor=white">
  <img alt="Rust 1.75+" src="https://img.shields.io/badge/Rust-1.75%2B-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="CONTRIBUTING.md"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-0F766E?style=flat-square"></a>
</p>

Husk 在 Android 上跑一个真实的 Linux 用户态：glibc、包管理器、编译器、shell 和服务都是普通
`untrusted_app` 进程，直接使用 Android 内核的系统调用。路径改写只在装载时做一次（改每个对象的
GOT），之后程序自己的 `svc` 指令原样进内核，每条 syscall 的成本与原生应用相同，沙箱放行的设备
节点可以直开。

没有 namespace，没有虚拟机，也不跑 `ptrace`。

运行时按 glibc 版本断言补丁点，不绑定某个发行版或某台设备。我们完整验证时用的是 Ubuntu 24.04
（glibc 2.39）和一台 arm64 Android 手机；型号与内核写在 `docs/STATUS.md`，那只是实测条件。

## 能跑起来的前提

> [!IMPORTANT]
> **这一节只讲一件事：它怎么才能跑起来。三条硬前提，缺一条就跑不起来；其它条件只影响跑得
> 多完整，不影响能不能跑。** 逐条的自查方式与原因在 [`docs/RUNNING.md`](docs/RUNNING.md) §2。

| # | 硬前提 | 不满足时的样子 | 怎么自查 |
|---|---|---|---|
| 1 | **arm64（aarch64）**：用户态是 aarch64 ELF，loader 的 GOT 改写与 syscall 直通只对这一个 ABI 成立 | 直接 `Exec format error` | `uname -m` → `aarch64` |
| 2 | **一个既能执行、又可写、沙箱也允许的目录**：loader 与用户态二进制都要被 `exec`。Android 上只有应用私有目录满足；`noexec` 挂载直接失败 | 看起来像 loader 坏了：`cannot execute the loader from here` | 直接跑 loader，它应回 `missing program name`（说明 `exec` 成功） |
| 3 | **系统没有对 `untrusted_app` 额外收紧 seccomp / SELinux**：用户态要 `execve` / `clone` / `open*` 直通内核；厂商另封一层就没有可绕的余地 | `Bad system call`，进程当场被杀 | 先跑 `bin/true`；或跑 `tools/device-tests/01-functional-matrix.sh` |

```sh
# 1、2：loader 自己能不能被 exec —— 期望输出 missing program name
cd files/linux/husk && ./libld.so

# 2、3：guest 程序能不能直通内核 —— 期望退出码 0
sh husk-session.sh files/linux/distributions/<发行版>/rootfs -- /usr/bin/true
```

## 目录

- [能跑起来的前提](#能跑起来的前提) · [为什么是 Husk](#为什么是-husk) · [和容器、虚拟机的区别](#和容器虚拟机的区别)
- [硬件接口](#硬件接口) · [特性](#特性) · [快速开始](#快速开始)
- [工作原理](#工作原理) · [性能](#性能) · [兼容性](#兼容性)
- [从源码构建](#从源码构建) · [仓库结构](#仓库结构) · [文档](#文档)
- [常见问题](#常见问题) · [参与贡献](#参与贡献) · [许可](#许可)

## 为什么是 Husk

同一台手机、同一份 rootfs、同一个 shell，只换运行器。5 轮中位数，2026-11-24 实测
（[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)）：

| 工作量 | Husk | proot | 倍率 |
|---|---:|---:|---:|
| `open` + `close` × 20 000 | 1 252 ms | 8 481 ms | **6.8×** |
| `make -j4`，200 目标链接 | 5 194 ms | 24 182 ms | **4.7×** |
| 顺序写 256 MB | 1 294 ms | 1 321 ms | 1.0× |
| 打开「设置」到首个窗口（桌面） | 4 462 ms | 5 353 ms | 1.2× |

前两行是路径与 syscall 密集的活，要付 proot 每条 syscall 的 `ptrace` 往返；第三行是大块写，
不经过被翻译的路径，所以两者持平。

> [!NOTE]
> 图形这一维也测了，但只有 Husk 这边有对象：桌面会话冷启动 8–9 s、X 根窗口 2400×1080
> 非黑像素 100%、帧率 640×480 clear+swap 是 **131 fps**，而同一台机器上软件渲染（llvmpipe）
> 是 300 fps。这一格 Husk 落后：把窗口 swap 拆开量过，GPU 渲染只要 1.3 ms，
> 其余是 GPU→CPU 回读再加进 X server 的拷贝（呈现约 24.8 ms/兆像素，DRI3 未通）。不是 GPU 算力。
> proot 侧没有可比的桌面，凡是「不可比」的格子是真的没有对象，不是我们没测。
> 完整条件、样本数与代价见 [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)。

### GPU 原生的实际加速

同一台机、同一份二进制（`gl-probe`），2400×1080 是本机日常分辨率：

| 负载 | GPU（zink → PanVK → Mali-G77） | 软件（llvmpipe） | 倍率 |
|---|---:|---:|---:|
| 着色器型：每像素 48 次 `sin`，2 遍，2400×1080 | 23.8 ms/帧（41.9 fps） | 266.7 ms/帧（3.7 fps） | **GPU 快 11.2×** |
| 同上，1280×720 | 14.9 ms/帧（67.1 fps） | 101.9 ms/帧（9.8 fps） | **GPU 快 6.8×** |
| 清屏 + 窗口呈现，2400×1080 | 50.2 ms/帧（19.9 fps） | 20.1 ms/帧（49.7 fps） | 软件快 2.5×，照实记录 |

第三行的原因是可拆的（2400×1080，ms/帧）：

| | GPU 渲染 | X 传输（`XPutImage`） | GPU→CPU 回读 | 实测合计 |
|---|---:|---:|---:|---:|
| zink | 2.25 | 17.66 | 30.27 | 50.18 |
| llvmpipe | 2.01 | 18.12 | 0（像素本来就在内存里） | 20.12 |

**算得动的活 GPU 快一个数量级；搬像素的活现在比软件慢**——每帧都要把 GPU 表面回读成线性
像素再交给 X server。要消掉这一段只能走 dma-buf 直传（DRI3），而本机的 X server 跑在应用
自己的进程里（bionic Xorg），`untrusted_app` 拿不到 `/dev/dri`、也没有 `/dev/dma_heap`，
DRI3 只能停在 `Could not get DRI3 device`。逐条数字与做法见
[evidence 174](docs/design/evidence/174-present-cost-split-20261125.md)。

## 和容器、虚拟机的区别

| | 传统容器 | 虚拟机 | PRoot 一类 | **Husk** |
|---|---|---|---|---|
| 内核 | 宿主内核（要 namespace/cgroup） | 自带内核 | 宿主内核 + `ptrace` | **宿主内核，零拦截** |
| 权限 | root / user namespace | 虚拟化 | 普通应用 + `ptrace` | **普通应用** |
| 每条 syscall 成本 | 原生 | 陷入虚拟机 | 一次 `ptrace` 往返 | **原生（0 额外）** |
| 路径隔离 | mount namespace | 完整隔离 | `ptrace` 逐条改写 | **装载时改 GOT，一次到位** |
| 设备访问 | 受 namespace 限制 | 虚拟设备 | 受 `ptrace` 限制 | **能直开的直开，其余走应用内后端** |

一句话：容器和虚拟机都是「把用户态装进一个边界」，Husk 是「让用户态自己跑，只在装载时
把路径改对一次」。所以它既不需要特权，也不为每条 syscall 付费。

> [!WARNING]
> **Husk 不是恶意代码沙箱。** 路径改写只覆盖「经过动态链接函数入口」的调用：自己发 raw
> `syscall` 的程序、静态链接的二进制、以及不经我们 loader 启动的进程，看到的都是宿主路径。
> 它能把普通 Linux 用户态按普通应用权限跑起来，**不能**用来隔离不可信的代码。

## 硬件接口

这是 Husk 和翻译层方案最实质的区别：**能用真实内核节点的路径使用真实 fd，必须经过 Android
应用身份的那部分由应用内后端提供标准 Linux 接口**。不把 facade 冒充成真设备节点，边界写在
[`docs/design/husk-backend-interface.md`](docs/design/husk-backend-interface.md)。

接法只有一根轴，不要混着说：

- **直连**——guest 进程自己开设备节点，应用侧一行代码都不参与（GPU）；
- **中转**——guest 看到标准 Linux 接口，后端经一条明确的 socket 协议到应用侧（相机、音频、视频）。

逐类的节点实测、理由，以及「能说什么、不能说什么」，写在
[`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md)。

> [!IMPORTANT]
> 表里写的是**验证到什么程度**，不是「功能已完成」。GPU 只到「能开、能算」，没到「能上线」：
> 窗口呈现（WSI）已经通了，但客户端的帧是经 SHM 拷贝进 X server 的，dmabuf 直传还没接
> （`docs/LIMITATIONS.md` §7）。另外 `glxgears` 会稳定触发 PanVK/Bifrost 编译器崩溃，
> 所以本版没有 GPU 长跑通过这一说；相机、视频、音频都是「有一条能跑的链路」，不是产品级的
> 稳定性承诺。

| 能力 | 接法 | 用户态里的路径 | 实测 |
|---|---|---|---|
| GPU | 直连 | 直开 `/dev/mali0`，kbase 后端的 PanVK | 枚举到 `Mali-G77 MC9`、compute 与离屏 graphics 回读通过；GL renderer `zink Vulkan 1.3(Mali-G77 MC9)`（evidence 156）；PanVK ICD 由 canonical guest 相对 manifest 选择，独立验证树已复跑 `vulkan-probe` / `gl-probe es3` 通过（evidence 241）。**X11 窗口呈现已通**（`stage 7→10`、60 swap，evidence 165），GNOME 桌面在 zink/PanVK 上出帧、X 根窗口非黑像素 100%（evidence 165/168）；默认值由加载器从 `/etc/husk/gpu-env` 注入，登录/非登录执行都吃得到（evidence 167、182、241）。上限 GLES 3.2（靠 `MESA_GLES_VERSION_OVERRIDE`，真实应用未验），呈现仍是 SHM 拷贝。**长跑未通过**：`glxgears` 在 PanVK/Bifrost 编译器 `emit_cf_list.isra.0+0x120` 稳定 SIGSEGV，pre-229 loader 同样复现；llvmpipe 对照和 gl-probe/ES3/Vulkan 探针通过（evidence 232） |
| 显示 | 应用侧，零拷贝 | `AHardwareBuffer` → dma-buf → `SurfaceControl` | 3-buffer ring + 真实 `SYNC_FD`：submitted=10 000 / completed=10 000（evidence 59） |
| 视频解码 | 中转 | MediaCodec `c2.mtk.avc.decoder` + AHB 三平面 | H.264 1080p30 连续 600 秒，`dropped=0`（evidence 80） |
| 相机 | 中转（标准 V4L2 facade） | guest `/dev/videoN` → Rust V4L2 shim → Camera2 NDK / `AImageReader` | `open()` 返回 memfd，不是真实 V4L2 fd；QUERYCAP / REQBUFS / DQBUF 可用，前后摄自动启动、无测试 marker；最终 0.0.15 构建复跑为 12.26–17.42 fps（evidence 240）。拍照 JPEG + 30 秒录像与硬件 H.264 编码见 evidence 93；不支持 DMABUF / read / controls |
| 音频 | 中转 | PulseAudio → Rust relay → AAudio | 48 kHz 双声道连续会话通过（evidence 62，10 秒级，不是长稳） |
| 输入 | 窗口层 | 触摸事件注入到用户态的窗口系统 | 触摸 → JNI → XI2，点哪落哪（evidence 148） |

这张表的口径：**能直开的设备使用真实 fd；必须经过应用身份的仍给标准 Linux 接口，但明确标注
facade 与中转**。按这一版实测：直连 1 类（GPU），中转 3 类（相机、音频、视频），显示那一段
零拷贝、输入走窗口层，传感器 / 定位 / USB / 蓝牙 / NFC 未做，指纹不做。没做到的写在
[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)。

GPU 默认值按设备节点分族，而不是按机型名写死：Mali/kbase 走 PanVK + Zink，Adreno/KGSL
把 guest 的 `/dev/dri/renderD128` 接成真实 `/dev/kgsl-3d0` fd，再交给 Turnip/freedreno。
Mali 路径已在本机验证；Adreno 适配器代码已落地，但**没有 Adreno 真机证据，状态是 UNKNOWN**，
不能写成已经支持（evidence 233）。

图形默认值也属于接口的一部分：ICD、GL 驱动、GLES 上限写在 `/etc/husk/gpu-env` 里，由**加载器**
在程序自己没设置时补，所以登录 shell、cron、桌面 fork 出来的进程、`env -i` 看到的是同一套
渲染器；程序自己 `export` 的永远优先（evidence 182）。文件首行的 `@GPU_FAMILY` 会阻止把
Mali 的 ICD/驱动配置带到一个 Adreno rootfs 上；登录 shell 仍按当前设备节点重新探测。

## 特性

- **零 `ptrace`、零 syscall 拦截。** 内核看到的是用户态自己的 syscall（evidence 130–146）。
- **原样的 glibc 用户态。** 除 libc 内部调用需要的 slot 项，树的二进制没有为 Husk 改过。
- **包管理器可用。** `apt-get install` 27 秒把树从 815 包装到 882 包，含 `build-essential`、
  `nodejs`、`python3-numpy`（evidence 154）。
- **工具链完整。** `gcc`、200 目标工程的 `make -j4`、`git clone`、`python3` + `numpy`
  （evidence 155）。
- **会话服务齐全。** polkit 授权、at-spi 无障碍总线、gvfs、RandR 都能在用户态里直接用
  （evidence 151、152）。
- **不需要任何特权。** 跑在普通应用沙箱里，机器可以是原厂状态。
- **图形默认值在加载器里，不在登录脚本里。** 非登录执行（cron、systemd user unit、桌面 fork
  出来的进程、`env -i`）也不会掉回软件渲染；程序自己设的变量永远优先（evidence 182）。
- **对比可以自己重跑。** `tools/bench/` 一条命令把同一份 rootfs 交给三个运行器，
  原始日志在仓库里。

## 快速开始


> [!IMPORTANT]
> **这些资产不能脱离应用沙箱直接用在别的终端环境里。** 别的环境没有这套依赖，也没有「可写 +
> 可执行 + 沙箱允许」三者同时成立的目录。交付形态只有一种：打进 APP，和 proot 方案一样。

交付形态和 proot 方案一样：**把运行时作为 assets 打进 APP**，首次使用时解压到应用私有目录。

```text
APK assets/husk/arm64-v8a/
  libld.so          loader（内嵌 path-shim）
  libc.so.6         打过两处补丁的 glibc
  husk-ldconfig     写 ld.so.cache 与 ld.so.husk-path
  husk-usermgr      useradd / groupadd / passwd 一族替身
  husk-su           提权替身
  husk-symlinks     绝对符号链接改相对 + 校验（否则 dlopen 找不到明明存在的库）
```

首次启动自动完成：解压 loader → 放三个工具进 `usr/sbin` → 按 sha256 替换 libc（只认已知
基线，不覆盖未知发行版）→ `ldconfig` 指向 `husk-ldconfig` → shadow 工具指向 `husk-usermgr`
→ 建 `run` / `tmp` / `dev/shm` → 写版本标记 `files/linux/husk/assets.version`。

会话启动契约（缺一个就看起来像 shim 坏了）：

```sh
NR_ROOTFS=<rootfs> NR_LD=<loader> NR_LIBRARY_PATH=<lib/aarch64-linux-gnu:usr/lib/...> \
  <loader> --library-path "$NR_LIBRARY_PATH" usr/bin/env -i PATH=... HOME=/root /bin/sh
```

装上发行版之后**必须改的 7 件事**（每一项为什么改、不改会怎样、由谁改）：见
[`docs/RUNNING.md`](docs/RUNNING.md)。

开发机上不装 APP 也能跑（`/data/local/tmp` 之类可执行的目录），这是调试路径，不是交付形态：

```sh
adb push husk-arm64.tar.gz /data/local/tmp/husk/
adb shell
cd /data/local/tmp/husk && tar xzf husk-arm64.tar.gz
sh install-husk.sh --rootfs linux-rootfs.tar --dest ./root --loader ./libld.so
./husk run linux
```

## 工作原理

<p align="center">
  <img src="docs/assets/architecture.svg" alt="Husk 架构" width="760">
</p>

Android 唯一执行的是一个静态 loader，path-shim 内嵌在里面。它映射用户态对象时改写该对象的
GOT，让每个 libc 路径调用落到 shim；shim 加一次根前缀，然后发真正的 syscall。`exec` 由
loader 自己解析（含 `PT_INTERP` 与 shebang），嵌套 exec 会重新回到同一个 loader，而不是掉回
宿主。因为此后没有任何东西挡住进程，程序从第一条指令起，syscall 就是原生的。

细节见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)。

## 性能

八维度对比表、每个数字背后的机制、以及这套做法的代价，都在
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)。要点：优势随 syscall 与路径密度放大
（进程类 1.5×，路径类 3–7×），在大块 I/O 上消失。

## 兼容性

> [!NOTE]
> 「可用」指在目标机上真的执行过并得到预期结果，不是「包已经装了」。浏览器不可用：随附的
> rootfs 来自离线包索引，里面没有浏览器包。

参考环境下验证过的：`bash`、coreutils、`git`、`python3`（+`numpy`）、`gcc`、`make`、
`vim`、`nano`、`node`、`apt`、`dpkg`、dbus、polkit、at-spi、gvfs。发行版层面完整验证过的是
参考发行版 Ubuntu 24.04 / glibc 2.39；逐项矩阵见
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)。

## 从源码构建

> [!WARNING]
> `make test` 会编译 aarch64 内联汇编，只有在 arm64 Linux 主机上能跑；在 macOS / x86_64 上
> 编译不过，那是平台限制，不是补丁坏了——那边用 `make check`。

```sh
# loader（需要 NDK；会断言它补丁的 glibc 版本）
NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin \
  sh tools/loader/build-embedded-loader.sh

# Rust crates（no_std shim + 宿主工具）
cargo build --release --manifest-path runtime/Cargo.toml

# 本地门禁：CI 把同一组命令拆成七个作业跑，任何一个红了就不算过
make check   # fmt + aarch64 cargo check + clippy + shell 语法 + 文档链接 + 版本三元组
```

## 仓库结构

```
runtime/crates/    loader 侧的 path-shim、exec、PTY、信号（Rust，no_std）
runtime/crates/nr-tools/     ELF 手术：scan、patch、embed、libc interpose
tools/loader/      入口与 hook 汇编，产出 loader 的构建
tools/bench/       产出 docs/BENCHMARKS.md 的三运行器夹具
tools/guest/       rootfs 安装器、会话脚本、release 打包（不需要 APK）
tools/guest-config/  libc 补丁、ldconfig、符号链接改写
tools/device-tests/  设备验收套件，含门禁 A
docs/design/evidence/  文档里每个数字背后的设备原始输出
```

## 文档

| 文档 | 回答什么 |
|---|---|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | loader、shim、exec 图怎么拼起来 |
| [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) | 对比、机制、代价 |
| [`docs/RUNNING.md`](docs/RUNNING.md) | 怎么装怎么跑，含 rootfs 准备三步 |
| [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) | 每个运行器能跑什么 |
| [`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md) | 每类硬件怎么接：直连 / 中转 / 未做 |
| [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) | 什么不工作，为什么 |
| [`docs/STATUS.md`](docs/STATUS.md) | 目标机上的当前状态 |
| [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md) | 1.0 之前的工程日记 |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | 怎么构建、怎么测、术语表 |
| [`QUICKSTART.md`](QUICKSTART.md) | 从零跑通：生成 rootfs、构建、推到设备 |
| [`ROADMAP.md`](ROADMAP.md) | 对外承诺：交付了什么、下一步、不做什么 |
| [`SUPPORT.md`](SUPPORT.md) | 先读哪几份、怎么提问、不在支持范围 |
| [`SECURITY.md`](SECURITY.md) | 什么算漏洞、怎么私密上报 |

## 常见问题

**需要 root 或解 Bootloader 吗？** 不需要。跑在 `untrusted_app` 里，能碰到的东西就是
应用沙箱允许的那些。

**这是容器还是虚拟机？** 都不是。同一个内核，没有 namespace、没有 cgroup、不模拟任何指令。
用户态就是一棵普通进程树。

**为什么 `apt-get update` 只快 1.1×？** 因为它受镜像和 dpkg 自身的工作限制，不取决于路径
查找。基准照实写，不粉饰。

**能跑 Debian / Fedora / Alpine 吗？** 只完整验证过参考发行版（glibc 的 Ubuntu 24.04）。
Alpine 是 musl，是另一个问题；其余没跑过。装载器的补丁是版本断言的，换树需要重新验证。

**为什么 rootfs 要打补丁？** 两个 Android 特有的缺口：libc 内部调用需要 slot 项；rootfs 里的
绝对符号链接会按 Android 根解析。步骤在 `docs/RUNNING.md`，工具在 `tools/guest-config/`。

> [!CAUTION]
> **能在 Termux 里跑吗？不能，也不支持。** 用户态需要的二进制与库 Termux 的 `$PREFIX`
> 不提供，在那边跑会直接卡在缺依赖上。

**验证过哪些 Android 与内核？** 参考环境是 Android 13、kernel 4.19.191、4 KiB 页、arm64
（具体型号见 `docs/STATUS.md`）。16 KiB 与 64 KiB 页只在 QEMU 上跑过。这不是要求，是实测
条件。

## 参与贡献

欢迎 issue 与 pull request；先读 [`CONTRIBUTING.md`](CONTRIBUTING.md)。注意：每个性能结论
都要有一个 `docs/design/evidence/` 下的证据文件。本项目遵循
[Contributor Covenant 2.1](CODE_OF_CONDUCT.md)。安全问题见 [`SECURITY.md`](SECURITY.md)。

## 许可

Apache License 2.0，见 [LICENSE](LICENSE)。第三方组件、glibc 构件与商标声明见
[NOTICE](NOTICE)。

## 致谢

`docs/BENCHMARKS.md` 里的对比是靠运行 [proot](https://proot-me.github.io/) 与 proot-distro
得到的——它们曾是 LinuxHub 2.0 的运行时，这些数字存在是因为它们能用。loader 的补丁针对的是
发行版发布的 glibc。
