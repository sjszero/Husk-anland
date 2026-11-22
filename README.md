# Husk（壳）

**Husk 是一个 Android 运行时，让完整的 Linux 发行版（glibc 用户态）以普通应用进程的身份，
直接由 Android 内核执行——不需要 root、不需要解锁 Bootloader、不需要硬件虚拟化。**

它不拦截系统调用，因此性能与原生应用处于同一量级。

> 当前状态：**运行时核心已验证；SoC 直连（GPU / 显示 / 视频 / 音频）进行中；与 LinuxHub 2.0 的集成尚未开始。**
> 2026-11-22 真机（RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页 / 普通应用权限 `untrusted_app`）：
> shell、常用工具、gcc 13.3 + make 全链路通过；2.0 的完整 Ubuntu 24.04 rootfs 能起 GNOME Shell 46；
> 生命周期（旋转 / 切后台 / Surface 重建 / 会话不重启）全部实测通过；
> SoC 侧 guest 进程可直接打开 `/dev/mali0`，分配 GPU 显存并在 CPU 侧读写（kbase 11.38）。
> GPU 最新证据：FristOneRR PanVK 的 glibc 构建已在 guest 内枚举 Mali-G77 MC9；纯 Rust 探针
> 已提交 compute job、完成离屏 graphics 三角形 draw + GPU 像素回读；发行版 Zink 也已在
> surfaceless EGL 报出 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。同一 guest AHB 已通过
> 平台 SurfaceView 的 BLAST SurfaceControl 直接提交给 SurfaceFlinger；3-buffer AHB ring 已连续
> 上屏 12 帧，并按 SF release fence 回收 slot。producer/acquire fence、WSI、资源池和窗口生命周期
> 仍未验收（见 evidence 53 / 54 与 `docs/PROJECT-MEMORY.md`）。

---

## 1. 它是什么

Husk 是**运行时**，不是终端模拟器：终端只是它的一个前端。

用户在应用里选择一个发行版（例如 Ubuntu），该发行版的用户态——shell、常用命令、
包管理、编译工具链——就以当前应用进程的身份运行。**一个发行版实例称为一个 husk。**

本仓库自带这个运行时目前唯一的载体：`app/` 里的 Android 应用。

名字取自"壳"：我们加在 Linux 用户态外面的唯一东西就是一层壳，用来适配 Android 的运行环境，
而不改变它本身的执行方式。

## 2. 它在哪里运行

运行载体是 **LinuxHub 应用本身**：一个普通的 Android APK，不需要任何额外环境、辅助应用或权限。

- 应用在自己的进程里启动 guest 的动态链接器，由它加载 guest 程序；
- guest 的文件系统位于应用的私有存储目录内，应用卸载即随之清除；
- 用户看到的是应用里的一个会话窗口，而不是一个独立的操作系统。

`app/` 把 guest 的 `bash` 跑在一个真正的 PTY 里，这是目前唯一的用户入口。
壳应用本身没有 Java 也没有 C：APK 声明 `android.app.NativeActivity`，入口直接落在
`app-rs/` 编出来的 Rust 库里（PTY、EGL 渲染、字体光栅化、输入、生命周期全在里面）。
界面仍是演示级——提示符渲染与滚动回看尚未完成。
`tools/probe-app` 是最小探针，只用于设备测试。

## 3. 它解决什么问题

在 Android 上运行 Linux 的既有做法都有明显代价：

| 做法 | 代价 |
|---|---|
| 系统调用重定向类方案（以 ptrace 实现） | 每个系统调用一次上下文切换，一次目录遍历就是数万次往返，整体慢一个数量级 |
| 容器类方案（chroot / namespace） | 需要 root 权限，普通应用不可用 |
| 虚拟机 | 需要硬件虚拟化支持 |

Husk 的取舍是：**运行时不做任何系统调用拦截。**

需要在启动前确定的适配——路径视角的转换、以及 Android 安全策略会拒绝的两个启动期系统调用——
全部在**构建期**写入 guest 的动态链接器；运行时由内核直接执行 guest 代码。
这是它能接近原生性能的根本原因。

## 4. 工作方式

1. **路径**：guest 视角的绝对路径（如 `/etc/os-release`）在进入系统调用之前，被解析为应用
   私有目录下的真实路径。符号链接与 `..` 均在 rootfs 边界内解析，无法逃逸。
2. **启动期安全策略**：Android 对普通应用进程禁用两个启动期系统调用，而 guest 的 C 运行库
   在动态链接器内部就会使用它们。这两处调用在构建期被移除，并由受控的替代路径补齐，
   因此启动阶段不需要任何拦截机制。
3. **子进程**：启动新程序时沿用同一个动态链接器，子进程自动具备相同行为，无需环境变量或包装脚本。

实现细节与边界情况见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 5. 实测表现

