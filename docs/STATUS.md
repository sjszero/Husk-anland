# 当前状态（2026-11-23 06:55）

> 本节随进展重写；下面的「附录」是按时间倒序的历史记录，保留不动。
> 只写**已上机验证**的结论，未验证的明确标注。

## 一句话

在 RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4 KiB 页）上，普通应用域（`untrusted_app`）
里跑起了 glibc bash 交互会话；壳应用是纯 Rust NativeActivity（无 Java）；GPU 侧已由
FristOneRR PanVK 在 guest 内报出 **Mali-G77 MC9**，纯 Rust 探针完成 logical device、queue、
host-visible storage buffer、compute job 和离屏 graphics draw；GPU 写回 `0x4855534b`、
像素回读 `0xff0000ff` 均已由 fence 后确认。发行版 Zink 也已在 surfaceless EGL 报出
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。显示链路已从静态单帧推进到 3-buffer ring：
evidence 54 完成 12 帧，evidence 55 用 eventfd completion bridge 完成 120 帧，SurfaceFlinger
接受 acquire fd，release fence 背压和 fd 回收均通过；evidence 56 又把同一路径跑到 5,600 帧 /
134 秒，app 与 guest 均全量完成。evidence 57 已在 guest 内直接生成真实 pending `sync_file`
（`status 0 -> 1`，同一 fd pending -> readable）。evidence 58 已把它接到 FristOneRR PanVK 的
`vkGetFenceFdKHR(SYNC_FD)`：同一 3-buffer SurfaceControl ring 连续 120 帧全部使用真实
`mali-mali.timeline*` fd，其中 11 帧导出时仍 pending，app `120/120` transaction complete，
无 eventfd fallback。evidence 59 已把每帧重建改为 3-slot 持久资源池，并连续跑完
**10,000 帧 / 665.385 秒**：真实 `SYNC_FD` 10,000/10,000，completion 与 9 点回读均
10,000/10,000，fallback=0，app transaction 10,000/10,000；app/guest fd 与 RSS 无累积增长。
**eventfd 仅保留为 fallback；WSI / swapchain、窗口生命周期和 SurfaceFlinger 内部等待 trace
仍未完成。** 可靠性审计（evidence 64）修复了路径解析 fd 泄漏、超长路径、open/stat/access 的最终 symlink
TOCTOU、AArch64 `O_NOFOLLOW` 常量和 `faccessat(AT_SYMLINK_NOFOLLOW)` 语义；新增 Rust
路径探针 8/8 通过。`chmod`/`chown`/`utimensat` 等 follow-final `pathop` 仍有待收口的竞态面；
GNU Hash 完整映射校验、GOT 失败注入和 16K/64K 实机运行仍是 **UNKNOWN**。

## acquire fence 当前实验（PanVK → SurfaceControl 已通过，WSI 未完成）

- **FACT**：PanVK 报告 `VK_KHR_external_fence` / `VK_KHR_external_fence_fd`；`vkGetFenceFdKHR`
  只能经 `vkGetDeviceProcAddr` 取得，`dlsym` 直接查找不存在。
- **FACT（旧实现）**：FristOneRR PanVK 旧的 kbase CPU-sync 后端会先 CPU 等待 GPU seqno；submit 后导出得到
  `rc=0 fd=-1`，且 `SYNC_FD` 导出会重置原 fence。导出后继续 wait 同一个 fence 会挂起，禁止重跑；
  evidence 58 已用新的 out-fence stream 路径修正。
- **FACT（evidence 55）**：改为 Rust completion 线程等待 `VkFence`，GPU 完成后写 eventfd；
  将 `READY <slot> <frame> <expected>` 与 eventfd 通过同一个 `sendmsg(SCM_RIGHTS)` 交给 app。
  3 个 slot 连续 120 帧通过：app `submitted=120 completed=120`，guest
  `rendered=120/120 releases=117`，最终 `buffers_released=3`。
- **FACT（evidence 55）**：SurfaceFlinger 在当前 Android 13 / RMX3357 路径上接受该 eventfd 并
  完成 transaction；release fence 背压保持，app fd 数由 ring 前 141 降至 cleanup 后 129，
  没有按帧增长。
- **FACT**：eventfd 是 pollable completion bridge，不是 `sync_file`；本次多数帧在 app 收到
  READY 前已经完成，不能把它写成“SF 等待 pending producer fence”的真实测量。
- **FACT（evidence 56）**：同一 eventfd 路径完成 3-buffer / 5,600 帧 / 134 秒短 soak；
  app `submitted=5600 completed=5600`，guest `rendered=5600/5600 releases=5597`，无 `FAIL`，
  但这不是 10 分钟门禁，也不能证明持久资源池。
- **FACT（evidence 57）**：64-byte `base_jd_atom_v2` 的
  `SOFT_EVENT_WAIT -> SOFT_FENCE_TRIGGER(ORDER dep)` 已在 glibc guest 内生成真实
  `mali-mali.timeline*` sync_file。set 前 `SYNC_IOC_FILE_INFO.status=0` 且 poll 250 ms 超时；
  set 后同一 fd readable、status=1，两个 atom 均为 DONE。
- **根因（FACT）**：旧探针一是混用 64/72-byte atom 字段，二是把 `SAME_VA` 的 pre-mmap cookie
  当成 GPU VA；mmap 后的映射地址才是 GPU VA。`MEM_ALLOC_EX` 不是必需项。
