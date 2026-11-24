<!-- 首屏：标题一行，图标下一格。GitHub 只在空行处结束 HTML 块，两段必须分开，
     否则标题会退回左对齐。 -->
<h1 align="center">Husk</h1>

<p align="center">
  <img src="docs/assets/brand/husk-mark-hexagon-h-512.png" width="128" alt="Husk">
</p>

<p align="center">让 Linux 用户态以普通应用进程的身份，直接跑在 Android 内核上。</p>

<p align="center">
  <a href="https://github.com/xishanp/Husk/releases"><img alt="Release" src="https://img.shields.io/github/v/release/xishanp/Husk?style=flat-square&color=EA580C"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/xishanp/Husk?style=flat-square&color=2563EB"></a>
  <img alt="Platform: Android arm64" src="https://img.shields.io/badge/Platform-Android%20arm64-3DDC84?style=flat-square&logo=android&logoColor=white">
  <img alt="Rust 1.75+" src="https://img.shields.io/badge/Rust-1.75%2B-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="CONTRIBUTING.md"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-0F766E?style=flat-square"></a>
</p>

Husk 是一个 Android 上的 **Linux 运行环境**，它属于容器与虚拟机之外的第三种做法：不建
namespace、不跑虚拟机、不做 `ptrace`，而是让真实的 Linux 用户态（glibc、包管理器、编译器、
shell、服务）作为普通 `untrusted_app` 进程直接跑在 Android 内核上。路径虚拟化在装载时做一次
（改写每个对象的 GOT），之后程序自己的 `svc` 指令原样进内核——所以每条 syscall 的成本和原生
应用一样，而沙箱允许的设备**可以直接开到硬件**。

运行时不绑定某一个发行版或某一台设备：装载器按 glibc 版本断言补丁点。我们完整验证时用的
**参考发行版**是 Ubuntu 24.04（glibc 2.39），**参考设备**是一台 arm64 Android 手机
（型号与内核写在 `docs/STATUS.md`，那只是实测条件，不是要求）。

## 目录

- [为什么是 Husk](#为什么是-husk) · [和容器、虚拟机的区别](#和容器虚拟机的区别)
- [硬件接口](#硬件接口) · [特性](#特性) · [快速开始](#快速开始)
- [工作原理](#工作原理) · [性能](#性能) · [兼容性](#兼容性)
- [限制](#限制) · [从源码构建](#从源码构建)
- [仓库结构](#仓库结构) · [文档](#文档)
- [常见问题](#常见问题) · [参与贡献](#参与贡献) · [许可](#许可)

## 为什么是 Husk

同一台手机、同一份 rootfs、同一个 shell，只换运行器。5 轮中位数，2026-11-24 实测
（[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)）：

| 工作量 | Husk | proot | 倍率 |
|---|---:|---:|---:|
| `open` + `close` × 20 000 | 1 252 ms | 8 481 ms | **6.8×** |
| `make -j4`，200 目标链接 | 5 194 ms | 24 182 ms | **4.7×** |
| 顺序写 256 MB | 1 294 ms | 1 321 ms | 1.0× |

前两行是路径与 syscall 密集的活，要付 proot 每条 syscall 的 `ptrace` 往返；第三行是大块写，
不经过被翻译的路径，所以打平——表里照写，不藏。

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

## 硬件接口

这是 Husk 和翻译层方案最实质的区别：**用户态进程拿到的文件描述符是真的**，能直开的设备节点
就直开，被 Android 沙箱拒绝的那部分（需要应用身份、需要 Surface、需要 MediaCodec 的）由
应用内的硬件后端提供接口，边界写在
[`docs/design/husk-backend-interface.md`](docs/design/husk-backend-interface.md)。

| 能力 | 用户态里的路径 | 实测 |
|---|---|---|
| GPU（Mali-G77） | 直开 `/dev/mali0`，PanVK 后端 | logical device、compute、离屏 graphics、Zink surfaceless 全通（evidence 97） |
| 显示 | `AHardwareBuffer` → dma-buf → `SurfaceControl`，零拷贝 | 3-buffer ring + 真实 `SYNC_FD`：submitted=10 000 / completed=10 000（evidence 59） |
| 视频解码 | MediaCodec `c2.mtk.avc.decoder` + AHB 三平面 | H.264 1080p30 连续 600 秒，`dropped=0`（evidence 80） |
| 相机 | V4L2 `/dev/videoN`（前端为 Camera2 + `AImageReader`） | 拍照 JPEG + 30 秒录制，硬件 H.264 编码（evidence 93） |
| 音频 | PulseAudio → Rust relay → AAudio | 48 kHz 双声道连续会话通过（evidence 62，10 秒级，不是长稳） |
| 输入 | 触摸事件注入到用户态的窗口系统 | 触摸 → JNI → XI2，点哪落哪（evidence 148） |

这张表的口径：**能直开的不经翻译，必须经应用身份的走一条明确的 socket 协议**，两边都不是
软件模拟出来的假设备。没做到的写在 [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)。

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
- **对比可以自己重跑。** `tools/bench/` 一条命令把同一份 rootfs 交给三个运行器，
  原始日志在仓库里。

## 快速开始

交付形态和 proot 方案一样：**把运行时作为 assets 打进 APP**，首次使用时解压到应用私有目录。

```text
APK assets/husk/arm64-v8a/
  libld.so          loader（内嵌 path-shim）
  libc.so.6         打过两处补丁的 glibc
  husk-ldconfig     写 ld.so.cache 与 ld.so.husk-path
  husk-usermgr      useradd / groupadd / passwd 一族替身
  husk-su           提权替身
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

参考环境下验证过的：`bash`、coreutils、`git`、`python3`（+`numpy`）、`gcc`、`make`、
`vim`、`nano`、`node`、`apt`、`dpkg`、dbus、polkit、at-spi、gvfs。发行版层面完整验证过的是
参考发行版 Ubuntu 24.04 / glibc 2.39；逐项矩阵见
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)。

## 限制

- ⚠️ 一个 ABI（arm64）；loader 的补丁带 glibc 版本断言，换发行版会直接报错，不会半好不坏
  地跑起来。
- ⚠️ stock rootfs 需要准备：libc 打补丁、绝对符号链接改相对、用 `husk-ldconfig`
  替换静态 `ldconfig`。
- 传感器、GPS、USB、蓝牙、NFC、指纹、硬件视频解码均未实现。Termux 不是支持的环境。

完整清单与每条的证据：[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)。

## 从源码构建

```sh
# loader（需要 NDK；会断言它补丁的 glibc 版本）
NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin \
  sh tools/loader/build-embedded-loader.sh

# Rust crates（no_std shim + 宿主工具）
cargo build --release --manifest-path runtime/Cargo.toml

# CI 里跑的检查
make check   # fmt + aarch64 cargo check + clippy + shell 语法 + 文档链接
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
| [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) | 什么不工作，为什么 |
| [`docs/STATUS.md`](docs/STATUS.md) | 目标机上的当前状态 |
| [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md) | 1.0 之前的工程日记 |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | 怎么构建、怎么测、术语表 |

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

**能在 Termux 里跑吗？** 不能，也不支持。用户态需要的二进制与库 Termux 的 `$PREFIX`
不提供。

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
