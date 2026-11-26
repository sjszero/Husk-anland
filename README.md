<!-- 首屏只放标题、图标和徽章。GitHub 只在空行处结束 HTML 块，各段必须分开，
     否则标题会退回左对齐。项目介绍从下面的“概览”开始。 -->
<h1 align="center">Husk</h1>

<p align="center">
  <img src="docs/assets/brand/husk-mark-hexagon-h-512.png" width="128" alt="Husk">
</p>

<p align="center">
  <a href="https://github.com/xishanp/Husk/releases"><img alt="Release" src="https://img.shields.io/github/v/release/xishanp/Husk?style=flat-square&color=EA580C"></a>
  <a href="https://github.com/xishanp/Husk/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/xishanp/Husk/actions/workflows/ci.yml/badge.svg?branch=main&style=flat-square"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/xishanp/Husk?style=flat-square&color=2563EB"></a>
  <img alt="Platform: Android arm64" src="https://img.shields.io/badge/Platform-Android%20arm64-3DDC84?style=flat-square&logo=android&logoColor=white">
  <img alt="Rust 1.75+" src="https://img.shields.io/badge/Rust-1.75%2B-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="CONTRIBUTING.md"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-0F766E?style=flat-square"></a>
</p>

## 概览

让 Linux 用户态以普通应用进程的身份，直接跑在 Android 内核上。

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