- **FACT（evidence 58）**：FristOneRR 的 kbase 后端新增 JM out-fence stream；`vkGetFenceFdKHR`
  已对真实 GPU atom 返回 `mali-mali.timeline*` sync_file。3-buffer ring 连续 120 帧全部为
  `kind=SYNC_FD`，其中 11 帧在导出时 `PENDING`、109 帧已 signal，120 帧 completion 后均为
  `status=1`；120 次 9 点 GPU 回读 PASS，eventfd fallback = 0，guest/app `FAIL` = 0，
  app `submitted=120 completed=120` 且最终回收 3 个 buffer。
- **FACT（evidence 59）**：每帧创建/导入/销毁 Vulkan 资源已改为固定 3-slot 资源池：
  `image_imports=3`、`readback_allocations=3`，逐帧 `image_create/image_import/image_bind`
  日志为 0。10,000 帧 / 665.385 秒全部为真实 `SYNC_FD`；7,106 帧导出时仍 `PENDING`，
  completion 与 9 点 GPU 回读均 10,000/10,000，eventfd fallback=0，app transaction
  10,000/10,000，最终回收 3 个 AHB。采样期间 app fd 145–148、RSS 141,972→140,392 KiB；
  guest fd 恒 21、RSS 26,996→26,932 KiB，无累积增长。
- **UNKNOWN / 未完成**：没有 SurfaceFlinger 内部 trace，不能声称每个 pending 帧都被 SF 显式
  等待；能确认真实 sync_file 已交给 `ASurfaceTransaction_setBuffer` 且 transaction 完成。
- **未完成**：`VK_KHR_swapchain` / Wayland / X11 WSI、旋转 / DPI / 多窗口、SurfaceView 重建；
  不能把本实验写成完整显示验收。
- 详细设计、禁止重跑的失败组合和下一阶段验收见
  [`docs/design/acquire-fence.md`](design/acquire-fence.md)；完整证据见
  [`docs/design/evidence/55-eventfd-acquire-fence.md`](design/evidence/55-eventfd-acquire-fence.md)、
  [`docs/design/evidence/57-pending-kbase-syncfile.md`](design/evidence/57-pending-kbase-syncfile.md)、
  [`docs/design/evidence/58-panvk-syncfd-ring.md`](design/evidence/58-panvk-syncfd-ring.md)、
  [`docs/design/evidence/59-panvk-pooled-syncfd-soak.md`](design/evidence/59-panvk-pooled-syncfd-soak.md)。

## SoC 直连最新（evidence 58，2026-11-23）

- **Vulkan 扩展面已实测（evidence 48，FACT）**：PanVK 实例扩展 18 项、设备扩展 171 项；
  `VK_KHR_external_memory_fd` / `VK_EXT_external_memory_dma_buf` 等 dma-buf 导入路径存在，
  但 `VK_KHR_android_surface` 与 `VK_ANDROID_external_memory_android_hardware_buffer` 缺失。
  因此 guest 不能直接建 Android Surface 或直接导入 AHB，必须走 app 侧 AHB → dma-buf fd →
  `SCM_RIGHTS` → guest Vulkan 导入；这条跨进程握手已由 evidence 49–51 实测打通。
- **surfaceless EGL/Zink 已通过（FACT）**：`eglinfo -B` 和纯 Rust `gl_probe` 都报出
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；壳应用的 guest bash 实际使用 `rootfs-full`，
  子进程环境已注入 PanVK ICD + Zink。GBM / Wayland / X11 / swapchain 仍未通。
- **壳应用环境已验证（FACT）**：`/proc/<guest bash>/environ` 含 `VK_ICD_FILENAMES`、
  `VK_DRIVER_FILES`、`__EGL_VENDOR_LIBRARY_FILENAMES`、`MESA_LOADER_DRIVER_OVERRIDE=zink`、
  `GALLIUM_DRIVER=zink`；logcat 显示会话与窗口尺寸正常。
- **零拷贝静态单帧已验证（evidence 53，FACT）**：纯 Rust JNI 找到当前 Activity，挂接平台
  `SurfaceView`，读取其 BLAST `mBlastSurfaceControl.mNativeObject`；`ASurfaceControl_create`
  + `ASurfaceTransaction_setBuffer` 将 guest 已写红的同一 AHB 直接提交给 SurfaceFlinger。
  截图正文 24,192/24,192 个采样点为纯红，SF 层树确认 `husk-ahb` BufferStateLayer 可见且为
  `DEVICE` 合成。
- **连续帧 / 背压已验证（evidence 54，FACT）**：app 一次分配 3 个 AHB，guest 连续 GPU 渲染
  12 帧；app `submitted=12 completed=12`，guest `rendered=12/12 releases=9`，3 个 slot 各复用
  4 次。app 用 `ASurfaceTransaction_setOnComplete` 取得上一 buffer 的 release fence，实测 fd
  `143/153/150` 均 signal 后才向 guest 发 RELEASE；最终蓝色截图正文 37,800/37,800 采样点纯蓝。
- **eventfd acquire bridge（evidence 55，FACT）**：同一 3-buffer ring 连续 120 帧通过；app 每个
  `READY` 都收到 1 个 fd，`completed=120`；guest 渲染 120 帧、收到 117 次 release，最终 3 个
  buffer 全部归还。该路径通过 SELinux 和 SurfaceFlinger，但不是 Vulkan `SYNC_FD`。
- **展示回退保留（evidence 52，FACT）**：app 也可用 `eglGetNativeClientBufferANDROID` +
  `eglCreateImageKHR` + `glEGLImageTargetTexture2DOES` 将同一 AHB GPU 采样上屏；该路径仍作为
  SurfaceControl 字段缺失时的降级，不冒充零拷贝。

## 显示零拷贝六段（evidence 49–55，2026-11-23）

- **FACT**：纯 Rust app 进程成功分配 `64x64 RGBA8` AHardwareBuffer；带 CPU 读写能力的探针版
  为 usage `0x322`、stride 64。
