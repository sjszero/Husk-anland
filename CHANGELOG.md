# 更新日志

本文件记录所有值得公开的变更。格式遵循
[Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本 2.0.0](https://semver.org/lang/zh-CN/)。

**每条结论都带证据编号**，原始输出在 `docs/design/evidence/`。0.0.1 之前的工程日记（36 条，
按时间顺序）在 [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md)。

## [0.0.25] - 2026-11-26

> 主题：**修掉 DRM / Adreno 假 fd 被 `dup2`、`dup3` 覆盖后留下的旧表项**。
> 这一版只改 fd 身份记账，不改变 GPU 渲染后端，也不宣称 Adreno 已真机验证。

### 修复

- **`dup2`/`dup3` 覆盖假 DRM fd 时先清理目标表项**（evidence 251）。即使源 fd 是普通文件，
  旧目标也必须从 `FAKE_FDS` 移除；否则普通文件上的 DRM ioctl 会被错误送进 `DRM_CTX`。
- **Adreno/KGSL 表采用同一规则**（evidence 251）。`ADRENO_FDS` 在判断源 fd 前先清掉目标号码，
  避免关闭或复用后留下伪设备身份。该分支目前只有静态审计，没有 Adreno 真机动态证据。
- **新增 DRM fd 生命周期探针并接入门禁 A**（evidence 251）。`drmfake` 覆盖普通文件覆盖假 fd、
  假 fd 复制到新 fd 两个方向；旧 loader 负对照稳定返回错误的 `Ok(())`，修后返回普通失败。
- **清洁工作树的发布构建可重复**（evidence 252）。`make dist` 不再默默复制不存在的
  `libc-nop.so.6`；缺少生成物时明确要求 `LIBC=<pristine glibc 2.39 libc.so.6>`，
  并从该输入重建后再打包。

### 验证

- 旧 loader 负对照：`dup2-ordinary-over-fake|FAIL|ioctl=Ok(())`、
  `dup3-ordinary-over-fake|FAIL|ioctl=Ok(())`。
- 新 loader 真机：两个覆盖方向均为 `PASS`，`dup2-fake-preserved|PASS`。
- 完整门禁 A：`PASS=38 FAIL=0`；coverage-probe 全部 PASS；coverage-sweep 扫描 1401 条候选路径；
  V4L2 与 DRM fd 回归均通过。
- `make check` 通过，shell 语法检查 71 个脚本全部通过。
- clean tree 用固定 stock libc 重建 `libc-nop.so.6` 成功，`make dist` 产出发布包。

### 边界

- 本版探针验证 Mali-G77 上的假 DRM fd；Adreno/KGSL 同等路径仍是 UNKNOWN。
- `SCM_RIGHTS` 跨进程传递假 DRM fd 的状态恢复仍未实现。
- 多线程同时对同一 fd 做 `dup2`/`dup3`/`close` 的长时压力仍是 UNKNOWN。

## [0.0.24] - 2026-11-26

> 主题：**修掉 V4L2 假相机 fd 的高位复制生命周期，并把相机上下文的所有访问串行化**。
> 这一版只改假设备的状态管理，不把合成 `camera.sock` 探针写成真实摄像头画质验收。

### 修复

- **V4L2 引用计数不再写死 fd < 1024**（evidence 250）。`refs()` 现在枚举 `/proc/self/fd`，
  `dup()` 到 1100 等高位 fd 后，关闭原 fd 不会提前拆掉仍被引用的 camera socket。
- **`/proc/self/fd` 读取失败时 fail closed**（evidence 250）。无法确认最后引用时保留设备状态，
  避免误拆仍在使用的 socket；代价是极端情况下宁可泄漏状态，也不造成活 fd 静默失效。
- **V4L2 上下文加锁**（evidence 250）。path-shim 与 preload 两层对 socket、设备、buffer 表、
  rootfs 初始化和 ABI 转发的访问统一串行化，修掉 lazy init 之后的并发数据竞争。
- **新增 Rust 回归探针**（evidence 250）。`v4l2-ref-probe` 在 guest 内启动假 `camera.sock`，
  覆盖 `open -> dup(1100) -> close(original) -> QUERYCAP(high) -> close(high)`，并接入门禁 A。

### 验证

- 修前负对照：高 fd `QUERYCAP` 失败，socket 被提前拆掉。
- 修后真机：`original-close|socket=alive`、`ioctl-high|rc=0|driver=husk`、
  `last-close|socket=eof`、`V4L2_REF|PASS`。
- 完整门禁 A：`PASS=38 FAIL=0`；coverage-probe 全部 PASS；coverage-sweep 扫描 1400 条候选路径无泄漏。
- `make check` 通过。

### 边界

- 本版探针验证的是假 fd 生命周期与 `camera.sock` 握手，不是真实 Camera2 连续取帧。
- 真实相机高 fd 下的 STREAMON/DQBUF 组合、长时并发压力仍是 UNKNOWN。
- 通过 `SCM_RIGHTS` 把 V4L2 memfd 送到另一进程后的跨进程状态恢复仍未实现。

## [0.0.23] - 2026-11-26

> 主题：**让假 netlink socket 在 `dup`、`dup2`、`fcntl(F_DUPFD*)` 后仍保持同一语义**。
> 这一版修 fd 身份与生命周期记账，不改网络数据面，也不把 netlink 模拟扩展成完整内核实现。

### 修复

- **netlink fd 改为按 socket inode 识别**（evidence 249）。旧表只记原始 client fd 号，
  `dup()` 出来的副本会被当成普通 `AF_UNIX` socket；关闭原 fd 时还会把共享 peer 一并拆掉。
  现在 `dup`、`dup2`、`fcntl(F_DUPFD_CLOEXEC)` 都能继续得到 `AF_NETLINK` 语义。
- **按 live 引用关闭私有 peer**（evidence 249）。同一 client inode 还有复制 fd 时，槽位和 peer
  保持存活；只有最后一个引用关闭后才统一清理。peer fd 号复用时还要通过 inode 校验，避免误关。
- **共享 live-fd 上界实现**（evidence 249）。`close_range` 与 netlink 引用计数共用
  `/proc/self/fd` 枚举，不把 1024 当作硬上限。

### 验证

- 真机输出：`families=16,16,16,16`，逐级关闭后仍有 `AF_NETLINK`，最后一个引用关闭后 fd 回到基线。
- `close_range(3,~0U)` 子进程仍能通过 guest 路径创建文件，root fd 保留。
- coverage-probe 全部 PASS，门禁 A `PASS=38 FAIL=0`，`cargo check` 通过。

### 边界

- 通过 `SCM_RIGHTS` 传到另一进程的假 netlink fd 尚无跨进程槽位，仍是 UNKNOWN。
- 多线程同时 dup/close 同一 netlink inode 的极端竞态未做专项压力测试。
- `CLOSE_RANGE_UNSHARE` 仍明确返回 `EOPNOTSUPP`，本版没有改变这个边界。

## [0.0.22] - 2026-11-26

> 主题：**让 `close_range(2)` 在 Android app 域真正可用，并补齐批量关闭时的私有 fd 记账**。
> 这一版修的是运行时接口，不改 X11/GPU 呈现路径，也不把 fd-table unshare 说成已支持。

### 修复

- **`close_range` 不再被 seccomp 的 ENOSYS 卡死**（evidence 248）。Android 13 的 app seccomp
  拦截 syscall 436，旧实现把 SIGSYS 回成 `ENOSYS` 后，glibc 没有回退，调用方直接失败。
  现在 path-shim 枚举 `/proc/self/fd`，按真实 fd 上界执行原始 `close(57)`；`3..UINT_MAX`
  不会遍历四十多亿个编号，同时跳过内部 root fd。
- **`CLOSE_RANGE_CLOEXEC` 与 `UNSHARE` 分清语义**（evidence 248）。前者只设置 `FD_CLOEXEC`，
  不注销仍在使用的假 fd；后者无法在用户态正确模拟，明确返回 `EOPNOTSUPP`，不伪造成功。
- **批量关闭统一清理 V4L2 / netlink / DRM / Adreno 记账**（evidence 248）。单 fd 与批量路径
  共用 `forget_fake_fd()`；netlink 槽位记录 client / peer 的 socket inode，注销时只在 inode 匹配的前提下关闭私有 peer，既修掉 fd 泄漏，也避免 fd 号复用后误关无关 fd。
- **用户态路径入口复核**（evidence 247）。`realpath`、`canonicalize_file_name`、`glob`、
  `nftw`、`statvfs` 在当前 loader 下均落到 guest 路径；这是覆盖复核，不是新增五项代码修复。

### 验证

- coverage-probe 真机输出：netlink `before=6 opened=8 after=6`；pipe 两 fd 均关闭；
  `CLOEXEC` 两 fd 均保留且 `F_GETFD=1`；`close_range(3,~0U)` 子进程仍能通过 guest 路径创建文件。
- 同一枚 loader 跑完整门禁 A：`PASS=38 FAIL=0`；coverage-probe 全部 PASS；
  coverage-sweep 扫描 1400 条候选路径，未发现未翻译的 guest 绝对路径。
- `cargo check --target aarch64-unknown-linux-gnu -p path-shim` 通过，loader interpose 表为 199 项。

### 边界

- `CLOSE_RANGE_UNSHARE` 仍不支持；依赖 fd-table unshare 的调用会得到 `EOPNOTSUPP`。
- 多线程并发 fd churn 没有做压力测试；V4L2 相机与 Adreno 真机上的 `close_range` 仍是 UNKNOWN。
- 本版没有改变“X11 窗口内容进入 X server 仍是一次 SHM 回读加拷贝”的边界。

## [0.0.21] - 2026-11-26

> 主题：**修掉 stdio 写入后硬链接复制后备不刷新的兼容性缺口**。这一版只改 path-shim 的
> `fclose` 处理，不把 emulated hard link 说成真正的内核硬链接。

### 修复

- **stdio `fclose` 刷新硬链接别名**（evidence 246）。Android app 数据目录拒绝 `link(2)`，
  Husk 用复制兜底并登记源 inode。旧实现只钩 `close(2)`，而 glibc 的 `fclose` 内部走
  `__close_nocancel`，所以 `fopen`/`fputs`/`fclose` 写入后别名文件仍停留在旧内容。
  现在导出 `fclose`：先 flush 流，再用 `fileno` 找到源 fd，刷新别名，最后调用真实 libc
  `fclose`；真实函数从 link map 解析并跳过 shim 自己，避免递归。

### 验证

- coverage-probe 真机新增 `HARDLINK_STDIO`，输出
  `link_rc=0 link_errno=0 content=AAABBB`。
- 同一枚 loader 跑完整门禁 A：`PASS=38 FAIL=0`；coverage-probe 全部 PASS；
  coverage-sweep 因 rootfs 没有 `strace` 按规则记为 `SKIP`。
- `make check` 全绿；`make test` 仍是 host-only 目标，macOS 的 aarch64 Linux 链接参数失败，
  照实记录，不写成通过。

### 边界

- 仍然不是内核级硬链接：`st_nlink`、共享 inode、跨进程即时共享都没有恢复。
- 只覆盖经导出 `fclose()` 的 stdio 路径；绕过该入口的 glibc 内部关闭路径仍是 UNKNOWN。
- 本版没有改 2.0 源码，没有改变 X11/SHM 呈现路径，也没有宣称桌面端到端零拷贝。

## [0.0.20] - 2026-11-26

> 主题：**真实 X11 窗口里的 Zink/PanVK 路径已经跑通，并把“Husk GPU 路径”与“2.0 当前软件桌面”
> 明确分开**。这一版不把当前 2.0 GNOME 会话说成全局 GPU，也不把 `glxgears` 的数字冒充桌面性能。
> 原始输出见 evidence 245。

### 验证

- **真实 X11 硬件 GLX**（evidence 245）。guest 内 `glxinfo -B` 报 `Accelerated: yes`，
  renderer 为 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；进程实际持有两个 `/dev/mali0`
  fd，并加载 `libvulkan_panfrost_x11.so` 与 `libGLX_mesa.so`。
- **`glxgears` 连续 600 秒**（evidence 245）。在 2400×1080 的现有 X11 桌面上跑了 119 个 5 秒窗口，
  共 147872 帧，215.779–299.303 FPS，平均 248.352 FPS；最后 `RC=124` 是外部 `timeout` 到点，
  不是程序崩溃。
- **当前 2.0 会话的环境边界**（evidence 245）。由 2.0 拉起的 `gnome-shell` 仍显式带有
  `GALLIUM_DRIVER=llvmpipe`、`MESA_LOADER_DRIVER_OVERRIDE=swrast`、
  `LIBGL_ALWAYS_SOFTWARE=1`，应用日志约为 1.0–1.2 FPS、`0/0 present copies offloaded to GPU`。

### 边界

- 当前 2.0 GNOME 不是 GPU 桌面。2.0 的 `linuxhub-sessiond` 仍有意把 Mutter 固定在 llvmpipe；
  改成硬件会话后的稳定性、帧率、主题/窗口损坏情况都还是 UNKNOWN。
- X11 窗口内容进入 X server 仍是 SHM 回读加拷贝，不是端到端零拷贝（evidence 228）。
- `glxgears` 只验证窗口化 GLX 的简单 fixed-function 路径；硬件 GNOME、浏览器、视频、游戏、多应用并发和
  Adreno/KGSL 真机仍是 UNKNOWN。

## [0.0.19] - 2026-11-26

> 主题：**fixed-function OpenGL 的 PanVK/Bifrost GS 崩溃增加默认规避，并让 loader 默认值穿透
> 真实 `setenv` 与 `execve`**。这是运行时默认值修复，不是 PanVK 编译器修复。
> 原始输出见 evidence 244。

### 修复

- **Mali 默认关闭 PanVK 的 DXVK 兼容 GS**（evidence 244）。`gpu-env` crate、安装器和登录 shell
  都写入 `PANVK_DXVK_COMPAT=0`；已有配置只补缺失项，用户显式写 `1` 不会被迁移覆盖。
  当前 Bifrost 后端会在 fixed-function 转译产生的内部 geometry shader 上以 `fa=0x21` SIGSEGV。
- **`setenv` 遵守 loader 环境规则**（evidence 244）。path-shim 导出 `setenv`，fill 默认值在
  `overwrite=0` 时视为已存在，forced/hidden 规则继续生效；显式 `overwrite=1` 仍可覆盖 fill 值。
- **`execve` 物化真实 environ**（evidence 244）。子进程的 `envp` 会收到 fill/force/hide 规则，
  因此不经过 shim `getenv` 的库（例如 PanVK 构造器）也能看到同一套默认值。
- **新增 `gl-probe fixed`**（evidence 244）。不启动 X11，使用 surfaceless desktop GL 2.1 固定管线
  画齿轮，把 fixed-function 编译问题与窗口/呈现层分开。

### 验证

- 当前源码重建的 loader SHA256 为 `020ab832e69e1aaadfe81219006636fc21347558a0e56b599e166a2240515c7c`；
  真机默认环境 600 帧 `RESULT PASS`，588.5 fps，像素 `255,214,57,255`（evidence 244）。
- 显式 `PANVK_DXVK_COMPAT=1` 仍以 `segv|sig=11|fa=21|RC=139` 复现，证明默认关闭不是硬编码。
- ES3 纹理/MRT、Vulkan compute/graphics/buffer 回读回归通过；`env -i` 经 bash `exec` 后能看到
  canonical ICD、Zink、GLES 3.2 和 `PANVK_DXVK_COMPAT=0`（evidence 244）。

### 边界

- PanVK/Bifrost 的 GS 编译缺陷仍在；依赖 geometryShader 的 DXVK 程序需要显式设置
  `PANVK_DXVK_COMPAT=1`，当前默认不保证 DXVK 可用。
- 真实 X11 `glxgears`、GNOME 桌面长跑、多应用并发和其他 fixed-function 程序仍是 UNKNOWN。
- 窗口内容进入 X server 仍是 SHM 回读加拷贝，不是端到端零拷贝；Adreno/KGSL 仍无真机证据。

## [0.0.18] - 2026-11-26

> 主题：**GPU 默认值统一成独立 Rust crate，App 启动级补 Adreno/KGSL，软件回退不再被 loader 覆盖**。
> Adreno 只有代码和主机侧单测，没有真机证据；本版不把它写成已支持。
> 原始输出见 evidence 243。

### 新增与修复

- **GPU 默认值抽成纯 Rust `gpu-env` crate**（evidence 243）。App 的 canonical ICD、驱动探测、
  `/etc/husk/gpu-env` 原子写入和软件回退不再堆在 `term.rs`；初始 shell 环境和 loader 随后读取
  的配置文件由同一个 crate 生成。loader 仍只负责消费该文件，不依赖 App crate。
- **App 冷启动补 Adreno/KGSL 路径**（evidence 243）。只有 `/dev/kgsl-3d0` 真实可读写且 rootfs
  中存在 Freedreno/Turnip 驱动时，才生成 Adreno `gpu-env` 和 guest 相对 ICD；有原生
  `kgsl_dri.so`/`freedreno_dri.so` 时走 kgsl/freedreno，否则走 zink。驱动不存在时不生成假配置。
- **软件回退传递到 loader 层**（evidence 243）。`HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF`
  回退现在会把 `HUSK_GPU=llvmpipe` 一并交给子进程，避免 loader 从 `gpu-env` 再次强制
  `MESA_LOADER_DRIVER_OVERRIDE` / `GALLIUM_DRIVER`。
- **回归门禁**（evidence 243）。`make check` 增加 `gpu-env` 编译、clippy 和 5 个主机侧单元测试；
  最终 0.0.18 APK 在 Mali 真机上删除配置后冷启动重建，`env -i` 仍为
  `zink Vulkan 1.3(Mali-G77 MC9)`，`HUSK_GPU=llvmpipe` 仍正确回退。

### 边界

- Adreno/KGSL 没有真机，真实打开、Turnip 枚举、ioctl/mmap、X11 桌面和长跑均为 UNKNOWN。
- X11 窗口内容进入 X server 仍是 SHM 回读加拷贝，不是端到端零拷贝。
- `glxgears` 的 PanVK/Bifrost 编译器 SIGSEGV 没有在本次重测为已修。

## [0.0.17] - 2026-11-26

> 主题：**GPU 默认值从 App 初始 shell 下沉到 loader，`env -i` 也拿到 Mali 硬件渲染器**。
> 这一版只移动默认值所在层并增加 Mali 设备门控；不改变 X11 呈现和 GPU 长跑边界。
> 原始输出见 evidence 242。

### 修复

- **loader 级 GPU 默认值**（evidence 242）。App 在 `/dev/mali0` 真实可打开且 PanVK 驱动存在时，
  自动生成 guest `/etc/husk/gpu-env`，把 canonical ICD、EGL vendor、Zink、GLES 3.2 和
  GTK 默认值放到加载器读取的位置。经 Husk loader 启动的进程即使使用 `env -i`，也会得到同一套
  默认值。
- **增加 Mali 设备门控**（evidence 242）。App 不再仅因 rootfs 中存在 PanVK 文件就注入 Mali
  环境；只有 `/dev/mali0` 以读写方式可打开时才走该路径，避免把 Adreno/其他 GPU 强行改成
  Mali 配置。
- **配置修复不覆盖已正确的自定义内容**（evidence 242）。已有 `@GPU_FAMILY=mali-kbase` 和
  canonical ICD 的 `gpu-env` 保留；缺失、旧宿主路径或旧 glob manifest 才会被原子重写。
- **真机验证**（evidence 242）。删除 `etc/husk/gpu-env` 和 canonical ICD 后冷启动，两份文件
  自动重建；无 GPU 变量的 `env -i` 下 `gl-probe es3` 仍报
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` 并 `RESULT PASS`；
  `HUSK_GPU=llvmpipe` 时正确回退到 `llvmpipe`。

### 边界

- X11 窗口内容进入 X server 仍是 SHM 回读加拷贝，不是端到端零拷贝。
- `glxgears` 的 PanVK/Bifrost 编译器崩溃没有在本次重测为已修。
- Adreno/KGSL 和通用 DRM 的 loader 默认值仍没有对应真机证据，状态为 UNKNOWN。

## [0.0.16] - 2026-11-26

> 主题：**修正 PanVK ICD 的宿主路径污染，GPU 默认路径在真机不再 SIGSEGV**。0.0.15 的
> “独立树 PanVK 旧、探针崩溃”是现象描述，不是根因；本版用同一驱动的 A/B 复核更正。
> 原始输出见 evidence 241。

### 修复

- **PanVK ICD 改用 guest 相对 `library_path`**（evidence 241）。App 不再把现有
  `husk_*.json` 的宿主绝对路径直接交给 Vulkan loader；驱动存在时生成 canonical
  `usr/share/vulkan/icd.d/husk_panvk_guest.json`，内容固定指向 guest 里的
  `/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so`。驱动不存在时不设置
  Vulkan 变量，避免“manifest 存在、驱动缺失”的假硬件路径。
- **安装器和登录/session 脚本统一 canonical manifest**（evidence 241）。
  `install-husk.sh` 不再按 glob 顺序选第一个 ICD；`/etc/profile.d/husk-gpu.sh`、
  `husk-session.sh` 和设备 GNOME 测试脚本都使用同一份 guest 相对 manifest。
- **真机无手工环境变量验证**（evidence 241）。冷启动 `com.linuxhub.husk` 后，初始 guest
  shell 的 `/proc/<pid>/environ` 指向 canonical manifest；`vulkan-probe` 枚举
  `Mali-G77 MC9`，compute/graphics 通过；`gl-probe es3` 报
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` 且 `RESULT PASS`。

### 更正

- 0.0.15 的独立树 SIGSEGV 被更正为 **ICD `library_path` 使用宿主 `/data/data/...` 路径**
  导致的驱动定位失败，不是“驱动版本旧”。证据 240 保留原发布时现象，并已追加证据 241 的
  后记，不静默改写历史结论。
- **仍然不能写**：X11 窗口呈现是端到端零拷贝；`glxgears` 的 PanVK/Bifrost 编译器崩溃已修；
  所有 GPU 家族均已验证。本次只修 ICD 路径，不改 SHM 呈现、不扩大设备适配边界。

## [0.0.15] - 2026-11-26

> 主题：**相机进入默认路径，接口口径按源码和真机行为纠正**。这一版不新增“所有硬件都直连”
> 一类说法；相机给 guest 标准 V4L2 接口，但后端是 App 内 Camera2，因此明确归入中转。
> 最终构建和真机证据见 evidence 240。

### 修复

- **相机桥不再需要测试 marker**（evidence 240）。删掉 `files/camera` /
  `files/camera-guest` 后冷启动 App，`files/rootfs-full/run/husk/camera.sock` 仍自动出现；
  最终 0.0.15 构建上前后摄复跑落在 12.26–17.42 fps。相机桥现在优先使用
  `files/rootfs-full`，没有时回退
  `files/rootfs`，并重建 V4L2 影子树、清理旧 `c81:N` udev 项。权限被拒时 socket 返回明确错误，
  不再表现为没有相机路径。
- **App 初始终端补齐 GPU 默认值**（evidence 240）。`HUSK_GPU=llvmpipe` 和
  `NR_GPU_DEFAULT_OFF=1` 现在对 App 自己创建的初始会话生效；硬件路径补
  `MESA_GLES_VERSION_OVERRIDE=3.2`。真机 `/proc/<pid>/environ` 已核到
  `GALLIUM_DRIVER=zink`、PanVK ICD 和 GLES 3.2 override。
- **能力矩阵不再把相机写成缺失**（evidence 240）。相机状态为 `degraded`，后端写明
  `V4L2 /dev/videoN -> Camera2 NDK`；输入状态改为 X11 点击/键盘已验证，并保留
  `adb input swipe -> XI2` 为 UNKNOWN。

### 文档更正

- **相机不是真节点直连**（evidence 91、240）。guest `open(/dev/videoN)` 得到 shim memfd，
  帧由 App 内 Camera2 `AImageReader` 产生并经 socket 传输。硬件分类由旧口径
  “直连 2 类（GPU、相机）/ 中转 2 类（音频、视频）”更正为
  **直连 1 类（GPU）/ 中转 3 类（相机、音频、视频）**。
- 独立验证树 `com.linuxhub.husk` 的 PanVK 仍比 evidence 232 的集成树旧：本轮
  `gl-probe es3` 和 `vulkan-probe` 在 zink/PanVK 上 SIGSEGV，llvmpipe 对照通过。因此本版只
  确认初始会话的 GPU 环境注入，不宣称该独立树的 GPU 探针通过；集成树结果不据此重写。

## [0.0.14] - 2026-11-26

> 主题：**发布前补审**。0.0.13 发布后又对并发、会话生命周期、APK 元数据和构建入口做了一轮
> 实证审计；这一版只收已经复现或由构建链路直接证实的缺陷。没有设备验证的适配仍按 UNKNOWN 记录。
> 最终构建、安装和真机回归结果见 evidence 239。

### 增加

- **Adreno/KGSL GPU 适配器代码**（evidence 233）。按 `/dev/kgsl-3d0` 选择 `adreno-kgsl`；
  guest 的 `/dev/dri/renderD128` 由真实 KGSL fd 承载，`DRM_IOCTL_VERSION` 返回 `kgsl`，
  KGSL 的其余 ioctl/mmap 直通；补了 dup/close/fcntl/close_range 记账、
  stat/statx 设备号、`/sys/dev/char/.../uevent` 与 `drmGetNodeTypeFromFd`。
  同时给 `/etc/husk/gpu-env` 增加 `@GPU_FAMILY`，防止把 Mali 的驱动配置注入 Adreno。
  **没有 Adreno 真机，硬件路径仍是 UNKNOWN；Mali 回归门禁 A 38/38、Vulkan/GL 探针通过。**

### 修复

- App 壳增加独立 Rust 会话看门狗：App 进程单独死亡时通过 lease EOF 清理带
  `NR_SESSION_ID` 的 guest 进程，包括已经 `setsid()` 脱离会话组的 daemon；同时修复
  `Term`/`teardown` 的重复 close 路径。证据：evidence 234。
- **合成 `/proc`/sys 文件会被并发线程串内容**（evidence 235）。`/proc/mounts`、
  `/proc/self/status`、`/proc/sysvipc/shm` 和 `/proc/uptime` 共用一个无锁的 4 KiB
  `VTEXT`；16 线程 × 4000 轮实测 64,000 次里 35,348 次返回了另一个文件的内容，
  例如 `/proc/mounts` 读到 `Name:\t…`。现在只锁住内容生成到 memfd 写完的窗口，
  同一线程重入返回 `EAGAIN` 而不是自锁；修后 64,000/64,000 通过。
- 虚构身份的共享状态改为原子发布（evidence 236）：`FAKE_ROOT`、`FAKE_UID/GID`、
  `IDENT_PUB_UID/GID` 和 `IDENT_NOTES` 不再用无保护的 `static mut` 跨线程读写；
  `getuid/setuid/getgid/setgid/getresuid/getresgid` 真机功能探针通过。
- APK 的 `versionName/versionCode` 改为从仓库根目录 `VERSION` 读取（evidence 237）。
  修前 debug APK 硬编码 `0.1 / 1`，和 Release 的 `0.0.13` 不一致。
- 构建入口接受 NDK 的 `<prebuilt>` 与 `<prebuilt>/bin` 两种写法（evidence 238）。
  修前文档里的 `NDK=<prebuilt>/bin make apk` 会在 loader 阶段找不到 `clang`。

## [0.0.13] - 2026-11-26

> 主题：**全链路审计版**。这一版收掉路径兼容层、并发、挂载、loader、GPU/DRM、X11 呈现、
> 音频桥和输入栈里已经复现的 24 个缺陷。不能闭环的部分写进限制或明确标成 UNKNOWN；
> 不把「代码改过」当「真机已经通过」。

### 修

- **`/proc/self/exe` 一直报宿主 loader**（evidence 217）。根因两条：第一个进程根本不经过
  `execve` 钩子；而且 `exec` 会把上一份进程内存整个换掉，只在钩子里写这条路本身不成立。
  改成 boot 阶段从 `/proc/self/cmdline` 取回程序、折回 guest 路径、用 seqlock 发布。
  实测：`/data/user/0/…/libld.so` → `/usr/bin/readlink`；20 线程 × 200 次并发读 0/4000 撕裂。
- **`__xstat` / `__lxstat` / `__fxstatat`（含 64 位变体）在 guest 里 `ENOENT`**（evidence 218）。
  老二进制（glibc < 2.33 链接）走的正是这组版本化入口，实测补钩子后 `rc=0`。
- **`dlmopen` 打不开 guest 里的 `.so`**（evidence 219）。补钩子时踩到自指：一旦导出
  `dlmopen`，`dlopen` 内部的调用会解析到自己身上无限递归，改为从 link map 里
  「跳过自身地址」取 libc 那一份。
- **`execveat` 绕过 shim**（evidence 219）。折成一次 `execve`：先解析 `dirfd` + 相对路径，
  再走同一个钩子，loader 重入与 `#!` 改写都保留。`AT_EMPTY_PATH` 返回 `ENOSYS` 让调用方回退。

- **三处并发竞态**（evidence 220，都会给出错误答案而不是变慢）：`PEER_CACHE` 的 24 字节
  元组是三段 store，stat 路径上可能把一个进程的 uid/gid 安到另一个进程头上（改 seqlock，
  有界自旋，拿不到锁就不缓存）；`FAKE_ID_READY` 先置位后写值，可能把身份报成 0
  （改 Release/Acquire、先写值再发布）；`RAND_COUNTER` 非原子自增会让两个线程拿到同一个
  临时文件名（改 `AtomicU64::fetch_add`）。

- **挂载表每进程只读一次，导致同一个路径在两个进程里指向两个目录**（evidence 221）。
  `mount -t tmpfs none /mnt/q` 之后，bash 的重定向写进 rootfs 里的原目录、它 fork 的
  `touch` 写进挂载目录，`ls` 只看得见后者——写进去的字节就这么消失了。现在默认每次
  用到挂载表都重读（`NR_MNT_REFRESH=0` 可关），代价实测 +0.5%。
- **死挂载项跨会话累积，攒满 8 条后 `mount` 直接 `ENOSPC`**（evidence 221 §2）。
  `mount()` 之前清理后备目录已不存在的项并把结果落盘。

- **会话 umask 是 0000**（evidence 222 §1）。Android 给 `run-as` 的 umask 就是 000，shim 不碰
  它，于是 guest 里 `mkdir` 得到 0777、文件 0666：DING 拒绝从世界可写的 `~/Desktop` 启动桌面
  文件，daemon 对 mode 的判断也跟着错。`husk-session.sh` 与 `husk-desktop.sh` 两个会话入口
  统一 `umask 022`，实测 755 / 644。

- **`readlink("/proc/self/cwd")`、`("/proc/self/root")` 返回 ENOENT**（evidence 222 §2）。
  这两个魔术链接指向宿主命名空间，按路径解析过去会跳出 rootfs，所以 `passthrough()` 拒绝交给
  内核——这个拒绝是对的，但被顺手用到了「读链接」上。`realpath` 类相对路径解析、「我在哪」
  判断全部拿不到答案。现在读链接从 guest 视图回答：`cwd` → `/root`、`root` → `/`；
  `/proc/self/fd/N` 的目标也改写成 guest 路径（以前 `/proc/self/fd/1024` 直接暴露应用数据目录）。

- **GPU / 相机侧的懒初始化没有串行化**（evidence 224）。`DRM_CTX`+`DRM_MALI_FD`、
  `V4L2_CTX`、`/dev/dri` 假目录、`/dev/mali0` 的 dev/rdev 缓存全是「`if READY == 0 { 初始化;
  READY = 1 }`」：两个线程同时第一次 open 就各做一遍（fd/socket 泄漏、结构体一半新一半旧），
  而且普通 store 之间没有顺序保证，另一个核可能先看到 READY 再读数据。实测 8 线程并发
  `DRM_IOCTL_VERSION`，15 个进程里 2–3 个出现一次 EACCES；改成 CAS + owner tid 的
  `once_begin`/`once_end` 之后 30 个进程 / 4800 次 ioctl 零失败。`FAKE_FDS` 的
  「找空槽→写进去」也一并加锁（两个线程抢同一个空位会让被覆盖的 fd 的 ioctl 落到 memfd 上，
  客户端判定「不是 GPU」退回软件渲染）。

- **并发 `dlopen` 会把进程打死**（evidence 223）。`nr_repatch()` 在 `dlopen` 返回之后沿
  link_map 补 GOT，此刻 glibc 的装载锁已经放开；别的线程的 `dlclose` 把对象摘链、unmap，
  我们就走在已经释放的内存上。实测 8 线程 × 60 轮 dlopen/dlsym/dlclose，10 次里 2 次
  SIGSEGV，关掉 repatch 后 12 次全过。修法：`dlopen`/`dlmopen`/`dlclose` 共用一把进程内锁
  （`dlclose` 因此第一次被钩，`interposed` 195 → 196），repatch 只在持锁期间跑；
  另外两处懒解析/互斥的 check-then-set 换成 CAS。修后 45 / 45 全过，门禁 A 不变。

- **sigsys 去重表在信号处理器里 TOCTOU**（evidence 224 §4）。`SEEN_NR`/`SEEN_COUNT` 用
  「读计数→判断→写→加一」的写法，两个线程同时命中时计数可以越过 24，之后遍历读越界元素，
  去重从此失效、日志重新开始刷。改成 `fetch_update` 领槽。

- **`inotify_add_watch` / `statfs` / `getxattr` 一族在路径解析失败时报 EPERM**（evidence 222 §3）。
  `xattr_pinned()` 失败时写 errno 并返回 `-1`，调用方再喂给 `ret_or_errno()`，后者把 `-1`
  当成 `-EPERM` 又覆盖一次 errno。后果是「文件不存在」被说成「没有权限」：
  `dbus-daemon` 起会话总线时打印 "Cannot setup inotify … Operation not permitted"，
  任何把 EPERM 当致命错误的 watcher 会直接放弃。改成回 `-errno`，实测 EPERM → ENOENT。

- **音频桥的 accept 循环被单个客户端堵住**（evidence 225/231）。`serve()` 在 accept 循环里
  同步跑 `handle_client()`，而后者是一条长连接会话：一个连上就不发数据的客户端能把整条
  监听堵 30 秒（输出侧读超时），采集侧 `handle_capture` 连超时都没有，于是永久堵死；
  两个监听口还共用同一个循环。改成每客户端一个线程（`Arc<Api>` + 上限 12 + panic 隔离）。
  Husk 最小包真机运行验证：两个监听口的 idle + `PING` 并发判据 `PASS=2 FAIL=0`；播放
  `OPEN → DATA → CLOSE` 写入 48000/48000 帧，1 次 xrun、平均 121.667 ms；采集
  `OPEN → READ → CLOSE` 收回 49152 帧 / 196608 字节。

- **挂载表读缓冲是共享的 `static mut`**（evidence 227 §2.1）。`mnt_read_file()` 的
  `BUF` 是静态的、读在锁外：两个线程同时进来会往同一块内存里写。读的是同一个文件时看不出来，
  但表文件在两次读之间被改小（另一个线程刚 `umount` 并落盘）之后，`n` 更大的那次会把上一版
  的尾巴一起解析，得到一条已经不存在的挂载——路径解析到错的地方。改成栈缓冲（`MaybeUninit`），
  只把「解析 + 发布」留在锁里。试过"连读也进锁"，4 线程路径解析 9.08 → 21.59 µs/op（2.4 倍），
  否决；栈缓冲单线程 ±1%、4 线程在噪声内。

- **挂载表落盘不是原子的**（evidence 227 §2.2）。`mnt_save()` 用 `O_TRUNC` 直写，而读的一方
  每次路径解析都读它：能读到 0 字节（当作没读到、保留旧表，无害）或者**半张表**（刚挂上的
  东西在这次解析里不存在，路径掉回 rootfs 里的原目录）。改成写 `mounts.tmp` 再 `rename()`
  原子替换，临时文件建不出来时退回老写法。

- **`mount -o remount,ro <tgt>` 在 guest 里一直失败**（evidence 227 §3）。
  `mount(8)` 报 `mount: <tgt>: mount point not mounted or bad option`，但同样参数从 C 里直接调
  却成功。加 `NR_MNT_DEBUG=1` 打出原始参数后看清：util-linux 会先自己规范化 target（打开目录、
  `readlink(/proc/self/fd/N)`），而模拟出来的 bind 在路径层是同一个 inode 的两个名字，于是它把
  bind 的**源**当成 target 交过来（实测 `tgt=/mnt/src flags=33`）。修法两条：匹配时同时认 `src`，
  以及去掉尾部 `/`（内核接受 `remount /mnt/b/`，原来直接 EINVAL）。修后 `remount,ro` → rc=0、
  写入 `EROFS`，`remount,rw` 恢复可写，tmpfs 与尾部斜杠同样成立。

- **GPU 的 BO 只建不释放**（evidence 229）。`GEM_CLOSE` 没实现、`KBASE_MEM_FREE` 定义了却
  没有调用点：每建一个 BO 就永久占一次 kbase 分配，实测第 64 个 `CREATE_BO` 起一律 ENOMEM
  （4 KiB / 64 KiB / 1 MiB 三种大小都是 63 个，说明是次数上限不是字节数）。Mesa 关缓冲走的
  正是 `GEM_CLOSE`，桌面跑得越久越接近上限。补上 `GEM_CLOSE → MEM_FREE（用分配它的那个
  kfd）→ 摘表`：修前 5 轮 × 60 个只能过第一轮，修后 300 个全成。

- **`DRM_CTX.dispatch()` 在并发下会发出重复 handle**（evidence 229）。BO 表的
  「读 `next_handle` → 写槽 → `n_bos+1`」没有锁，8 线程 × 100 次里出现过 2–3 个重复 handle，
  后写的覆盖前一条。加 `DRM_LOCK` 串行化 dispatch；修后 8 线程 × 5 轮 × 64 个零重复，
  只剩 kbase 真实的 64 个活 BO 上限。GPU 冒烟中 `glxinfo`/`gl-probe` 正常；`glxgears`
  只完成了启动，随后在同一 Mesa 树里崩溃，不能算通过（见下方已知限制，evidence 232）。

### 加

- `docs/design/evidence/229-drm-bo-lifecycle-20261126.md`：DRM 层 BO 生命周期与 dispatch
  竞态的修前/修后对照。
- `docs/design/evidence/230-x11-input-audit-20261126.md`：X11 输入全屏 5/5、双客户端各 5/5；
  `adb input swipe` 25 秒 0 事件，按 UNKNOWN 记录，不冒充已支持。
- `docs/design/evidence/231-audio-bridge-runtime-20261126.md`：音频并发入口、播放 DATA、
  采集 READ 的真机协议结果，包括 1 次 xrun 和 121.667 ms 平均延迟。
- `NR_MNT_DEBUG=1`：把每次 `mount(2)` 的原始 `src`/`tgt`/`type`/`flags` 打到 stderr
  （evidence 227 §3 就是靠它定性的）。不设变量时只多一次 `env_lookup`。
- `docs/design/evidence/227-mount-table-and-remount-20261125.md`：挂载表跨线程的实际结论
  （9 处访问点逐条列出 + 两个真洞 + remount 那条 + 回归数据）。
- `docs/design/evidence/228-x11-presentation-20261126.md`：X11 呈现这条链的实测——
  X server 是应用里的 bionic 进程（`/proc/<pid>/maps` 里是 bionic libc、没有 libld.so），
  MIT-SHM 因内核没有 SysV IPC 而 `BadAccess`，客户端只能 `XPutImage`（2400×1080 上
  18.9 ms/帧、7–9 ms/兆像素），应用侧全屏动画时只有 5–9 FPS 且
  `0/0 present copies offloaded to GPU`。结论：除 DRI3 外当前布局没有更便宜的交付路径，
  最便宜的下一步在应用侧（把回读后的 CPU 拷贝换成 GPU 上传），真正的零拷贝要把 X server
  搬进 guest。
- `docs/design/evidence/226-audit-inventory-0.0.13.md`：0.0.13 的审计索引——已确认并修掉的
  24 条（带证据编号）、记录在案的 7 条限制、9 项「核过不是缺陷」，以及仍未闭环的 GPU
  长跑和跨设备验证。
- `tools/probe/audio-bridge-concurrency.py`：音频桥的 head-of-line blocking 判据
  （idle 客户端 + 第二个客户端 PING），evidence 225 §3。
- `tools/probe/audio-capture-probe.py`：采集桥的 `OPEN → READ → CLOSE` 与 PCM 字节数判据。
- `tools/probe/symbol-gap-probe.c`：把没钩的符号逐个**试出后果**（判据是效果不是符号）。
- `docs/design/compat-layer.md`：三层拦截、覆盖账（98 / 111）、与 ptrace 型的代价对比、未闭合项。
- 门禁 A 里 `/proc/self/exe` 的旧断言写的是**坏行为**，换成三条正向断言；现 38 项。

### 已知限制

- **GPU 长跑不能宣称通过**（evidence 232）。`glxgears -info` 在 zink → PanVK 上于 45 秒超时前
  自行退出 `RC=139`，崩溃地址为 `libvulkan_panfrost_x11.so` 的
  `emit_cf_list.isra.0+0x120`，故障地址 `0x21`。pre-229 loader 同样复现；关闭 shader cache、
  `PANVK_DEBUG=noopt`、改 GL 版本都不改变结论。llvmpipe 同一程序跑到预期的超时，
  `gl-probe window` / ES3 / Vulkan probe 仍通过。具体 NIR 破坏原因 **UNKNOWN**；本版不把
  “代码改过”或“单个探针通过”写成 GPU 全量稳定。

## [0.0.12] - 2026-11-25

> 主题：**`scandirat` 补上，覆盖度探针进门禁**。0.0.11 里唯一没修通的那条漏网符号
> （glibc 内部走 `__openat64_nocancel` 的直接调用，导出同名符号拦不到、body patch 又会
> `sig=11`）改成自己实现扫描体；`tools/probe` 并进 `make gate-a`，门禁从"命令返回 0"
> 变成"文件确实落在 guest 里"。

### 修

- **`scandirat(3)` 在 guest 里一直是 `ENOENT`**（evidence 215）。它的路径参数由 glibc 内部的
  `__openat64_nocancel` 直接发起，我们导出同名符号也拦不到。改成自己实现：用 `openat`
  拿目录 fd（翻译与只读检查都在里面）→ `fdopendir` → 与 `scandir` 共用同一个扫描体。
  实测 `n=-1 errno=2` → `n=36 errno=0`。

### 加

- **`make gate-a` 现在跑两段**：36 项验收 + `tools/probe/run-coverage-probe.sh`。
  探针的判据是"这些文件到底落在 guest 里还是宿主里"，不是"命令返回 0"（evidence 215 §2）。

## [0.0.11] - 2026-11-25

> 主题：**安装器不再把失败当成功 + path-shim 的漏网符号补上并实测**。三处把子进程退出
> 状态丢掉的地方（第二遍解包、符号链接改写与校验）改成读真实退出码；第一遍解包的状态捕获
> 在 `set -e` 下本来就走不到；`NR_GPU_DEFAULT_OFF` 改成按布尔值判断；删掉没有调用者的入口桩。
> 另一半是覆盖度：`inotify_add_watch` 与 `utime` 实测确实漏翻译，已修并给出 10 条点测；
> 顺带交付「符号差集」和「探针」两个工具，把覆盖度从猜测变成可重跑。

### 修

- **第二遍解包失败会被当成功**（evidence 212）。`install-husk.sh` 里
  `'tar -xf …' 2>&1 | sed 's/^/   /'` 的退出状态是 `sed` 的，`tar` 挂了看不出来。
  改成写日志文件、直接读 `tar` 的返回码，非 0 就 `exit 69`。
- **符号链接校验永远不会失败**（evidence 212）。`husk-symlinks --root / --verify --quiet`
  同样走了管道，后面那个 `|| { … exit 71; }` 因此永远不触发。改成两次调用都读真实返回码，
  `--apply` 或 `--verify` 任一非 0 就 `exit 71`。
- **第一遍解包的"只容忍硬链接错误"分支是死代码**（evidence 212）。脚本开头 `set -eu`，
  `tar …; rc=$?` 在 `tar` 失败时已经退出，`rc` 永远读到 0。改成 `if tar … ; then rc=0; else rc=$?; fi`。
- **`NR_GPU_DEFAULT_OFF` 只看变量在不在**（evidence 212）。文档写的是 `=1` 关闭默认 GPU 注入，
  代码却对任意取值都关，`NR_GPU_DEFAULT_OFF=0` 也会关掉。改成按布尔值判断：`1`/`true`/`yes`/`on`
  关，`0`/`false`/`no`/`off`/空串都不关。

### 删

- **`tools/loader/entry-stub.S`**（及其构建产物 `entry-stub.bin`）。它是 `_start` 之前装 SIGSYS
  handler 的入口桩；两个 `svc` 调用点(`set_robust_list`/`rseq`)改成构建期 `nop` 之后它没有
  调用者，`build-embedded-loader.sh` 也不编译它。内容在 git 历史里。`docs/ARCHITECTURE.md`
  和 `docs/assets/architecture.svg` 里把它画成实际入口的那两处一起改掉。

### 修（覆盖度，evidence 213/214）

- **`inotify_add_watch` 没被翻译**（evidence 214）。GIO 的文件监视（Nautilus 自动刷新、编辑器
  感知外部改动、构建工具 watcher）全走这条，guest 里一直是**静默失效**：不报错，只是永远不来
  事件。实测改前 `wd=-1 ENOENT`，改后 `wd=1` 且事件 `n=32`。
- **`utime(3)` 没被翻译**（evidence 214）。glibc 自己发 `utimensat` 系统调用，不经过我们导出的
  `utimensat` 符号。实测改前 `rc=-1 ENOENT`，改后 `rc=0`。
- 新增 `__openat_nocancel` / `__openat64_nocancel` 导出，并把四个名字加进 `BODY_PATCH`。

### 加（工具，evidence 213/214）

- `tools/dev/path-symbols.txt` + `tools/dev/hook-coverage.sh`：把「需要路径翻译的 111 个符号」
  和 path-shim 实际导出的 184 个做差，覆盖度从猜测变成一条命令（当前 87/111）。
- `tools/probe/coverage-probe.c` + `run-coverage-probe.sh`：在设备上逐条验「翻译到底发没发生」，
  跑一次 `--- coverage-probe: PASS`。顺带把 213 里四条**推断错误**（`remove`/`freopen`/`pathconf`/
  `truncate64` 实测都是通的）改回实测口径。

### 测过（FACT，evidence 212）

| 项 | 值 |
|---|---|
| 截断的 rootfs 包 | 安装器 `exit 68`，不再打印 `== done ==` |
| 正常 rootfs 全新目录安装 | `rc=0`，`== done ==`，自检 `bash -l -c 'id; head -1 /etc/os-release'` 通过 |
| `NR_GPU_DEFAULT_OFF=0` | `gl-probe` 仍报 `zink`（改前报 `llvmpipe`） |
| `NR_GPU_DEFAULT_OFF=1` | 不注入，`gl-probe` 报软件渲染 |
| 门禁 A | `PASS=36 FAIL=0` |

### 没做

- 没有把第二遍解包的错误分类（哪些算"可容忍"）。这一遍本来就只做硬链接物化，非 0 一律当失败。

## [0.0.10] - 2026-11-25

> 主题：**中文界面**。0.0.9 让 locale、字体、输入法通了，但界面文字还是英文——缺翻译。
> 这一版把 `zh_CN` 的 413 个 `.mo` 单独打成 5 MB 的附加包，安装器解开，不重打 1.3 GB 的 rootfs。

### 新增

- **`tools/langpack/husk-lang-zh.tar.gz`**（evidence 211）：从 noble 的
  `language-pack-zh-hans-base` 与 `language-pack-gnome-zh-hans-base`（都是 `24.04+20260905`）
  解出 413 个 `.mo`，装到 `usr/share/locale-langpack/zh_CN/LC_MESSAGES/`。Ubuntu 的 glibc 会搜
  `locale-langpack`，所以 `/usr/share/locale` 里原有的 19 个不用动。来源、sha256 与重建步骤写在
  `tools/langpack/README.md`。
- **`install-husk.sh` 解开语言包**：树包旁边有这个文件就解到 `$DEST`，数 `.mo`，少于 400 个
  直接 `exit 74`；没有就跳过并打印"界面文字保持英文"。
- **`make dist` 带上它**，并断言包里有 `usr/share/locale-langpack/zh_CN/LC_MESSAGES/gnome-shell.mo`。

### 测过（FACT，evidence 211）

| 调用 | 无 LANG | `LANG=zh_CN.UTF-8` |
|---|---|---|
| `dgettext("gnome-shell","Activities")` | `Activities` | 活动 |
| `dgettext("nautilus","Files")` | `Files` | 文件 |
| `dgettext("gtk40","Cancel")` | `Cancel` | `Cancel`（该域没有这条 msgid，不是查找失败） |
| `gnome-calculator --help` | `Usage: / Help Options: / Show help options` | `用法： / 帮助选项： / 显示帮助选项` |
| 全新目录安装（0.0.10 树包） | `rc=0`，`locale-langpack/zh_CN/LC_MESSAGES/` 413 个 `.mo`，`gnome-shell.mo` 与包内逐字节一致，`sonames ok` | |

### 没做

- 只有 `zh_CN`，`zh_TW` / `zh_HK`（繁體）没打。
- 翻译版本绑在 `24.04+20260905`，换 Ubuntu 版本要重下。

## [0.0.9] - 2026-11-25

> 主题：**中文默认值。** 新装的树默认 `LANG=zh_CN.UTF-8`、中文字体族落到 Noto Sans CJK SC、
> 会话起来拉起 fcitx5 并带一组「键盘 + 拼音」。界面翻译包不全，这条写进 `docs/LIMITATIONS.md` §12。

### 新增

- **`tools/guest-config/husk-locale.sh`**（evidence 210）：装到 `/etc/profile.d/husk-locale.sh`，
  `LANG` 为空、`POSIX`、`C`、`C.UTF-8` 时补 `zh_CN.UTF-8`；程序自己设了就不动。
  安装器同时写 `/etc/default/locale`（树里编译了 `zh_CN.utf8` 才写）。
  为什么不用 `/etc/default/locale` 单独承担：它只对走 PAM 的登录 shell 生效，run-as 起的进程、
  桌面 fork 出来的程序、`env -i` 都读不到。
- **`tools/guest-config/fcitx5-profile`**：一组「键盘（keyboard-us）+ 拼音（pinyin）」，
  `DefaultIM=pinyin`。安装器只在 `/root/.config/fcitx5/profile` 不存在时写，用户改过不覆盖。
  不带这个文件时 fcitx5 第一次启动只建「键盘」一组，拼音要用户自己进 configtool 加。
- **`husk-desktop.sh` 第 5c 步**：拉起 `fcitx5 -d --replace`，`fcitx5-remote -o` 成功则打
  `CHECK im=fcitx5`；会话环境补 `GTK_IM_MODULE=fcitx`、`QT_IM_MODULE=fcitx`、
  `XMODIFIERS=@im=fcitx`，以及 `LANG` / `LANGUAGE`。
- **`tools/device-tests/input/imtest.sh`**：起编辑器 → 给焦点 → `ctrl+space` → 打 `nihao` →
  空格 → `ctrl+s` → `od` 读回文件字节，一条命令跑完整条链路。

### 测过（FACT，evidence 210）

| 项 | 值 |
|---|---|
| `locale` / `nl_langinfo` | `zh_CN.UTF-8`；`MON_1=一月`；`D_FMT=%Y年%m月%d日` |
| gnome-shell 进程 environ | `LANG=zh_CN.UTF-8`、`LANGUAGE=zh_CN:zh` |
| 字体族 | `fc-match "sans-serif:lang=zh-cn"` → `Noto Sans CJK SC`（改前落 `Noto Sans CJK JP`） |
| 会话自检 | `CHECK wm=yes / ding=yes / gpu=zink / input=primed / im=fcitx5` |
| 输入法 | 有焦点窗口 `ctrl+space`：`fcitx5-remote` 0 → 2，`-n` = `pinyin` |
| 端到端 | `gnome-text-editor` 打 `nihao`+空格 → 文件字节 `e4 bd a0 e5 a5 bd`（`你好`），`ctrl+s` 落盘 |
| 全新目录安装（0.0.9 树包） | `rc=0`，安装器自检 `husk -- bash -l -c 'id; head -1 /etc/os-release'` 通过；三处新写入都在：`etc/profile.d/husk-locale.sh` 724 B、`etc/default/locale` = `LANG=zh_CN.UTF-8`、`root/.config/fcitx5/profile` 533 B（`DefaultIM=pinyin`），`etc/profile.d/husk-gpu.sh` 照旧 3418 B |

### 没做

- **界面翻译包不全**：`/usr/share/locale/zh_CN/LC_MESSAGES/` 只有 19 个 `.mo`，没有 `gtk40`、
  `gnome-shell`、`mutter`、`nautilus`。`LANG` 改成中文只影响日期格式、字体族与排序，界面文字
  仍是英文。补语言包约 30 MB 且要重打 1.3 GB rootfs，本版不做。
- 输入法只在 GTK4 上验过；XIM 前端、Qt、候选窗翻页与数字选词未测。

## [0.0.8] - 2026-11-25

> 主题：**把 0.0.7 留下来的那条 SIGSEGV 悬案结掉。** 结论是崩的并不是 `XIUngrabDevice`：探针调
> 的是 XInput-1 的 `XUngrabDevice(Display*, XDevice*, Time)`，却按 `(Display*, int, Time)` 声明
> 并传了整数，libXi 把整数当指针解引用。运行时代码没有改动，这一版只修测试工具、把错判撤掉，
> 并让 `docs/LIMITATIONS.md` §7 的现状与实测一致。

### 修复

- **`tools/device-tests/input/xi2probe.c` 调错了函数**（evidence 209）。原声明
  `extern int XUngrabDevice(Display *, int, Time);` 并调用 `XUngrabDevice(d, 2, 0)`，而
  X.Org 的 `XInput.h` 里 `XUngrabDevice` 的第二个参数是 `XDevice*`。传整数 2 进去，libXi 解引用
  该地址，进程 SIGSEGV。改成 `XIUngrabDevice(d, 2, 0)` 后，`grab` 模式完整跑完：
  `XIGrabDevice ret=0` → `RESULT mode=grab …` → `rc=0`；修前是同一条命令 `segv|sig=11`、`rc=139`，
  且看不到 `RESULT`。
- **新增 `tools/device-tests/input/ungrabmin.c`**：只做 `XOpenDisplay` → 一次解除抓取 → `XSync`，
  四个模式（`xi` / `core` / `core-pt` / `xi-grab`）分别复跑，用来区分核心协议、XInput-1、XI2
  三条路。本机实测：`xi` rc=0、`core-pt` rc=1（GrabNotActive）、`xi-grab` rc=0、错声明下的
  `core` rc=139。

### 更正

- **evidence 208 §1 / §2 作废**：「崩点在 `XIUngrabDevice`」不成立（那个程序从未调用它），
  server、libXi、path-shim 都没有问题，责任在探针自己的函数原型。
- **`docs/LIMITATIONS.md` §7 标题**原写「桌面还吃不到」。当前事实是 gnome-shell 46 由
  zink → PanVK 出帧、X 根窗口非黑像素 100%（evidence 165/168），标题改为「桌面已经在 GPU 上
  出帧，剩下的是呈现那一次拷贝」，并在开头补现状段。
- **§7 里 nautilus「内容区空白」定案**（evidence 207）：清掉残留实例后 15 秒出窗，窗口内
  26 286 种颜色、内容区 17 223 种、白底占比 0.757，内容区不空白；「点了没反应」是单实例被上一轮
  会话的残留进程劫持，`--new-window` 把请求转给一个 X 连接已死的实例。

### 产物

- 树包除 `VERSION` 一个文件外，与 0.0.7 的树包**逐字节一致**：15 个条目逐个比对 sha256 全部相同
  （`libld.so fd1ec526…`、`libc.so.6 69ceef37…`、`install-husk.sh 79e1f8c1…`、`husk-desktop.sh
  d7193c89…` 等）。1 928 579 字节，sha256
  `49de7400173232345ae7c29eeb1eeca9708a92a68d366597e4018d0c2e4863f2`。

## [0.0.7] - 2026-11-25

> 主题：**把 0.0.6 里"窗口拖不动"这件事查到底，然后修掉。** 结论是这条旧结论本身是错的——
> 拖不动的只有**每个会话里的第一次**，第二次起就正常。修法是在会话起来后做一次输入预热。
> 同时把 197/198/203/205 四份证据里"拖动完全失效"的判断全部更正。

### 修复

- **会话里第一次指针拖动不搬窗**（evidence 206）。`husk-desktop.sh` 在自检之后加第 5b 步
  「输入预热」：屏幕中下部（`W/2, H*3/4`，避开左侧 dock 与 DING 图标列）做一次
  `click` + 一次 `mousedown → +8px → mouseup`，输出 `CHECK input=primed`。
  - 修前：起会话后立刻拖 → `236,188` 一格不动；45 秒后同一会话再拖 → `536,388`。
  - 修后：起会话后立刻拖 → `536,388`（第一次就动），实测两次会话一致。
  - 预热不是修根因：第一次 grab op 为什么丢位置更新还没定（mutter 日志显示抓取成功、
    边缘计算也跑了，就是位置没更新），留作 UNKNOWN。
- **顺带验了吸附**：拖到左边缘松手 → `46 12 1207 1088`，即 2400×1080 的左半屏平铺。

### 更正（重要）

- 197 / 198 / 203 / 205 写的"拖动完全失效"**作废**：那四份测的都是新会话里的第一次拖动，
  把一次性故障当成了功能缺陷。
- 205 初版把原因归到"这台 server 的 XI2 选择/枚举是坏的"，**也是错的**——我们自己的临时探针
  把 `XISelectEvents` 的请求构造错了。用仓库里的 `xi2probe.c` 重测：`rc=0`，事件到手。
- 本轮把事件层逐条排干净：核心抓取（窗口级 / root 级）、`XISelectEvents`（窗口 / root）、
  `XIGrabDevice(root)` + XI2、XI2 motion 的 `root_x/root_y` 坐标——全部正常。

### 测过

| 项 | 数 |
|---|---|
| 第一次拖动（修前 / 修后） | `236,188` 不动 / `536,388`（+300,+200） |
| 第二次拖动（同会话） | `536,388` |
| 边缘平铺 | `46 12 1207 1088` |
| XI2 motion 收到数（grab + root 选择） | 10 motion + 1 button |
| XI2 motion 里的坐标 | 与注入值逐点一致（400,300 / 520,370 / 640,440） |

### 工具

- `tools/device-tests/input/xi2probe.c`：映射后等 400 ms 再抓（否则 `XIGrabDevice` 回
  `3=NotViewable`，测出来的是"窗口没画完"）；新增 `SELECT_ON_ROOT=1`、`GRAB_ON_WINDOW=1` 两个开关。
- `tools/device-tests/input/drag-window.sh`：Super+拖 vs `alt+F7` 的对照脚本入仓库。

### 已知问题

- 第一次 grab op 丢位置更新的**根因未定**（UNKNOWN），本版用预热绕过。
- 挂载变化通知失效（evidence 202）、`getuid()==0` 的 privileged 提示、应用被 Android 冻结后
  桌面停摆（evidence 203）——三条与 0.0.6 相同，仍未修。

## [0.0.6] - 2026-11-25

> 主题：**把"桌面会话"从一堆手搓命令收成一个入口，并把三件长期挂在"根因 UNKNOWN"上的事钉死。**
> 0.0.5 说的是"硬件怎么接"，0.0.6 说的是"这个会话凭什么能稳定起来、以及它哪里还坏着"。
> 这一版新增了**一条用户可见的边界**（应用被冻结时桌面会停摆），也把一个被当成 bug 的现象
> 明确降级成已知限制。

### 改

- **新增 `husk-desktop.sh`（会话单一入口）**，安装器落盘（evidence 200、203）。
  清残留进程 → 清 `gnome-shell-disable-extensions` / 陈旧 `bus` / `gvfsd` / `at-spi` →
  起系统总线与会话总线 → 起 shell → 等 `_NET_SUPPORTING_WM_CHECK` → 自检 → 拉起壁纸看护。
  实测输出 `CHECK wm=yes` / `CHECK ding=yes` / `CHECK gpu=zink`，`EXIT=0`，两次独立会话均可复现。
  两条硬坑写死在注释里：**dbus 地址必须用 guest 侧路径**（宿主路径会二次加 rootfs 前缀，
  main.js 直接抛异常）；**后台进程必须重定向 stdin/stdout/stderr**（否则调用方 `| tail` 挂住）。
- **发布包补齐运行时要用的脚本**：`make dist` 以前只把 `install-husk.sh` / `husk-session.sh`
  打进 tarball，`husk-desktop.sh`、`husk-ding-fallback.sh`、`husk-wallpaper-fit.sh`、
  `guest/husk-wallpaper-scale.js`、`guest-config/pulse-client-no-shm.conf` 全都不在里面。
  后果是"从 Release 的 tarball 装出来的树"和"开发机上的树"不是同一棵树：DING 兜底永远为假、
  壁纸不做预缩放、pulse 客户端静默丢掉 `enable-shm=no`（就是那个"装了 pulse 却没声音"）。
  现在五个文件都进包，并加了逐条 `tar tzf | grep -qx` 断言——少一个就构建失败，不再靠人记。
  安装器也补上把 `husk-ding-fallback.sh` / `husk-wallpaper-fit.sh` 放到 `husk-session.sh` 同级。
- **壁纸缺失/失效兜底**（evidence 199）：`husk-wallpaper-fit.sh` 在空 URI 时按
  `husk-default.png` → 内置壁纸 → 任意 png/jpg 兜底，并复位 `primary-color`；
  安装器此前只装 `husk-wallpaper-scale.js`、**从没装 `husk-wallpaper-fit.sh`**，
  于是 `husk-session.sh` 的可执行判断永远为假——这一处一并补上。
  验收：跨整会话重启的六个采样点逐字节一致。

### 测过（数字）

- **输入延迟**（evidence 201，3 轮 × 100 次，同进程 `XTestFake*` + `XSync` 往返）：
  键盘中位 **1.04–1.21 ms**、按钮中位 **0.25–3.09 ms**，最小 0.14 ms ——
  path-shim 与 X server 的请求通路**不是**瓶颈；尖峰 **8.3 / 30.3 ms** 正好是 120 Hz / 33 Hz
  一帧，是被批量到下一个 grab/frame 之后投递，视为一拍延迟（INFERENCE，未做帧计时器对拍）。
  限定：不含 `/dev/input` → X 这一段，也不含合成器 → 屏幕扫描，**不等于手指到画面的延迟**。
- **拖动窗口**（evidence 203 复核）：`Super`+拖位移 **0**（`236,188` → `236,188`）；
  `alt+F7` + 指针 motion 位移有效（→ `186,120`）。
- **发版门禁 A：PASS=36 FAIL=0**（evidence 204）。第一次跑是 35/36，失败项是
  `guest /data/user does not exist`——rootfs 里有一条 14 层深、32 KB 的空目录链，
  正好等于把 `$ROOTFS` 的宿主绝对路径当成 guest 路径 `mkdir -p` 了一次
  （时间戳 16:17，会话调试期间）。已清；成因记 UNKNOWN 并写明候选，
  这条门禁断言就是探测器。
- **`/proc/self/mountinfo` 的 EPERM 定根因**（evidence 202）：虚拟 proc 文件全是 **memfd**
  （`S_IFREG`、`dev 0:5`、`/memfd:husk-virt (deleted)`），而 libmount 的内核挂载监视只做
  `epoll_ctl(ADD)`，**内核对普通文件一律 EPERM**。不是 mountinfo 独有，`/proc/uptime` 一样。

### 已知问题（本版未修，已核实）

- **窗口拖不动、连带对齐/吸附**（限制 §10）：坏在 mutter 的 grab op，位置在 X server 侧；
  客户端请求、X 层被动抓取、「mutter 没用 `XIGrabDevice`」三条全部实测排除。
  替代操作 `alt+F7`。给 2.0 侧的复现步骤在 evidence 203 §4。
- **挂载变化不再有通知**（限制 §11 之外的另一条，evidence 202）：`GUnixMountMonitor` 拿不到
  channel 后**既不挂 watch 也不退回轮询**（3 秒轮询只属于"找不到 mtab 路径"那条分支），
  于是 `udisksd` / `gvfsd` / `nautilus` 都只在主动读时才看到最新挂载表。
  修法三条已列（pipe 承载 / hook `epoll_ctl` / 不动），前两条各自的代价写在 evidence 202 §5，
  本版选"不动"，因为它**不产生错误行为**，只产生噪音与延迟。
- **Nautilus 的 `This app cannot work correctly if run as root`**：会话真 uid 就是 0，
  假身份（`NR_FAKE_UID`）被 dbus 的 EXTERNAL 认证挡死（evidence 200）。要消只能改
  gnome-shell 资源或预载层丢 `Notify`——都是动别人代码，0.0.6 不做。
- **屏幕熄灭后桌面"像死了一样"**（限制 §11）：应用转 cached 被 Android Cached Apps Freezer
  整体 SIGSTOP，X server 住在应用进程里，于是新连接一律卡住。判据是
  `/proc/<x11 pid>/task/*/wchan` 全为 `get_signal` 且 `dumpsys power` 显示 `mWakefulness=Asleep`；
  唤醒即恢复，**不是崩溃**。

### 文档

- [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)：§8 输入一行补"拖动失效"；新增 §10（拖动缺陷与
  `alt+F7` 替代、三条已排除路径、给 2.0 侧的三步复现）、§11（冻结判据与"别当崩溃查"）。
- 新增 evidence 199 / 201 / 202 / 203；198 追加两次（XI2 设备树、mutter 不用 `XIGrabDevice`）。

## [0.0.5] - 2026-11-25

> 主题：**说清楚「硬件到底怎么接」，并把图形默认值从登录脚本下沉到加载器。**
> 0.0.4 把代价钉死了，0.0.5 把「谁在什么地方吃到 GPU」这件事从"登录 shell 才算"改成
> "经加载器起来就算"，同时把接口口径收成一根轴。

### 改

- **图形默认值下沉到加载器**（evidence 182）。`getenv` / `secure_getenv` / `__secure_getenv`
  由 path-shim 提供，值来自安装器写的 `/etc/husk/gpu-env`；查询顺序是**先 `environ`，
  查不到才用默认值**。于是登录 shell、cron、systemd user unit、桌面 fork 出来的进程、
  `env -i` 看到的是同一套渲染器：`env -i` 起 `gl-probe es3`
  → `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`（改动前是 `llvmpipe`），两者都 PASS。
  - 程序自己 `export` 的变量永远优先（只覆盖一个变量时其余仍取默认值，实测过）；
  - `HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF=1` 一条都不注入；
  - `secure_getenv` 尊重 `AT_SECURE`；
  - 树里没有 `husk_*.json` 时 `/etc/husk/gpu-env` 保持空文件，行为与改动前完全一致；
  - 门禁 A：**PASS=36 FAIL=0**。

### 已知问题（本版未修，已核实）

- **桌面会话形态没验过**：本仓库的验收只到「X 根窗口非黑像素 100%」「帧有变化」
  「GTK 客户端出画」「键盘能敲进去」，**没有**验「这是不是一个带主题的正常会话」。
  实测这台机器上主题文件是齐的（`/usr/share/themes/{Adwaita,Adwaita-dark,…}`、
  `gnome-shell-theme.gresource` 710 151 B、`gnome-shell-common` / `yaru-theme-gnome-shell` /
  `gnome-themes-extra` 都是 `install ok installed`），所以「主题丢了」不是缺包，更像会话侧
  （session mode / dconf / gnome-settings-daemon 的 xsettings）没把主题用起来——**根因 UNKNOWN**，
  要开着会话抓 `gnome-shell` 日志才能定。
- **窗口拖不动、位置不对齐**：输入默认是 trackpad 语义（2.0 侧 `touchMode` 默认值不在本仓库），
  触摸拖动在 mutter 里不是「按住标题栏拖窗口」。现有证据只证明「点哪落哪」（evidence 148、180），
  **拖动 / 对齐 / 吸附没有测过**。

### 文档

- 新增 [`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md)：每类硬件
  一根轴——直连（GPU、相机）/ 中转（音频、视频）/ 未做，并附 2026-11-25 在应用域实测的节点布局
  （`/dev/mali0` = `10,33` `0666`、`/dev/video0` = `81,0`、`/dev/dri` 只有 `card0`、
  `renderD128` 不存在、`/dev/snd` 打不开）。
- [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)：§8 表加「接法」列；§7 补两件实测——
  呈现那次拷贝是**架构共性而非本机带宽**（guest 内 `memmove` 10.05 GiB/s，2400×1080 一帧
  值拷贝下限 1.0 ms，对实测回读 30.27 ms/帧），以及「三条可选路线各自落在哪一层」。
- README：硬件接口按「直连 2 类 / 中转 2 类 / 显示零拷贝 / 输入窗口层 / 未做 5 类、指纹不做」
  重写，并明确「全部直连」和无限定的「零拷贝」都是不能说的话。

## [0.0.4] - 2026-11-25

> 主题：**把「GPU 到底快多少」和「哪里确实还坏着」用数字钉死**。0.0.3 说的是「桌面由 GPU 出帧」，
> 0.0.4 说的是「哪类负载快多少倍、呈现那一段贵在哪、哪些真实客户端画不出来、以及为什么」。
> 这一版没有新增能力面，全部是测量、定位与工具。

### 测过

- **GPU 原生加速按负载分开量**（2400×1080，本机日常分辨率）：着色器密集负载 GPU 23.84 ms/帧
  （41.9 fps）对软件 266.73 ms/帧（3.7 fps），**快 11.2×**；纯清屏离屏 2.25 vs 2.01 ms；
  清屏 + 窗口呈现反而是软件快（19.9 vs 49.7 fps）——差在每帧把 GPU 表面回读再交给 X server
  （30.27 ms/帧 @ 2.59 兆像素），但 llvmpipe 同尺寸的残差是 −0.01 ms，说明模型成立
  （evidence 174）。
- **窗口 swap 拆成「渲染」和「呈现」两段**：GPU 渲染在 160×120→1080×1080 之间都是 1.2–1.7 ms，
  呈现按窗口面积线性涨（zink 24.8 ms/兆像素，llvmpipe 11 ms/兆像素）。640×480 的 8.9 ms 里
  GPU 只占 1.3 ms，其余是 GPU→CPU 回读 + 进 X server 的拷贝；DRI3 在这台机上没有设备可用
  （evidence 174）。
- **关屏 150 秒再唤醒，会话没死**：同一 pid、线程 33 不变、fd 41→40、内存 +4 kB，唤醒后
  X 根窗口仍 100% 非黑（evidence 173）。注意：全程 USB 供电 + adb 在线，**不是**深睡/Doze。
- **真实 GTK4 客户端跑通**：GTK 4.14 的 `gnome-text-editor`（`GSK_RENDERER=ngl`）在 zink/PanVK 的
  X11 上开窗并画出完整界面，X 根窗口 2400×1080 抓图 28 565 种颜色、非黑；同时记录
  `KBASE_IOCTL_MEM_EXEC_INIT` 被拒（EPERM）但渲染正常，原因未查清（evidence 176）。
- **文件管理器实测是个反例**：`nautilus` 46 能开窗、能枚举（日志逐条打出 `file:///etc/*`），
  但内容区整块空白、连列头都不出现；`GSK_RENDERER=cairo` 与 `ngl` 一样、`list-view` 与
  `icon-view` 一样，**同一份 rootfs 换 proot 起也照样空白**，而同一会话里 `gnome-control-center`
  与自写的最小 `GtkColumnView` 程序都正常——所以不是 path-shim 造成的，根因 UNKNOWN
  （evidence 178，`LIMITATIONS` §7/§8）。
- **5 个打不上的 libc hook 核清四条、留一条 UNKNOWN**：反汇编证明 `__readlink_chk` /
  `__readlinkat_chk` / `__realpath_chk` / `__mq_open_2` 的"跳进窗口"是它们自己跳自己，
  真正的活是尾调用已接管的入口；`__openat64_nocancel` 这版没导出符号，标 UNKNOWN
  （evidence 175，`LIMITATIONS` 新增第 9 节）。
- **键盘输入打通到 GTK4 客户端**：XTEST 敲进 `gnome-text-editor`，标题栏与正文都跟着更新、
  右边界折行正常，改窗口尺寸触发全量重绘后内容一致、无残影。只测键盘，没量延迟，
  IME/组合键未测（evidence 180）。
- **`glReadPixels` 的机制量出来了**：目的行距恒为 16 字节（与宽度无关），每行只有前 4 个像素
  落地，写入量 = `H×16 + (W×4 − 16)`；换 `PACK_ROW_LENGTH` 无效、`PACK_ALIGNMENT=1` 更糟。
  **逐行读与逐像素读 100% 正确**，1×H 单列读同样坏。llvmpipe / zink / proot 三处完全一致
  → 不在 path-shim、不在 GPU 后端，落在这版 Mesa 的公共读回层（具体文件 UNKNOWN）
  （evidence 179）。

### 改进

- **CI 入仓**（`.github/workflows/ci.yml`）：`fmt` / `cargo check` / `clippy` / shell 语法 /
  文档链接 / 版本一致性，另有一个 libc 作业——从锁定的 `libc6 2.39-0ubuntu8.12` arm64 包重建
  `libc.so.6` 并校验 sha256 `69ceef37…`，所以「补丁版 libc 与包里的那份是否一致」不再靠人手比对。
- `gl-probe` 新增 `readback [WxH]`：把每像素唯一色的纹理挂到 FBO 上整块读回，用值本身反解
  「这块数据原本是哪一格」，并把「整块 / 逐行 / 逐列 / 单点」四种读法一次报全。这是查
  `glReadPixels` 缺陷的探针，也是 evidence 179 的取证工具。
- `gl-probe` 的 `window` 子命令接受 `WxH` 和帧数（原来是写死的 640×480 / 60），并新增
  `pbuf WxH N` 离屏基线：同一套 EGL/GL、同样 clear+swap，但每帧 `glFinish`，用来把 GPU 渲染
  和呈现分开量。不带 `glFinish` 的离屏版本会量到 0.1 ms/60 帧的假数字。

## [0.0.3] - 2026-11-24

> 主题：**图形通路落到 GPU，并且默认就生效**。桌面不再是软件渲染——gnome-shell 46 跑在
> zink → PanVK 上出帧，默认值写在 guest 的 `/etc/profile.d/husk-gpu.sh`，不需要给应用单独配置。

### 修复

- **桌面在 X11 上由 GPU 出帧**：重编 PanVK 打开 x11 平台后 `VK_KHR_xcb_surface` 才会暴露
  （`panvk_instance.c:125` 的 `#ifdef` 之前被 `platforms=[]` 关掉），`gl-probe window` 从
  "死在 `eglMakeCurrent`" 变成 1→10 全过、60 次 swap；gnome-shell 46 在 zink 上起来并出帧，
  X 根窗口抓到完整桌面（evidence 165）。
- **mutter 启动崩溃（MIT-SHM `BadAccess`）的定位**：guest 的 SysV IPC 是 shim 用文件模拟的，
  X server 在 guest 外，拿整数 `shmid` 去 `shmat` 必然 `BadAccess`。这个机理是实测的（FACT）；
  但"让 `XShmQueryExtension` 返回 0"这条修法**未验证生效**（补测它仍返回 1），
  桌面不再崩另有原因，按 UNKNOWN 记（evidence 165 §2.1）。
- **假 DRM 节点现在"像"设备**：`/dev/dri/renderD128`、`card0` 的 `stat`/`statx`/`fstat`
  回真 `/dev/mali0` 的字符设备号（10:33）+ 0666，并合成
  `/sys/dev/char/<maj>:<min>/device/uevent`、`/sys/class/drm/renderD128/{dev,uevent}`、
  `/proc/bus/pci/devices`（空表）。这是 Chromium/Electron 与 GStreamer VA 认可 GPU 的前提；
  DRI3 零拷贝仍未通（evidence 165）。
- **假 DRM 节点的 dup 记账**：`dup`/`dup2`/`dup3` 与 `fcntl(F_DUPFD)`/`fcntl64` 以前不认假 fd，
  复制出来的渲染节点（`rdev=0:0`、memfd）上的 DRM ioctl 全部落空，客户端退回软件渲染。
  现在副本也报 `rdev=10:33`（evidence 166）。
- **libc 内部调用少打 5 个 hook 而一直没人发现**：105 个 hook 里
  `__readlink_chk`、`__readlinkat_chk`、`__realpath_chk`、`__mq_open_2`（入口窗口里有分支跳进来）
  与 `__openat64_nocancel`（这个 libc 根本没导出）一直被 interposer 拒绝，旧的 shell 脚本不做
  集合比对，所以"装上了"和"少 5 个"看起来一样。应用层这 5 个名字仍由 shim 的 GOT 覆盖，
  libc **内部**的直调仍然落到 host —— 这是已知缺口，本版只是把它变成会失败的断言，没有修
  （evidence 170 §3）。
- **发布启动器包里没有 GPU 默认值**（打包缺陷）：`make dist` 打的 `husk-arm64.tar.gz` 一直
  没带 `guest-config/husk-gpu-defaults.sh`，而安装器那个 `if [ -f … ]` 没有 else，于是从发布包
  装出来的树**没有** `/etc/profile.d/husk-gpu.sh`、也没有任何提示。现在包里带上了，`make dist`
  在打完的 tar 上断言它存在，安装器缺文件时打 WARNING（evidence 172）。
- **`make-release.sh` 会悄悄打出带 `tmp/proc/dev/run/root` 的包**（打包缺陷）：排除表用
  here-document 写，而 mksh 的临时目录在 `run-as` 下不可写，脚本只有 `set -u`，于是排除表是空的、
  `tar -X` 拿到空表。改成 `printf` + 非空检查 + 失败即退出，并加前置拒绝：
  树里出现 `data`/`storage`/`sdcard`（guest 里造出来的宿主路径自镜像）就不许打包（evidence 172）。
- **假 DRM fd 只进不出**：`close()` 从不注销 DRM 假 fd，8 个槽位用满之后新开的节点不再被接管；
  `close()`/`close_range()` 现在会注销，槽位真的用完时会打一条日志（evidence 166）。

### 测过

- **桌面 30 分钟空载长稳**：PSS 137.3 → 139.9 MB（前 60 秒启动分配，之后 29 分钟 +11 KB），
  线程 33 / fd 41 恒定、僵尸 0、同一 pid；结束后 X 根窗口 2400×1080 仍 100% 非黑
  （evidence 169）。八维第 7 维补上 GPU 桌面这一行；睡眠/唤醒与负载型 soak 仍未测。
- **桌面在重绘，不是静态位图**：同一会话四帧根窗口指纹，空载两帧逐字节相同、开关一次
  GNOME 概览各换一帧（evidence 171）。

### 测过

- **发布产物端到端**：`make dist` 的启动器包 + 从验证过的树打出的 rootfs tar → 全新目录装一遍
  → 门禁 A **34 PASS / 0 FAIL**（少的 2 项是发布包不含 apt 缓存、`dpkg-deb` 重装那两条被跳过，
  不是失败）。树包 102 110 条目、1.42 GB，设备侧与主机侧 sha256 一致（evidence 172）。

### 工具

- **libc 补丁链 Rust 化**：`nr-tools libc-patch`（`make libc-patch` / `make libc-verify`）
  取代 `tools/guest-config/install-libc-patch.sh`（已删）。除了原来那两步变换，它把"结果对不对"
  变成断言：glibc 版本绑定、slot 集合必须等于 `libc-interpose.expected`、`clone3` 归零、
  `rseq` 不被动、4 KiB/16 KiB/64 KiB 页都对齐；失败就中止，不再出现"推了一半也算成功"。
  同一份 stock libc 打出来的产物与仓库里 `tools/loader/libc-nop.so.6` **逐字节一致**（evidence 170）。
- `gl-probe root [out.ppm]`：用 `XGetImage` 读 X 根窗口并写 PPM，再打印非黑像素与直方图。
  手机 `screencap` 抓的是 App 自己的 UI，不能当 guest 出帧的证据。
- `gl-probe window` 现在打印 swap 循环的耗时与 fps（`stage 10: 60 swaps OK (506.8 ms, 118.4 fps)`），
  可用于 GPU / CPU 两条路径的 A/B。

### 文档

- `docs/BENCHMARKS.md` 补上第 5 维（桌面与图形）与第 6 维（交互延迟）的数字，并把
  「图形不进表」那段替换成这些数字能说明什么、不能说明什么（evidence 168）。
- `docs/LIMITATIONS.md` §7 更新：WSI 已经通了但呈现是 SHM 拷贝；桌面由 GPU 出帧，
  但轻负载 fps 不如 llvmpipe（照实记录）。

## [0.0.2] - 2026-11-24

这一版没有新功能，只有「装得上、跑得稳、说得准」。全部结论来自目标机实测
（`docs/design/evidence/156`、`157`）。

### 修复

- **独立安装（不装 APK）真的能用了**：`/data/local/tmp` 里从零装、起 `bash`、`apt-get update`
  拉 38.1 MB、`apt-get install` rc=0、程序能跑（evidence 157）。此前有六个问题挡着：
  `make dist` 打出 644 的 loader；toybox `gzip -dc` 写完仍以 1 退出把好包判成坏包；
  分派漏了 plain tar 分支；启动器把 `-c` 当程序名；缺 `etc/resolv.conf`；缺 `var/log/apt`。
- **安装器不再吞错误**：pass 1 的 `tar … | sed … || true` 把 tar 的退出码丢进管道，
  坏包可以"装成功"。现在落日志再判码，只容忍 SELinux 拒 `link(2)` 这一类，其它 exit 68。
- **绝对符号链接在建装时就被规范化**：新增 `husk-symlinks`（Rust），`--apply` 改相对、
  `--verify` 确认没漏网；再用 `ctypes.CDLL('libblas.so.3')` / `liblapack.so.3` 验 soname。
  实测 29 476 条链接、5 条绝对、改 4 条、1 条悬空、0 条残留。
- **libc 被 `apt upgrade` 换掉会被发现并恢复**：`.husk-install-state` 记 libc 与 loader 的
  sha256，`husk-session.sh` 每次开会话前比对；能恢复就恢复，不能就 exit 74 并说清原因。
- **`make check` 名不副实**：它只做 cargo check，别的门禁散在 `lint`/`docs`。现在
  `check` = fmt + cargo check + clippy + 53 个脚本 `sh -n` + 文档链接 + 版本三处一致。
- **许可证元数据冲突**：workspace 写 `MIT`、LICENSE 是 Apache-2.0，14 个 crate 还各自写死 MIT。
  统一成 `license.workspace = true` + Apache-2.0。

### 变更

- **GPU 接进 0.0.1 之后的树**：kbase 后端的 PanVK（FristOneRR）与它的 ICD 进包，
  `vulkan-probe` 枚举到 `Mali-G77 MC9` 且 compute 回读 `0x4855534b`；会话默认把 GL 交给
  Zink，renderer 从 `llvmpipe` 变成 `zink Vulkan 1.3(Mali-G77 MC9)`（evidence 156）。
  `HUSK_GPU=llvmpipe` 可退回软件路径。**上限仍是 GLES 2.0，窗口呈现（WSI）未做。**
- `make dist` 打 `tools/loader/libc-nop.so.6`（与目标机树上一致），不再打 `binaries/` 里
  11-22 的旧件；包里增加 `husk-symlinks` 与 `VERSION`。

### 文档

- LIMITATIONS §7/§8 按实测重写，并补一张「能力 → 状态 → 证据」表；COMPATIBILITY 补 GPU 段；
  BENCHMARKS 写清 `make -j4` 两个数字（5 194 / 3 509 ms）为何不可互算。
- README 与 SECURITY 明写「不是恶意代码沙箱」，列出 raw syscall、静态 ELF、非 loader 进程
  三类不受控路径。
- 顶层文档全中文；去掉仓库里的设备序列号与本机代理地址；删掉多余的 `v0.1.0` 标签。

### 更正

- 本版最初按「小数位加一」写成 `0.02` / `v0.02`，两段式不是合法 SemVer；发布当天更正为
  `0.0.2` / `v0.0.2`，上一版同理（`0.01` → `0.0.1`）。代码与资产内容不变，只有版本字符串改了。
  规则现在写死在 `CONTRIBUTING.md`：patch 位加一，**发出去的 tag 不再改名、不再移动**。

## [0.0.1] - 2026-11-24

首个发布版本。参考环境：arm64、Android 13、kernel 4.19.191、4 KiB 页，以 `untrusted_app`
运行，无 root。机型与 SoC 只是那一次实测的现场，不是运行时的要求。

### 新增

- **原生运行时。** 一个内嵌 path shim 的 aarch64 loader，让未经修改的 glibc 用户态跑在同一个
  进程树里：无 `ptrace`、无 namespace、无虚拟化、无 root。路径虚拟化在装载时做一次——改写
  每个被装载对象的 GOT——之后程序自己的 `svc` 原样进内核（evidence 130–146）。
- **包管理器可用。** `apt-get install` 27 秒把树从 815 包装到 882 包，含 `build-essential`、
  `git`、`vim`、`nodejs`、`python3-numpy`（evidence 154）。
- **工具链完整。** `gcc`、200 目标工程的 `make -j4`、`git clone`、`python3` + `numpy`
  （evidence 155）。
- **会话服务可用。** polkit 授权（`pkcheck` rc=0）、at-spi 无障碍总线、gvfs 不回退到会话总线、
  RandR 1.2（`xrandr` 无缩放告警）（evidence 151、152）。
- **三运行器基准夹具**（`tools/bench/`）：同一份 rootfs 交给 Husk / proot / proot-distro，
  只换运行器，数字见 [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)。
- **独立安装形态**：`tools/guest/install-husk.sh` 装一份 rootfs 并生成 `./husk` 启动器，
  不需要 JVM、不需要 Python（[`docs/RUNNING.md`](docs/RUNNING.md)）。
- **版本自证**：`./husk --version` 打印 `VERSION` 文件里的版本号，`make version-check` 保证它
  与本节标题、release tag 三处一致。

### 变更

- Android 侧集成里的 proot 已移除：同一份 rootfs 只经 loader 启动，功能没有损失
  （evidence 144）。

### 修复

- shim 缺 `statvfs` / `statvfs64`，导致 `apt` 量的是宿主路径的空闲空间（evidence 154）。
- 发行版自带的静态 `ldconfig` 看不见 guest，换成 `husk-ldconfig`（evidence 154）。
- `execve` 把 argv 放在 64 槽的栈数组里，第 63 个之后的参数被静默丢掉；表现为 200 目标工程
  的 `make -j4` 报了指向 `__TMC_END__` 的链接错误，而不是指向 argv（evidence 155）。
- rootfs 里 552 个绝对符号链接按 Android 根解析，改成相对链接后 `numpy`、`libblas`、
  `liblapack` 才能按 soname 装载（evidence 155）。
- `nr-sigsys.log` 噪声：一次会话一行，而不是 4 499 行（evidence 152）。

### 运行时上限（不属于本版交付能力）

这些是运行时被验证到过的上限，**0.0.1 交付的资产里不含桌面那一层**，它们由应用侧提供：

- GNOME Shell 46 的 X11 会话能在 loader 下起完整一帧（evidence 147）。
- 触摸经 `TouchInputHandler` → 进程内 X server → XI2 → mutter 落到窗口（evidence 148）。

### 已知问题

- 桌面渲染是软件渲染。guest 的 GPU 路径存在，但没接到桌面那条链上
  （[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) §7）。
- 传感器、GPS、USB、蓝牙、NFC、指纹、硬件视频解码、WSI：未开始。
- 随附 rootfs 里没有浏览器包；Termux 不是支持的环境。

[未发布]: https://github.com/xishanp/Husk/compare/v0.0.25...HEAD
[0.0.25]: https://github.com/xishanp/Husk/releases/tag/v0.0.25
[0.0.24]: https://github.com/xishanp/Husk/releases/tag/v0.0.24
[0.0.23]: https://github.com/xishanp/Husk/releases/tag/v0.0.23
[0.0.22]: https://github.com/xishanp/Husk/releases/tag/v0.0.22
[0.0.21]: https://github.com/xishanp/Husk/releases/tag/v0.0.21
[0.0.20]: https://github.com/xishanp/Husk/releases/tag/v0.0.20
[0.0.19]: https://github.com/xishanp/Husk/releases/tag/v0.0.19
[0.0.18]: https://github.com/xishanp/Husk/releases/tag/v0.0.18
[0.0.17]: https://github.com/xishanp/Husk/releases/tag/v0.0.17
[0.0.16]: https://github.com/xishanp/Husk/releases/tag/v0.0.16
[0.0.15]: https://github.com/xishanp/Husk/releases/tag/v0.0.15
[0.0.14]: https://github.com/xishanp/Husk/releases/tag/v0.0.14
[0.0.7]: https://github.com/xishanp/Husk/releases/tag/v0.0.7
[0.0.6]: https://github.com/xishanp/Husk/releases/tag/v0.0.6
[0.0.5]: https://github.com/xishanp/Husk/releases/tag/v0.0.5
[0.0.4]: https://github.com/xishanp/Husk/releases/tag/v0.0.4
[0.0.3]: https://github.com/xishanp/Husk/releases/tag/v0.0.3
[0.0.2]: https://github.com/xishanp/Husk/releases/tag/v0.0.2
[0.0.1]: https://github.com/xishanp/Husk/releases/tag/v0.0.1
[0.0.8]: https://github.com/xishanp/Husk/releases/tag/v0.0.8
[0.0.9]: https://github.com/xishanp/Husk/releases/tag/v0.0.9
[0.0.10]: https://github.com/xishanp/Husk/releases/tag/v0.0.10
[0.0.11]: https://github.com/xishanp/Husk/releases/tag/v0.0.11

[0.0.13]: https://github.com/xishanp/Husk/releases/tag/v0.0.13
[0.0.12]: https://github.com/xishanp/Husk/releases/tag/v0.0.12