- [概览](#概览) · [能跑起来的前提](#能跑起来的前提) · [为什么是 Husk](#为什么是-husk)
- [和容器、虚拟机的区别](#和容器虚拟机的区别)
- [硬件接口](#硬件接口) · [特性](#特性) · [快速开始](#快速开始)
- [工作原理](#工作原理) · [性能](#性能) · [兼容性](#兼容性)
- [从源码构建](#从源码构建) · [仓库结构](#仓库结构) · [文档](#文档)
- [常见问题](#常见问题) · [参与贡献](#参与贡献) · [许可](#许可)

## 为什么是 Husk

同一台手机、同一份 rootfs、同一个 workload，只换运行器。2026-11-24 的 5 轮轮转中位数见
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)：

| 工作量 | Husk | proot | 倍率 |
|---|---:|---:|---:|
| `stat` × 20 000 | 650 ms | 3 734 ms | **5.7×** |
| `open` + `close` × 20 000 | 1 252 ms | 8 481 ms | **6.8×** |
| `make -j4`，200 目标链接 | 5 194 ms | 24 182 ms | **4.7×** |
| 顺序写 256 MB | 1 294 ms | 1 321 ms | 1.0× |

2026-11-27 在最终发行树上又做了一次独立现场探针，确认优势方向仍在路径密集型调用；这组数字
是单次采样，不和上面的 5 轮表混算：

| 工作量 | Husk | PRoot | 结果 |
|---|---:|---:|---|
| `stat` × 20 000 | 741.2 ms | 2 338.7 ms | Husk 快 3.16× |
| `open` + `close` × 20 000 | 980.9 ms | 5 676.3 ms | Husk 快 5.79× |
| shell 循环 × 1 000 | 18.2 ms | 51.5 ms | Husk 快 2.84× |
| `/bin/true` × 50 | 708.5 ms | 323.6 ms | **Husk 慢 2.19×** |

最后一行的代价来自每次短进程启动都要重新装载并完成 GOT 改写；它不是 syscall 拦截，但仍是
真实的启动成本。大块顺序 I/O 也不占优：数据成块进内核时，两种运行器都没有可翻译的路径。

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

Husk 不按应用逐个开洞。能直接使用的内核节点就直接交给 guest；必须经过 Android 应用身份的
能力，则提供标准 Linux 接口，并明确标注为 facade 或中转。

| 能力 | 接法 | 用户态接口 | 当前验收 |
|---|---|---|---|
| GPU | **直连** | `/dev/mali0`，PanVK + Zink | Mali-G77 枚举、compute、离屏 graphics、EGL/GL 均通过；默认环境由 loader 注入，登录与非登录进程一致。Adreno/KGSL 有适配代码，但没有真机证据，状态为 UNKNOWN |
| 相机 | **中转** | 标准 V4L2 `/dev/videoN` facade | 前后摄可启动；`read`、`readv`、`mmap`、拍照和录像通过。`open()` 返回 shim 的 memfd，不是真实 V4L2 内核 fd；DMABUF 与 controls 未支持 |
| 音频 | **中转** | PulseAudio → Rust relay → AAudio | 输出与采集均有真实 PCM 数据；`/dev/snd` 在应用域不可直接打开 |
| 视频解码 | **中转** | VA-API → Rust bridge → MediaCodec | H.264 1080p30 连续 600 秒，`dropped=0` |
| 传感器 | **中转** | IIO sysfs 影子树 ← NDK `ASensorManager_*` | 加速度、磁力、陀螺仪、环境光、距离五类可读，数值随真实事件更新 |
| 显示 | **应用侧中转** | `AHardwareBuffer` → dma-buf → `SurfaceControl` | 3-buffer ring 与真实 `SYNC_FD` 跑过 10,000 帧；这一段是应用侧零拷贝 |
| 输入 | **窗口层** | Android 输入事件 → 宿主 window/X11 | bridge 已实现；端到端结果取决于宿主是否提供窗口系统 |

口径必须写清：**GPU 是直连，相机、音频、视频和传感器是中转，显示只在其应用侧接管段为零拷贝。**
定位、USB、蓝牙属于中转，但还需要最小 Java 回调垫片；NFC 与指纹对 Linux 用户态不是必需的
设备接口。逐项理由、被宿主拒绝时的原始错误串和“不能说什么”见
[`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md)。

GPU 默认值属于运行时接口的一部分。ICD、GL 驱动和 GLES 上限写在 `/etc/husk/gpu-env`，由
loader 在程序没有显式设置时注入，因此登录 shell、非登录进程和 `env -i` 看到同一套渲染器；
程序自己的环境变量永远优先。Mali/kbase 的 PanVK 快照随发行包安装，安装器会校验设备节点与
驱动 SHA-256。Adreno/KGSL 目前没有真机证据，不能写成已经支持。

同一份 `gl-probe`、同一台参考机上的 GPU 与 llvmpipe 对比：

| 负载 | GPU | 软件 | 结果 |
|---|---:|---:|---|
| 2400×1080，每像素 48 次 `sin`，2 遍 | 23.8 ms/帧（41.9 fps） | 266.7 ms/帧（3.7 fps） | **GPU 快 11.2×** |
| 1280×720，同一负载 | 14.9 ms/帧（67.1 fps） | 101.9 ms/帧（9.8 fps） | **GPU 快 6.8×** |

窗口内容进入宿主 X server 仍是回读与 SHM 拷贝，DRI3 在本机应用域的结构性限制下不可达；
这段不属于 Linux 用户态运行时，也不能宣传成零拷贝。详见
[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) §7。

## 特性

- **零 `ptrace`、零 syscall 拦截。** 内核看到的是用户态自己的 syscall（evidence 130–146）。
- **原样的 glibc 用户态。** 除 libc 内部调用需要的 slot 项，树的二进制没有为 Husk 改过。
- **包管理器可用。** `apt-get install` 27 秒把树从 815 包装到 882 包，含 `build-essential`、
  `nodejs`、`python3-numpy`（evidence 154）。
- **工具链完整。** `gcc`、200 目标工程的 `make -j4`、`git clone`、`python3` + `numpy`
  （evidence 155）。
- **系统服务可用。** dbus、polkit、at-spi、gvfs 与 RandR 能在 Linux 用户态里直接启动；
  它们是否需要桌面会话由宿主决定（evidence 151、152）。
- **不需要任何特权。** 跑在普通应用沙箱里，机器可以是原厂状态。
- **GPU 默认值在加载器里，不在登录脚本里。** 非登录执行、嵌套 `exec` 和 `env -i`
  也不会掉回软件渲染；程序自己设的变量永远优先（evidence 182）。
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
| [`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md) | 每类硬件怎么接：直连 / 中转 / 不需要 / 权限或厂商限制 |
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