- **FACT**：`AHardwareBuffer_getNativeHandle()` 返回 3 个 fd，其中一个由
  `/proc/self/fd` 确认为 `/dmabuf:G:pid [...]`；native-handle fd 顺序不能写死，必须扫描。
- **FACT**：真实 AHB dma-buf 经 `AF_UNIX/SOCK_SEQPACKET` + `SCM_RIGHTS` 跨进程进入 glibc guest；
  PanVK 报告 DMA_BUF buffer 能力 `compatibleHandleTypes=0x201`，external buffer 创建、
  `VkImportMemoryFdInfoKHR` 导入、bind 全部成功，GPU compute 写入并回读 `0x4855534b` PASS。
- **FACT**：app 通过 AHB CPU view 写入 9 个 cookie，guest 直接 `mmap` 同一 dma-buf 后按
  `DRM_FORMAT_MOD_LINEAR` / offset 0 / rowPitch 256 读回 9/9，证明本机新分配 AHB 是线性布局。
- **FACT**：PanVK 以 `VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT` +
  `VkImageDrmFormatModifierExplicitCreateInfoEXT` 成功创建 LINEAR 2D image，导入 dma-buf、
  bind image 均成功；GPU `vkCmdClearColorImage` 后 staging readback 9/9 红，直接 dma-buf
  CPU mmap 也 9/9 红。
- **FACT**：重复使用同一个 glibc fd 分别 import buffer 和 image 会让 image import 报
  `invalid dmabuf size`；每次 import 独立 `dup(fd)` 后两条路径同轮通过。驱动内部精确原因仍为
  UNKNOWN，不把推断写成根因。
- **FACT（evidence 53）**：app 侧 `ASurfaceControl` 已把同一 AHB 零拷贝提交并上屏；SurfaceFlinger
  层树里 `husk-ahb` 有 64x64 RGBA_8888 activeBuffer，显示区域为 `[0,120,1080,2359]`，
  合成类型为 `DEVICE`。
- **FACT（evidence 54）**：3-buffer ring 连续 12 帧、slot 复用、release-fence 背压均通过；这不是
  单帧演示。
- **FACT（evidence 55）**：eventfd acquire bridge 连续 120 帧，app 收到 fd、SF 完成 transaction、
  release 背压和资源回收通过；这只证明当前 SurfaceControl 消费路径可用。
- **FACT（evidence 56）**：3-buffer eventfd ring 连续 5,600 帧、134 秒，guest image_complete
  5,600/5,600，app 5,600/5,600，无 FAIL；但未达到 10 分钟门禁，且每帧仍重建 Vulkan 资源。
- **FACT（evidence 58）**：PanVK `vkGetFenceFdKHR(SYNC_FD)` 已返回真实 sync_file；
  同一 3-buffer SurfaceControl ring 连续 120 帧全部使用真实 fd，其中 11 帧导出时 pending，
  120 帧 completion 后均 `status=1`，app `120/120` transaction complete，eventfd fallback 为 0。
- **FACT（evidence 59）**：3-slot Vulkan 资源池连续 10,000 帧 / 665.385 秒，真实 `SYNC_FD`
  10,000/10,000；7,106 帧导出时仍 pending，completion/回读 10,000/10,000，fallback=0，
  app transaction 10,000/10,000；app/guest fd 与 RSS 无累积增长。
- **UNKNOWN / 未完成**：SurfaceFlinger 内部是否显式等待每个 pending 帧尚无 trace；
  GBM / Wayland / X11、WSI / swapchain、旋转 / DPI / 多窗口、SurfaceView 重建仍未验证，
  不能写成“显示目标完成”。

## GPU 基线（evidence 46，2026-11-23）

- **FristOneRR PanVK 已产出 glibc 版**：源码 commit
  `0c9dce35094dbe89c412b79bf57c6067c7b97e59`，Mesa 26.3.0-devel；产物
  sha256 `fbfa3b43e69d3fd7f4088fe6b9e94a621e19ca3f95502265281f92ba908b6147`。
- **真实物理设备可见（FACT）**：`vulkaninfo --summary` exit 0，报 `deviceName=Mali-G77 MC9`、
  `deviceID=0x90800011`、`driverID=DRIVER_ID_MESA_PANVK`、`apiVersion=1.3.354`。
- **纯 Rust `vulkan-probe` 已通过（FACT）**：instance / physical / queue family / logical device /
  queue / memory types / 4 KiB buffer allocate+bind+map+readback / teardown 全部 PASS。
- **compute job 已通过（FACT）**：`vkCmdDispatch` + `vkQueueSubmit` + fence 后，GPU 将
  `0x4855534b` 写入 storage buffer；修复了 `MEM_EXEC_INIT` 在 JIT 前初始化的顺序问题。
- **离屏 graphics 已通过（FACT）**：真实顶点缓冲、vertex/fragment SPIR-V、render pass、
  `vkCmdDraw`、image-to-buffer copy + fence 后回读红色像素 `0xff0000ff`，连续两次 PASS。
- **evidence 46 当时未验证（FACT）**：swapchain、WSI、Android Surface 和实际窗口显示尚未打通；
  后续 evidence 52 用 EGLImage 回退上屏，evidence 53 用 SurfaceControl 完成静态单帧零拷贝上屏，
  evidence 54 完成 3-buffer ring 连续 12 帧与 release-fence 背压，evidence 55 用 eventfd
  completion bridge 完成 120 帧并把 acquire fd 交给 SurfaceFlinger；evidence 57 已生成真实
  pending kbase sync_file，evidence 58 已由 PanVK `vkGetFenceFdKHR(SYNC_FD)` 接入同一
  SurfaceControl ring 并连续跑通 120 帧；evidence 59 已完成 3-slot 持久资源池与
  10,000 帧 / 665.385 秒长稳；WSI/swapchain 仍未验证。