真机环境：**PLE110 / Android 16 / kernel 6.6**（历史测值；当前目标机 RMX3357 / Android 13 / 4.19.191 尚未重测性能表）。guest 为 Ubuntu 24.04 / glibc 2.39。

| 指标 | Husk | 对照 |
|---|---|---|
| 启动一个程序（fork + exec + wait） | **2.5 ms** | 同进程内 Android 原生程序 6.8–8.0 ms |
| `getpid` 等无路径参数的调用 | **196 ns** | 与原生一致 |
| `stat`，单层路径 | **5.4 µs** | 同一路径不经解析 2.9 µs |
| `stat`，四层路径 | **11 µs** | — |

路径解析是唯一存在额外成本的环节；其余系统调用没有额外开销。
完整数据与来源见 [docs/RESULTS.md](docs/RESULTS.md)。

## 6. 当前能力

- **Shell 与常用工具**：bash、ls、find、cp、mv、chmod、sort、sha256sum、管道、重定向、子 shell
- **编译工具链**：gcc 13.3 全链路（预处理、编译、汇编、链接）与 make（并行、增量、clean）
- **兼容性验证**：31 项功能用例全部通过；11 项越界测试无逃逸
- **运行环境**：普通应用权限，无需 root 与开发者选项之外的任何配置
- **完整发行版**：2.0 的 Ubuntu 24.04.4 链（817 个 `usr/bin`、1.9 GB）可解到私有目录并起
  GNOME Shell 46.0 / gjs / dbus-daemon / Xwayland（evidence 33）
- **内核级兼容**：`#!` 脚本（解释器映射进 rootfs）、`/proc/self/exe` 返回 guest 路径（evidence 34）
- **SoC 底座**：guest 直开 `/dev/mali0`；kbase `VERSION_CHECK` / `SET_FLAGS` / `mem_alloc` /
  `mmap`（CPU 读写显存）实测通过（evidence 35 / 36）

## 7. 当前限制

- **GPU 窗口渲染部分打通**：FristOneRR PanVK 已枚举 Mali-G77，logical device、host-visible
  storage buffer、compute job、离屏 graphics draw + 像素回读均通过；发行版 Zink 的 surfaceless
  EGL 也已报出 Mali-G77 renderer；同一 AHB 已通过平台 SurfaceView SurfaceControl 零拷贝上屏，
  3-buffer ring 连续 12 帧并依据 release fence 复用（evidence 53 / 54）。但 producer/acquire
  fence、持久资源池、GBM / Wayland / X11、WSI / swapchain、旋转 / DPI / 多窗口和窗口生命周期
  尚未验证，不能把当前正确性探针当成完整 GPU 验收；DRM 语义层保留为后备；
- **显示 / 视频 / 音频桥进行中**：AHB dma-buf 已跨进程导入 PanVK 并由 GPU 访问，3-buffer ring
  已连续零拷贝上屏 12 帧并按 release fence 复用；producer/acquire fence、资源池、MediaCodec
  硬解回传、AAudio 通路都还没完成；
- `execl` 系列函数不生效；`freopen`、`glob`、`nftw`、`dlmopen` 未覆盖；
- **libc 内部调用**（NSS / locale / `/etc/hosts` / `ld.so.cache`）仍读宿主，槽位方案未实现，
  因此 `apt`/`dpkg`/locale 在完整 rootfs 里尚不可靠；
- 页大小：4K 设备实测；16K / 64K 只有构建期同余保证与 `check-elf` 体检，**未在 16K 设备上跑过**；
- 终端界面：提示符 / 光标 / 滚动回看 / 旋转 / 生命周期已可用；SGR 颜色、UTF-8 宽字符未做，
  且终端壳属演示载体，不作为运行时验收项；
- rootfs 靠 `adb` 手工导入，应用内尚无导入 / 选择 / 删除流程；
- 与 LinuxHub 2.0 的集成（替换 proot、消除 proot 补丁）未开始。

## 8. 仓库结构

| 路径 | 内容 |
|---|---|
| `app/` | APK 打包工程（manifest + gradle）；**全部逻辑在一个 Rust 动态库里** |
| `runtime/` | 运行时本体与构建工具（Rust workspace，`no_std` 路径层 + `nr-tools`） |
| `tools/loader/` | loader 手术：`init-hook.S`、构建脚本、Python 参考实现 |
| `tools/device-tests/` | 设备验收脚本（`run-as` 下跑矩阵、越界、gcc/make） |
| `tools/probe-app/` | 最小探针应用，只用于测试 |
| `binaries/` | 产物指纹文件（sha256 锚点） |
| `docs/` | 文档、设计与设备证据 |

## 9. 文档