- **保留对照（FACT）**：`funnymdzz/mesa` `kbase-v0.1.2` 在 `vkCreateDevice` 处因 kbase fd 上
  `drmSyncobjCreate()` 失败而报 `VK_ERROR_OUT_OF_HOST_MEMORY`，不再作为主动路线。

## 上一阶段 GPU 基线（evidence 45）

- **假 `/dev/dri/renderD128` 全链路打通**：loader 里的 path-shim 与 LD_PRELOAD 层通过
  `HUSK_DRM_API` 函数表共用一份 kbase 状态；`open → gbm_create_device` 全 PASS。
- **Mesa panfrost gallium 已在 guest 内加载**，并通过 Rust DRM 语义层从 kbase 读到
  真实 GPU 参数（PROD_ID `0x9080` = Mali-G77 等 15 项 ×2 轮）。
- **卡点（FACT）**：`panfrost_open_device()` 在发出任何 `CREATE_BO` 之前失败，
  `egl: failed to create dri2 screen`；Ubuntu Mesa 25.2.8 的模型表没有 G77/G78，伪装成 G57 仍失败。
- **路线调整（JUDGMENT，未完成）**：暂停把 panfrost + DRM 语义层当作唯一实现；
  当前主动路线改用前人已验证的 **panvk kbase fork（glibc 构建）+ 发行版 Zink**。
  DRM 语义层保留为 M1/M2 资产和后备，不删。
- 原始参考清单：`docs/design/reference-request-20261122.md`（12 类、120 个条目，逐字保存）。
  真实路径与结论：`docs/design/references.md`；逐条矩阵：`docs/design/reference-audit-matrix.md`
  （120/120：69 GitHub + 45 官方上游/组件 + 6 UNKNOWN；机器快照 JSON 同目录）。

## 能力表（FACT）

| 能力 | 状态 | 证据 |
|---|---|---|
| 普通应用权限下跑 glibc 用户态（`untrusted_app`） | 可用 | 12, 14, 17 |
| 功能矩阵（bash + coreutils + 管道/重定向/子 shell） | 31 / 31 | 17 |
| 越界与符号链接逃逸 | 0 escapes | 18 |
| gcc 13.3 全链（`cc1`/`as`/`ld`）+ make | 通过 | 16, 17 |
| 交互式会话（`bash -i`，PTY） | 可用 | 25 |
| 纯 Rust 壳应用（`app-rs`，NativeActivity，`hasCode=false`） | 可用 | 28, 29 |
| 提示符 / 身份（`husk@husk:/$`、`id`/`groups`） | 通过 | 29 |
| 键盘输入 / 旋转 / 切后台恢复 / 备用屏 / 全屏 TUI | 通过 | 21:20 附录 |
| 僵尸与重启风暴 | 零复现（唯一 reaper + subreaper + 点了才重启） | 附录 21:00 |
| 页大小体检（`nr-tools check-elf` 4K/16K/64K） | 工具可用；loader 静态同余通过；整机页大小无关**未完成**（`libhuskapp.so` 64K FAIL） | 30, 64 |
| 路径边界可靠性（fd / 长路径 / 符号链接 / `faccessat`） | 当前 4K 设备通过；异常 ELF 与 GOT 失败注入仍 **UNKNOWN** | 64 |
| guest 打开 `/dev/mali0` + kbase 建 kctx | **可用**（UAPI 11.38） | 31 |
| 厂商 `libGLES_mali.so` 在 guest 内加载 | **不可行，已关闭**（25 个 bionic DT_NEEDED） | 31 |
| 长稳 10 分钟（goal A1 验收） | **通过**：600 秒 / 568 tick / 11 360 次 fork-exec，同 pid、零僵尸 | 32 |
| 2.0 完整 rootfs（Ubuntu 24.04.4，817 bins） | **可用**：GNOME Shell 46.0 / mutter / gjs 1.80.2 / dbus / Xwayland 23.2.6 均能起 | 33 |
| 带 shebang 的 guest 脚本（`#!/bin/bash`、`#!/usr/bin/python3`…） | **可用**：shim 的 execve 里做解释器映射 | 34 |
| guest 直连视频硬解 | **不可行，已关闭**（`/dev/video*` = `0660 media:system` 相机节点；厂商解码 .so 带 bionic DT_NEEDED） | 39 |
| 本机硬解器与容量 | `c2.mtk.{avc,hevc,vp9,av1}.decoder`；1080p 自报 avc 113–248 / hevc 125–276 fps（未实测） | 39 |
| 视频路径（设计定稿） | guest 内 Rust VA-API 后端驱动 `husk` → 通道 → app 侧 AMediaCodec；Chromium 与 GStreamer 共用一份后端，不改不重编 | 39 / goals v11 |
| 内核 kbase 版本（自报） | `K:r32p1-00bet5(GPL)`，与厂商 `libGLES_mali.so` 同代 | 40 |
| atom 结构 | 两套：64 B `base_jd_atom_v2`（atom_number@0x28/core_req@0x2c）或 72 B `base_jd_atom`（jc@0x08/atom_number@0x30/core_req@0x34）；stride 必须与结构一致 | 40 / 57 |
| timeline fd（`STREAM_CREATE`） | **不能当完成通道**：任何被接受的 submit 都让它 POLLIN，`read()` 返回 EINVAL | 40 |
| CSF / panthor 可用性 | **不可用**：`CS_GET_GLB_IFACE`（`0xc0188033`）= ENOTTY，内核只编了 JM | 41 |
| kbase dma-buf 导出 | **没有**；只有 `MEM_IMPORT`（`0xc0188016`，可达，ENOMEM）→ 零拷贝改成 app→guest 导入 | 41 |
| 完成通道 | `KBASE_IOCTL_KINSTR_JM_FD`（`0xc0088033`，16B 记录 / version 2）；但 COMPLETE 不区分成败，只有 START 有判据 | 40/41 |
| GPU 实现形态 | **当前主动路线**：FristOneRR PanVK（glibc）+ Zink；Mesa 25.2 panfrost + DRM 语义层保留为后备（evidence 46） | 36 / 45 / 46 |
| DRM 语义层（M1） | **可用**：假 `/dev/dri/renderD128` + CREATE_BO/MMAP_BO/GET_PARAM 全通（kbase MEM_ALLOC / GPU VA） | 43/44 |
| PanVK kbase 物理设备 | **通过**：guest 内 `vulkaninfo --summary` 报 Mali-G77 MC9 / PanVK | 46 |
| PanVK logical device + buffer | **通过**：instance、device、queue、4 KiB host-visible buffer map/readback | 46 |
| PanVK compute job | **通过**：SPIR-V shader + compute pipeline + `vkQueueSubmit` + fence + GPU readback | 46 |
| PanVK graphics | **离屏 draw 通过**：真实顶点缓冲 + vertex/fragment + draw + GPU 像素回读 | 46 |
| Zink surfaceless renderer | **通过**：`eglinfo -B` / 纯 Rust `gl_probe` 报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` | 47 |
| PanVK swapchain / WSI / Surface | **部分通过**：Android 窗口 3-buffer ring 连续 12 帧经 SurfaceControl 零拷贝上屏（evidence 54），eventfd fallback 连续 120 帧且 SF 接受 fd（evidence 55），同一路径完成 5,600 帧 / 134 秒短 soak（evidence 56），PanVK `vkGetFenceFdKHR(SYNC_FD)` 已在同一 SurfaceControl ring 连续 120 帧使用真实 fd（evidence 58）；3-slot 持久资源池完成 10,000 帧 / 665.385 秒长稳（evidence 59）；SurfaceFlinger 等待 trace、GBM / Wayland / X11、swapchain / WSI 仍未通 | 47 / 52 / 53 / 54 / 55 / 56 / 57 / 58 / 59 / 后续 |
| PanVK external fence | **通过（evidence 58 / 59）**：`vkGetFenceFdKHR(SYNC_FD)` 返回 `mali-mali.timeline*`；120 帧门禁后，持久资源池再完成 10,000 帧 / 665.385 秒长稳，7,106 帧导出时 pending，completion/回读 10,000/10,000，fallback=0 | 58 / 59 |
| kbase soft event / sync_file | **通过（evidence 57）**：64 B v2 `SOFT_EVENT_WAIT -> SOFT_FENCE_TRIGGER(ORDER)` 生成真实 `mali-mali.timeline*` sync_file；set 前 pending、set 后 readable，status 0→1 | 57 |

## 本轮（21:24–21:31）做了什么

1. 把积压的 3 个 commit 推上 GitHub（`origin/main` = `91ce86c`）。
2. **长稳**：在真机会话里注入 soak 脚本（每秒 20 次 fork/exec + 写一行文件），
   跑到第 277 tick（约 4 分 37 秒）时设备 **kernel_panic 重启**（21:30:05）。
   不是电量/过热：`ro.boot.bootreason=kernel_panic`。时间上紧接我的 kbase ioctl 盲扫，
   判断极可能是盲扫打崩的内核（细节与纪律见 evidence 31 §6）。**不可再盲扫内核 ioctl。**
3. **GPU 直连第一段取证**：新增 `runtime/crates/gpu-probe`（纯 Rust，guest 侧运行），
   量出 kbase 的真实命令字编码、UAPI 版本 11.38、`SET_FLAGS` 在 guest 内成功；
   同时证明 `/dev/kgsl-3d0` 不存在、`/dev/dri`、`/dev/snd`、`/dev/ion` 在 guest 侧都打不开。
   详见 [evidence 31](design/evidence/31-guest-soc-kbase-20261122.md)。
4. 目标文件升到 **v3**（`docs/GOALS.md`）：C 段按实测重写，厂商 EGL 路线标记关闭。

## 下一步（按依赖）

刚完成（21:47–21:58）：
- A1 长稳通过（evidence 32）；旋转期间也不掉（1080x2400 ↔ 2280x1080）；
- 2.0 完整 rootfs 铺好并跑起来（evidence 33）：`files/rootfs-full` 1.9 GB、
  用 `nr-tools rootfs --base/--debs` 纯 Rust 构建、本镜像自己的 loader + 补丁过的 glibc 2.39。

不依赖设备的：

1. 查清这台内核的 **GPU 提交接口**：nr=2 全 size ENOTTY，需确认是 CSF 模型还是 MTK 改号，
   这决定 panfrost/panvk 能不能真的提 job（只到“建 kctx”还不够）。
2. `nr-tools libc-interpose`：构建期槽位（设计已在 `docs/design/libc-interpose.md`），
   B 段 apt/dpkg/locale 的前置。
3. 终端渲染：SGR 颜色、UTF-8/宽字符、滚动回看。

依赖设备的：

1. 重跑 10 分钟长稳并写证据；
2. 完整 rootfs（2.0 资产）解到 `files/rootfs-full` 并起桌面；
3. `/dev/ion` 访问权复核（O_RDONLY / O_RDWR 各一次，guest 与 app 两侧对照）。

---

# 2026-11-22 深夜追加：壳应用生命周期 + 第二台设备诊断

## 修掉的真 bug（FACT，已上机验证）

**会话生命周期**：旧实现把"启动会话"放在渲染循环里，子进程一死就下一帧重新 `fork`，
而且 `waitpid` 只在 PTY 读到 EIO 时用 `WNOHANG` 试一次、失败就再没人回收。
结果：一次运行 2 秒起了 171 个 loader、堆出 700+ 个僵尸 `[libld.so]`。
你看到的"一直闪退、一直点开退出"就是它。

现在的模型（对照 proot / Termux / tini 的做法，用 Rust 实现）：

- `start_reaper()`：**全进程唯一**的 `waitpid(-1)` 线程，先 `PR_SET_CHILD_SUBREAPER`，
  bash 的孙进程死亡后也重挂回我们、由它收割；`EINTR` 重试，`ECHILD` 时 `usleep` 不空转。
- 状态回传：reaper → `EXITS` 队列 → 渲染线程 `poll_children()` → 只交给 pid 对得上的 session。
- 启动**一次**（`maybe_start`），死了就死着，**点了才重启**（`restart_session`，400ms 限流）。
- 会话死亡原因**写到屏幕上**：`[husk] session ended: signal 31 (SIGSYS)`。
- `read_available` 见到 EIO 只标记一次 + `close`，不再每帧刷日志。

验证（RMX3357）：`onCreate` → `run: thread up` → `spawned pid=11338` → `pty read errno=5; session closed`
→ `session pid=11338 ended: signal 31 (SIGSYS)`。**一次 spawn、零僵尸。**

## 这台机器（RMX3357 / Android 13 / 4.19.191 / 4K）的新事实

| 事实 | 结论 |
|---|---|
| loader 每次都能跑到 `.init_array`（`sigsys.log` 每次 spawn 精确 +33 字节） | 自举期没有 syscall 被拦，**旧结论"自举被 SIGSYS 杀"错误** |
| app 域 `openat2(437)` 实测 = **TRAP**，我们自己装的 handler 跑到了（`rt_sigaction ret=0`） | 这台 ROM 的 app seccomp **可捕获可仿真**，不是 KILL |
| guest 子进程仍然 `signal 31 (SIGSYS)`，而 shim 的 handler 从未产生二进制记录 | 问题在**我们 loader 里的 handler 没装上/没生效**，不是 ROM 限制 |
| `dmesg` 被拒、logcat 无 `type=1326` | 这台 ROM 拿不到 syscall 号，需用 ptrace 旁路观察或让 handler 自己报 |

## 下一步

1. 让 `install_sigsys` 打印 `rt_sigaction` 的返回值与 errno（现在无条件打 "installed"），确认是"没装上"还是"装上了被覆盖"。
2. `nr-tools` 加 `check-elf`（页大小体检），构建期断言 `p_offset ≡ p_vaddr (mod 0x4000)`。
3. 页大小设计见 [design/page-size.md](design/page-size.md)。

## 2026-11-22 深夜二：SIGSYS 根因锁定（ptrace 旁路取证）

新工具 `app-rs/src/trace.rs`（纯 Rust，ptrace "syscall snitch"）：存在 `files/trace` 时，
本次启动不走会话，而是 fork 一个 PTRACE_TRACEME 的子进程、按产品同样的 pty + `setsid`
+ `TIOCSCTTY` 形状 exec loader，父进程单步 `PTRACE_SYSCALL` 记录每个 syscall，
死亡时输出最后 12 个 syscall 号与未完成的入口号，并把 guest 自己的报错从 pty 捞进 `files/trace.err`。

实测（RMX3357，`bash -i`，322 个 syscall 后）：

    spy: tracee killed by signal 31 (SIGSYS) after 322 syscalls; last pending entry = -1
    spy: last enters (old -> new): 151 139 176 152 154 135 135 135 134 172 172 129

**最后一个 syscall 是 129 = `kill(2)`，且没有退出停点 —— 它就是被拦的那条。**

结论（FACT + 推论分开）：

- FACT：loader 自举、`.init_array` 装 handler（`rt_sigaction ret=0`）、加载 libc/bash 全部成功；
  `openat2(437)` 实测为 **TRAP 可捕获**；而 `kill(2)` 实测 **SIGSYS 直接杀，连 handler 带 ptrace
  都接不住** → 同一份 filter 里两种动作并存，`kill` 属于不可捕获的那一档。
- INFERENCE：触发点是 bash 的作业控制（`kill(shell_pid, SIGTTIN/…)`），不是 loader。
  这与"非交互 `bash -c` 在同样条件下能跑通"一致。
- 已尝试：在 shim 里 interpose `kill`/`tkill`/`tgkill`（自查目标 → 作业控制信号返回成功、指向别人 → EPERM），
  `nr-tools embed` 报 `interposed: 79`（+3），但**运行时 `kill` 仍进内核** → 说明该调用点的 GOT 没被补上，
  或调用发生在 shim 不覆盖的对象里。这是下一步要查的（`nr-tools/src/main.rs` 的 interpose 表如何应用到 guest DSO）。

## 页大小（补）

实测对齐：guest `bash`/`libc.so.6`/`ld-linux` 全部 `p_align = 0x10000`；我们的 `libhuskapp.so` `0x4000`；
`libld.so` 混合但满足 16K 同余。详见 [design/page-size.md](design/page-size.md)。

---

# 2026-11-22 21:00 追加：黑屏已修（真根因，已上机验证）

**FACT**：`ANativeActivity_onCreate` 里替换 `activity->callbacks` 指针无效——平台在我们返回后
把它指回自己的静态表，所以整张回调表（`onStart`/`onResume`/`onInputQueueCreated`/
`onNativeWindowCreated`）一条都收不到。窗口有 surface、`HAS_DRAWN`，但那是 WindowManager
自己画的背景，我们从未拿到 `ANativeWindow`。

修法：**原地覆盖平台给的那张 128 字节回调表**，不动指针。实测现在能收到
`onStart` / `onResume` / `onInputQueueCreated` / `onNativeWindow win=... 1080x2400`，
`bash -i` 的提示符和光标正常显示在屏幕上。

证据：[evidence 28](design/evidence/28-nativeactivity-callbacks-black-screen-20261122.md)

同时把三个诊断点固定下来（每个回调一行 `cb: ...`、等窗口时打一次 `waiting for onNativeWindowCreated`、
`EGL surface failed` 带窗口地址），以后黑屏能一次分清是「没窗口」「0x0 窗口」还是「EGL 建面失败」。

界面仍待办（都是上面这次修复**之后**才看得见的问题）：
提示符前 `I have no name!`（rootfs 里没有本 uid 的 passwd 条目）、`pwd` 是 `/root` 不是 `/`、
`groups` 告警、软键盘要点一下才出、旋转/Surface 重建、`vim`/`htop`。

---

# 2026-11-22 21:10 追加：身份/提示符修好；libc 内部调用绕过 interpose（重要）

**FACT**：`bash -i` 现在提示符是 `husk@husk:/$`，`groups`/`id` 不再刷
`cannot find name for group ID`。做法见 [evidence 29](design/evidence/29-nss-identity-20261122.md)：
身份函数（`getpwuid`/`getgrgid` 及 `_r`/`get*nam` 共 8 个）由 shim 直接读 rootfs 的
`/etc/passwd`、`/etc/group` 回答；文件里没有的 Android 内核组 id 返回合成名 `husk<gid>`；
app 启动时把自身 uid/gid/补充组幂等写进这两个文件，并把 `PS1` 固定成 `\u@husk:\w\$`
（`\h` 是 `gethostname()`，Android 恒为 `localhost`）。

**新发现的架构性缺口（FACT）**：shim 的 interpose 只改 GOT 槽，libc **内部**的直接调用
绕不过去。所以 glibc 内建的 `files` NSS 读的是宿主 `/etc/passwd`；locale、`/etc/hosts`、
apt/dpkg 的状态文件同理。运行时改 libc 文本在本机**不可行**：`mprotect(rw)` 成功，但补完
`mprotect(r-x)` 被 EACCES 拒绝（app 域 W^X），整页失去执行权限后必崩（已在
`_IO_file_seekoff → lseek64` 上实测到）。因此文本修补必须在**构建期**做：把入口改成
`adrp/ldr/br`，目标地址放在新增的**可写** PT_LOAD 槽里，运行时只写数据段。
设计写在 `docs/design/libc-interpose.md`。

---

# 2026-11-22 21:20 追加：界面实测（可交互终端）

全部在 RMX3357 上实测，`bash -i` 跑在 `untrusted_app` 里：

| 项 | 结果 | 证据 |
|---|---|---|
| 首屏出字 / 光标 | 通过 | 截图：`husk@husk:/$` + 光标 |
| 键盘输入 → guest | 通过 | `input keyevent` 敲 `ls`/`top`/`less` 均执行 |
| 输入法自动弹出 | 通过（窗口获得焦点即请求） | `cb: onWindowFocusChanged focused=1` 后 IME 起来 |
| 旋转 | 通过 | 新窗口指针 2280x1080，GL/布局重建，内容保留 |
| 切后台再回来 | 通过 | `onPause` → surface 销毁 → 回到前台重新建面，**会话与输出都在**（不是重启） |
| 备用屏（`less`/`vim` 类） | 通过 | `less /etc/passwd` 退出后原位恢复，不污染会话 |
| 全屏 TUI | `top` 可用（不切备用屏，退出后顶部残留它自己的行） | `Tasks: 3 total, 1 running…` 渲染正确 |
| 标点/排版 | 已修 | 之前逗号/横线被画到下一行（字形双偏移），现在 `less`/`top` 的 `:` `,` `-` `/` `*` 全部正确 |

渲染侧这轮修掉三个真 bug：
1. `ESC ( B`（字符集设计）没被吞，`top` 每次重绘漏出一个 `B`；
2. 图集格子固定 32px 而屏幕格子 16×41，字形偏移被加了两次 → 标点错行；
3. 没有备用屏（`?1049h/l`），`less` 退出后画面残留。

O S C 现在也认 `ESC \` 结束符（否则会一直吞到下一个 BEL）。

**仍未做**：滚动回看的手势之外的键盘滚动、SGR 颜色、宽字符/中文（需要 UTF-8 解码 +
字体回退，当前只处理 ASCII）、选中复制粘贴、`SIGWINCH` 之外的尺寸变化通知。

---

# 2026-11-22 21:25 追加：A3 页大小（可查、不猜）

- 新增 `nr-tools check-elf <elf|dir>… [--page N]…`：按 `p_offset ≡ p_vaddr (mod PAGE_SIZE)`
  逐段判定，默认报 4K/16K/64K 三档，任一 PT_LOAD 不满足就 `FAIL` 并 exit 1。
- `nr-tools embed` 现在自带 16 KiB 同余断言，产物不可在 16K 内核上加载时**构建失败**。
- `boot.rs` 运行期不再假设 4096：入口从 auxv 的 `AT_PAGESZ` 读一次，`page_size()` 优先用它。
- 实测：guest 的 bash/libc/loader 三种页大小全通过（本来就 64K 对齐）；我们的 `libld.so`
  三种全通过；`libhuskapp.so`（NDK 29 = 16K 对齐）在 64K 页内核上会失败（Android 无此情况）。
  证据：[evidence 30](design/evidence/30-check-elf-20261122.md)。

## 2026-11-22 22:08 生命周期收口（载体级）

* 补全 NativeActivity 全部回调：`onStop` / `onSaveInstanceState`（返回 null）/ `onLowMemory`（裁 scrollback + 后台时丢 EGL）/ `onContentRectChanged`（记 insets 并触发重排）。
* `escalate` 加 pgid 复用防御：比较 `/proc/<pid>/stat` 的 starttime，pid 被回收就拒绝 `kill(-pid)`。
* `session:` 每行加 `fds=`（`/proc/self/fd` 计数）。
* 修两个真泄漏（设备实测抓到）：
  1. `Term` 没有 `Drop`：reaper 先报退出、master 还没读到 EIO 时被丢弃 → 每次重生残留 1 个 `/dev/ptmx` + 1 个 `pty.log`。现在 `Drop` 走 `close_master()`（先 `ALooper_removeFd` 再 close）。
  2. `Gfx` 没 `eglTerminate`：每次旋转/重建泄漏 swap-chain 的 `sync_file` / ashmem / gralloc_extra。
* 实测：旋转 `67x58 -> 142x26 -> 67x58` 同 pid；guest `stty size` = `58 67`；fds 启动 98 → 旋转 12 次后 167（每次重建约 +2，尚未判定为泄漏，需长跑）。
* 边界写清：运行时只管载体（进程/PTY/fd/信号/Surface 接入），GNOME/浏览器/DPI 属桌面层，不在运行时范围。

## 2026-11-22 22:12 C5 阻塞点解除（evidence 35）

* 从厂商 `libGLES_mali.so`（r32p1）反读出全部 kbase 命令字，确认 `JOB_SUBMIT = 0x40108002`（W,16）。
* guest 实测：`version_check -> 11.38`、`set_flags -> 0`、`job_submit(nr_atoms=0) -> EINVAL(22)`。
  EINVAL 而不是 ENOTTY ⇒ 命令可达，evidence 31 的"提交接口待查"是扫描方法错误造成的假阴性。
* 工具：`gpu-probe submit0`（新增，零 job 提交，不碰 21:30 panic 的那条路径）。
* 教训写进 evidence 35 §4：判定"内核不支持"必须先从真实客户端取命令字。

## 2026-11-22 22:20 C5 路线现实核对（evidence 36）

* FACT：上游 Mesa（25.2.0 与 main）的 panfrost 只有 DRM / Panthor 两种 kmod 后端，**没有 kbase**；
  Ubuntu 24.04 rootfs 的 libgallium 也只有 `DRM_IOCTL_PANFROST_*`，且没有 panvk ICD。
  ⇒ 原目标"走 Mesa 的 kbase 后端"在发行版 Mesa 上不可能，必须在 Rust 侧做 DRM 语义层。
* FACT：kbase `mem_alloc` 可用。in/out 结构：返回 0，字段0 写回实际 flags（0x200d），字段1 写回 GPU VA（0x41000/0x42000）。
* FACT：对这些 VA 做 `mmap(2)` 返回 EPERM，是当前最靠前的具体障碍。
* 下一步：从厂商 .so 反读它的 mmap 用法（offset 里的标志位），再对齐 mem_query。

## 2026-11-22 22:26 GPU 显存 CPU 映射打通（evidence 36 §5）

* kbase `mem_alloc(flags=0xf)` -> VA `0x41000`，`mmap(prot=3, MAP_SHARED, off=VA)` 成功，CPU 写 A5 读回一致。
* 关键规则（反汇编得出并实测）：`mmap` 的 prot 必须等于 `mem_alloc` 输入 flags 的低两位，否则 EPERM；
  对 PROT_READ 映射写会 SIGSEGV（不是 errno）。
* 仍未通：`mem_query`（结构布局），`job_submit`（atom 未构造）。

## 2026-11-22 22:47 JOB_SUBMIT 接受 atom（evidence 37）

* `job_submit {addr=用户态72字节全零, nr_atoms=1, stride=72}` → **errno 0**；`stride=0` 才是 EINVAL。
* atom 是否真被执行：**未验证**（缺完成通道），evidence 里已明确标注。
* `mem_query`（公开 UAPI 的 in{gpu_addr,query}/out{value}）三种 query 全 EINVAL，语义与公开头不符，待查。
* 下一步：接完成通道 + 用"GPU 写回内存"的最小 job 升级证据。

## 2026-11-22 22:52 kbase 命令字→名字（evidence 38）

* 26 条命令字与公开 kbase UAPI 头的 (nr,dir,size) 三元组逐条对上，得到名字：
  JOB_SUBMIT / MEM_ALLOC / MEM_FREE / MEM_QUERY / MEM_COMMIT / MEM_ALIAS / **MEM_IMPORT** / MEM_SYNC /
  **STREAM_CREATE** / FENCE_VALIDATE / **SOFT_EVENT_UPDATE** …
* MEM_FREE 实测闭环：alloc 0x41000 → free → alloc 又得 0x41000（行为验证，不只是读头）。
* 意义：完成通道（stream+fence）与显示零拷贝（MEM_IMPORT）两条路都已有明确命令字。
* 待办：MEM_QUERY 语义、nr 29/30/38/50/54 未知命令、atom 执行验证。

## 2026-11-22 22:56 STREAM_CREATE 可用（evidence 38 §6）

* `stream_create 0x40208018` → timeline fd=5，poll 正常（revents=0，无待完成）。
* 完成通道第一段打通；缺 atom↔timeline 的关联与错误读取。