| 文档 | 内容 |
|---|---|
| [架构](docs/ARCHITECTURE.md) | 分层、适配点、路径解析、已知边界 |
| [实测数据](docs/RESULTS.md) | 功能矩阵、越界测试、性能数据、对照与未验证项 |
| [工程设计](docs/ENGINEERING.md) | 组件划分、构建工具、产物指纹、语言构成 |
| [后续工作](docs/FUTURE-WORK.md) | GPU/音频桥、发行形态、厂商接口可行度、待验证前提 |
| [设计索引](docs/design/) | 设计文档与证据的入口 |
| [长期记忆](docs/PROJECT-MEMORY.md) | 接手手册：目标、硬约束、路线变更、参考仓库、操作纪律 |
| [运行时方案](docs/design/runtime.md) | 运行时架构、验收标准、剩余工作 |
| [目标](docs/GOALS.md) | 权威目标：已完成项、剩余工作、验收标准、回滚包指纹 |
| [状态](docs/STATUS.md) | 逐日进度、修掉的 bug、待办 |
| [生命周期规格](docs/design/lifecycle.md) | 会话 / PTY / fd / 信号 / 关停的完整契约与不变量 |
| [libc 内部调用](docs/design/libc-interpose.md) | 槽位方案设计（未实现） |
| [证据](docs/design/evidence/) | 01–50：设备原始输出，与方案引用一一对应 |
| [参考仓库原始清单](docs/design/reference-request-20261122.md) | 用户提供的 12 类、120 个条目，逐字保存 |
| [参考仓库审计](docs/design/references.md) | 真实路径、可用性、已拉取 commit 与借鉴结论 |
| [参考仓库逐条矩阵](docs/design/reference-audit-matrix.md) | 原始 120 条逐项对应：69 GitHub + 45 官方上游/组件 + 6 UNKNOWN |
| [参考仓库机器记录](docs/design/reference-audit-20261123.json) | 120 条审计快照：来源、级别、star、push 日期、状态 |
| [变更记录](CHANGELOG.md) | 版本与已完成事项 |

开发者上手请见 [QUICKSTART.md](QUICKSTART.md)。

## 10. SoC 直连现状（2026-11-23）

| 项 | 状态 |
|---|---|
| `/dev/mali0` | guest 可 rw 打开；kbase UAPI 11.38 |
| 命令面 | 26 条，取自厂商 `libGLES_mali.so`（r32p1）反汇编，不是猜 header |
| `JOB_SUBMIT` | `0x40108002`（W,16）：实测 EINVAL（命令可达，参数被拒），不是 ENOTTY |
| 显存 | `mem_alloc` + `mmap` 实测可分配并 CPU 读写（`prot` 必须等于 flags 低两位） |
| 上游 Mesa | 25.2 与 main 均**没有** kbase 后端；Ubuntu 25.2.8 panfrost 模型表也无 G77/G78 |
| 当前 GPU 路线 | FristOneRR PanVK（Mesa 26.3.0-devel，glibc 构建）+ 发行版 Zink；DRM 语义层保留为后备 |
| PanVK / Zink 验证 | `vulkaninfo` + 纯 Rust probe：Mali-G77 / logical device / compute job / 离屏 graphics draw + GPU 像素回读 PASS；Zink surfaceless renderer 也报 Mali-G77；Android 窗口 3-buffer ring 连续 12 帧零拷贝上屏，acquire fence / WSI 未验收 |
| 显示 / 视频 / 音频 | 显示已完成 AHB→dmabuf→guest PanVK 导入、GPU 访问、3-buffer ring 连续 12 帧零拷贝上屏和 release-fence 复用（[evidence 49](docs/design/evidence/49-ahb-dmabuf-handle.md)、[evidence 50](docs/design/evidence/50-guest-dmabuf-panvk-import.md)、[evidence 53](docs/design/evidence/53-surfacecontrol-zero-copy.md)、[evidence 54](docs/design/evidence/54-surfacecontrol-ring.md)）；acquire fence / 资源池 / 视频 / 音频未完成 |

细节与原始输出：[evidence 35](docs/design/evidence/35-gpu-ioctl-surface-from-vendor-driver.md)、
[evidence 36](docs/design/evidence/36-gpu-route-reality-check.md)、
[evidence 45](docs/design/evidence/45-panfrost-loads-over-kbase.md)、
[evidence 46](docs/design/evidence/46-panvk-fristonerr-kbase-g77.md)、
[evidence 47](docs/design/evidence/47-zink-eglinfo-surfaceless.md)、
[evidence 48](docs/design/evidence/48-panvk-extension-scan.md)、
[evidence 49](docs/design/evidence/49-ahb-dmabuf-handle.md)、
[evidence 50](docs/design/evidence/50-guest-dmabuf-panvk-import.md)、
[evidence 53](docs/design/evidence/53-surfacecontrol-zero-copy.md)、
[evidence 54](docs/design/evidence/54-surfacecontrol-ring.md)。

## 11. 许可与来源

路径适配的对象是 Ubuntu 24.04 的 glibc 二进制（LGPL），
其衍生文件仅用于固定被测产物的身份，来源与许可说明见 [LICENSE](LICENSE)。
