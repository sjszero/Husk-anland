# 变更记录

## 2026-11-23 — 页大小体检：4K / 16K / 64K 全绿（含两个我们自己造的坑）

- **evidence 122**：guest rootfs 里 **2987 个 ELF** 在 4K/16K/64K 三档全部通过，
  loader 与我们自己产出的 6 个 guest 二进制也是 `0x10000` 对齐。
- 补上第二条判据：glibc `dl-load` 要求 `p_align % PAGE_SIZE == 0`，
  不满足直接拒绝加载（`ELF load command alignment not page-aligned`）。
  之前只检查同余，所以一个"同余正确、p_align 只有 4K"的段在 4K 机器上永远看不出问题。
- `nr-tools libc-interpose`：槽位页不再塞进 `.text` 后的空洞（那里只有 0x323f 字节，
  只能声明 4K 对齐，到 16K 内核上会把整个 `libc.so.6` 变成不可加载），改为
  **追加在文件末尾、`p_align = 0x10000`**；存根页留在洞里（分支射程要求）。
  新增断言：每个 LOAD 必须 `p_align` 为 2 的幂且 `p_align % 16K == 0`、
  `vaddr ≡ offset (mod p_align)`、按 vaddr 升序。重打后的 libc 功能未退化
  （DNS、`TZ`、`locale -a`、apt、1180+ 包）。
- `nr-tools embed`：loader 里嵌入的 shim 段同样改成 `SEG_ALIGN = 0x10000`
  （`V_BASE - F_BASE` 恰等于它），loader 三段全 `0x10000`。
- `nr-tools check-elf`：纳入 `p_align % 页` 规则，遍历目录时跳过 `.orig`/`.pre-*`/`.bak*` 备份，
  并可交叉编译进 guest 直接体检。
- 新增 `tools/guest-config/build-guest-tools.sh`：`husk-usermgr` / `husk-ldconfig` / `husk-rd`
  按 64K 重编。踩到的坑写进脚本注释：只用 `rust-lld` 链接会**静默产出 `e_entry = 0`**
  （rustc 不传 `crt1.o`），程序在任何 loader 下都在 `main` 前 SIGSEGV；必须让 NDK 的
  `clang` 当链接驱动。
- 8 项外部审计（B-01…B-08）在最新 `main` 上复测：05-mount-audit **18/18**、09 **11/11**、
  10 **9/9**、11 **8/8**、12 **8/8**、13 **6/6**、IPC 3× PASS；它们早在 `38d6d05` 就已闭合。
- 仍未验证：16K/64K 内核真机（本机只有 4 KiB），那两档只是静态判定 + 4K 实跑。

## 2026-11-23 — VA-API 系统层 access-unit 采集通过，解码仍未完成

- **evidence 82**：标准 GStreamer `vaapih264dec` 管线通过 Husk 的 Rust `husk_drv_video.so`
  连续采集 **150 个真实 H.264 access unit**。首帧由 12 个 slice 拼成 94,924-byte AU，
  surface 在 5 个 VASurface 间轮转，证明标准媒体栈已进入 Husk VA-API 的
  surface/context/buffer/render 数据面。
- 驱动当前在 `vaEndPicture` 明确返回 `VA_STATUS_ERROR_UNIMPLEMENTED`；app 侧
  `AMediaCodec` transport、decoded surface / dma-buf export 尚未接上。因此本变更只证明
  “应用不改、系统层统一入口 + access-unit 采集”，不证明 H.264 实时解码、Chromium、
  HEVC、AV 同步或硬解完成。
- 原始设备输出：`docs/design/evidence/82-va-system-layer-smoke.log`；组件哈希：
  `docs/design/evidence/82-components.sha256`。

## 2026-11-23 — 1080p30 H.264 连续 10 分钟门禁通过

- **evidence 80**：6 个固定 YV12 AHB slot 经持久 PanVK converter 每帧写回 RGBA8 AHB，
  再由 app SurfaceControl 上屏。真实 `c2.mtk.avc.decoder` 完成 18,000/18,000 帧，
  `dropped=0`；app `sent/completed/presented=18,000/18,000/18,000`、`present_fail=0`，
  实际耗时 600.097 秒，guest 统计 30.03 fps。9:59 截图仍显示 `testsrc2` 彩条和时间码。
- 新增显式 `expected/completed/dropped` 计数，并把 app 汇总改成只有
  `completed == sent == presented` 且 `present_fail=0` 才 PASS。
- 资源对照：RSS 峰值 205,616 KiB，长测后稳定约 203,300–204,020 KiB；第二次 150 帧会话后
  fd=136、线程=44，没有逐会话增长。仍未回到启动前精确基线，原因标为 UNKNOWN。
- 测试仍是 MediaCodec ByteBuffer → CPU YUV 拷贝，不是 decoder→GPU 零拷贝；HEVC/VP9/AV1、
  GStreamer/VA-API/Chromium、AV 同步和硬软解 CPU 对比仍未完成。

## 2026-11-23 — 单连接持久 AHB 交接 150/150

- **evidence 76**：app 与 guest 把 evidence 75 的“每帧重连”改成整场解码会话只建立一条
  Unix socket。app 最多保留 3 个在途 `AImage` / AHB，guest 完成 PanVK 三平面校验并 ACK
  后才释放对应 `AImage`，形成有界背压。
- app 计数 `FRAME=150 / ACK=150 / RELEASE=150 / FAIL=0`，`SESSION START/STOP` 各 1；
  guest `STREAM CONNECTED/CLOSED` 各 1，`frame_summary`、`gpu_luma`、`gpu_plane1`、
  `gpu_plane2` 各 150/150 PASS，最终 `SUMMARY PASS|completed=150/150`。
- 修复了上一版 `CLOSE` 时提前停止 worker 导致尾部三帧丢失的 147/150 缺陷：
  `shutdown()` 现在先等待在途帧完成 ACK/RELEASE，最多 30 秒后再停止 worker。
- `libhuskapp.so` sha256 `4e1017b661c1045de5eae643f2e9aa2bdf7b7241bdd601ccfe154555354d93c1`；
  `vulkan-probe` sha256 `f005d26e038130cd6e9f75d177eaae141e81004d57a65e7267fb818611a9042f`；
  debug APK sha256 `56addd9dad897c9674134f0087bcc0eb74c05a71f1c08a8d36780a260eac543b`。
- **边界**：这还不是预分配 3-slot AHB import ring；guest 仍每帧创建、导入、销毁 Vulkan
  image/memory。YUV→RGB、guest WSI、VA-API/GStreamer/Chromium、HEVC/VP9/AV1、AV 同步和
  10 分钟视频门禁仍未完成；本证据使用 `HUSK_VIDEO_REALTIME=0`，不是实时播放验收。

## 2026-11-23 — 真实 MediaCodec 三平面 YUV AHB 逐帧交接 150/150

- **evidence 75**：app 与 guest 把 evidence 74 的单帧诊断推进到 **150/150 帧一帧一连接
  交接**。marker `files/video-ahb` 存在时，app 每帧重新建立 listener、发送完整 AHB native
  handle，等待 guest 完成三平面 GPU 回读后 ACK，再进入下一帧。
- app 计数：`BEGIN=150`、`HANDLE rc=0=150`、`ACK byte=0x44=150`、`DONE=150`、`FAIL=0`；
  guest 计数：`gpu_luma / gpu_plane1 / gpu_plane2` 各 150/150 PASS，
  `vk|video-ahb|SUMMARY|PASS|completed=150/150`。最后一帧三平面哈希一致：
  Y `2de7f38d259a6e33`、plane1 `5d1b68abc3eda0d0`、plane2 `c3b0fee494cf5024`。
- 解码端仍为真实 `c2.mtk.avc.decoder`，150 samples / 9,821,765 bytes / 25,944 ms，
  `EOS_OK`、`CLOSE_OK`、`RESULT PASS`；测试用 `HUSK_VIDEO_REALTIME=0`，因此
  `present|ok=false` 是探针模式预期，不是播放失败。
- `libhuskapp.so` sha256 `f13fc4d632ad7e10b644942606451f0d32cabe6bad79dbb4de41d4477d8944e7`；
  `vulkan-probe` sha256 `834dc347a4a07a37a461428195a5d4cd9d02691629d0e7bc2aae030c46058587`；
  debug APK sha256 `a18df07fcbdac20508472e0768030b79cbab64802cdc014293a8ed6b60049d75`。
- **边界**：这是每帧重新连接的一次性交接，不是持久多 slot AHB ring；YUV→RGB、guest WSI、
  VA-API/GStreamer/Chromium、HEVC/VP9/AV1、AV 同步和 10 分钟视频门禁仍未完成。

## 2026-11-23 — 真实 MediaCodec 三平面 YUV AHB 回导 PanVK，三平面 GPU 哈希通过

- **evidence 74**：`c2.mtk.avc.decoder` 输出的 1920x1080 AHB（`format=842094169`,
  `0x32315659` / `"YV12"` private FourCC）经 `AHardwareBuffer_sendHandleToUnixSocket`
  进入 guest。app 用 `AHardwareBuffer_lockPlanes` 取得真实三平面布局，而不是猜测 fd[0]
  或依赖 `AImage_getNumberOfPlanes`（PRIVATE image 返回 0）。
- guest PanVK 以 `VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM` + `DRM_FORMAT_MOD_LINEAR` 导入
  同一个 dma-buf，分别对 plane 0/1/2 执行 `vkCmdCopyImageToBuffer` 和 host readback。
  三平面 FNV-1a 全部与 app 侧一致：
  `d4abebfa3f8cb0fc / a8a073543b9ed2db / f164ae94471897d1`。
- 真实解码会话仍为 150 个 AVC access unit、150 个输出 image，EOS/stop/delete 全部通过，
  guest `RESULT PASS`，约 5.1 秒。
- Vulkan probe 的 socket 连接增加 90 秒重试；真机日志先出现
  `socket_wait|retry=1|... No such file or directory`，随后成功连接，消除了“必须先启动
  decoder 再启动 probe”的时序竞态。
- `libhuskapp.so` sha256 `3d8b0caf8bf899206d15c2d89c2ef30faeca30a4969872e00beed9c33f834603`；
  `vulkan-probe` sha256 `8658fcbd56fb56e413eb9771b31ae1a09b8a0529c971565dff02b95749ac895b`；
  debug APK sha256 `2e551f5aa285e6a11f594901434de51af126b5d13ab649c51bdb26fa4965f923`。
- **边界**：这是首帧一次性诊断，不是逐帧 ring；YUV→RGB、guest WSI、VA-API/GStreamer/
  Chromium、HEVC/VP9/AV1 bridge、AV 同步和 10 分钟视频门禁仍未完成。

## 2026-11-23 — AHB 回导 guest：sticky map 与 PanVK GPU 写回

- **evidence 73**：app 分配的 `64x64 RGBA8` AHardwareBuffer 经 `SCM_RIGHTS` 进入 glibc
  guest；正确的 `MEM_IMPORT` 布局返回 `cookie=0x41000 / va_pages=4`，
  `STICKY_RESOURCE_MAP`（`0x4010801d`，`{count, address}`）返回 0。
- 修正了 AHB meta 的 stride 解释：`stride=64` 是像素，必须按 `stride * 4` 计算 pitch；
  kbase UMM mapping 只作为 GPU 翻译窗口，CPU 权威视图直接 `mmap(dmabuf)`。guest 写入后
  直接 dma-buf 视图、kbase 视图和 app AHB 的 9 点回读均为红，app 经 SurfaceControl 实际上屏。
- 同一 fd 又交给 guest PanVK：LINEAR 2D image 创建、dma-buf import、bind 均成功；
  GPU `vkCmdClearColorImage` 后 staging 与直接 dma-buf 视图均 9/9 红，app 再次回读并上屏。
- `gpu-probe` sha256 `2a9bd65836cbc2a384baef81615b30d7f3162789562211a3c67893cb3c5247cd`；
  `vulkan-probe` sha256 `10a1fd47dc8eab25a3275ea47805146c50156337523938887c2b53e23f012ab7`；
  `libhuskapp.so` sha256 `bc79e576569eecdae7dfd9d89b418a8c1c440d27782164923f95c6c6d5c9ab2f`；
  debug APK sha256 `31905cc784c69dd8590983920e2381282c209543f9536bff341a0baebc07b49d`。
- **边界**：当前证据的 AHB 是 RGBA8 LINEAR 测试面，不是 MediaCodec 的
  `AIMAGE_FORMAT_PRIVATE / YUV420_888` 解码帧；guest WSI、VA-API/GStreamer/Chromium、
  AV 同步和 10 分钟视频门禁仍未完成。

## 2026-11-23 — 视频跨旋转与 SurfaceControl 重建

- **evidence 72**：播放中的 1080p30 AVC 从 1080x2400 旋转到 2280x1080 后继续上屏，
  guest 仍 `150/150`、`RESULT PASS`；presenter 目标区域从
  `(0,896)-(1080,1504)` 更新为 `(180,0)-(2100,1080)`。
- SurfaceView 的父 SurfaceControl 不再永久缓存。JNI 最多每秒刷新一次；Home 后回前台检测到
  新 parent，presenter 创建新子层，旧层按回调代际延迟释放，日志 `video|present|rebuild|PASS`。
- 后台没有活动窗口期间，解码帧明确跳过而不是崩溃；该会话最终 `submitted=119`，恢复后继续
  出画并正常 stop/delete。
- `libhuskapp.so` sha256 `bfc6c56ef06655386494a8c46ccb89708e25994946e062a657e666b4710e8217`；
  debug APK sha256 `e1e6a98ba863fd63baa0aa400a78f4b0c189de13328faad0c8a59f293681bfe1`。
- **边界**：guest 侧 WSI、`MEM_IMPORT`、AV 同步、10 分钟播放和多窗口/分屏仍未完成。

## 2026-11-23 — MediaCodec AHB 实时硬解上屏

- **evidence 71**：guest 通过正常 `rootfs-full` 内的 `/run/husk/video.sock` 实时发送
  1920x1080@30 AVC 的 150 个 access unit；app 用 `c2.mtk.avc.decoder` 产出 150 个
  AHardwareBuffer，并逐个直接提交到持久 `husk-video` SurfaceControl 子层。
- 两次独立会话均 `RESULT PASS`、约 5.1 秒；presenter 两次均为 `submitted=150`、
  `completed=150`、`released=150`，release fence 全部成功。两张截图显示不同时间码与运动位置，
  证明是连续动态播放而非单帧停留。
- 追加两轮 5 秒会话时，app fd 在 133-135 之间波动，RSS 约 155 MiB；只记录为短时有界，
  不冒充 10 分钟无泄漏验收。
- **边界**：这是 app 容器内的硬解输出直接上屏，不是把解码 AHB 经 kbase `MEM_IMPORT` 回导
  guest。guest GL/X11/Wayland WSI、AV 同步、GStreamer/VA-API、Chromium、HEVC/VP9/AV1 bridge、
  10 分钟门禁仍未完成。
- `libhuskapp.so` sha256 `05e39bfff905bc0e8225cf5934cb434191fb44a8f67c393b749bea46d38a300f`；
  `video-bridge` sha256 `e8dadebd659daa399c416ea5b1677dfdb993f5dc8cc32894739043f7bcfbc18e`；
  debug APK sha256 `285e4f7135f329f31feee0655bf02a6277fba00429138cce5cc1d6ce25667968`。

## 2026-11-23 — Guest 可见 Unix socket 路径隔离打通

- **evidence 70**：path-shim 新增 pathname `AF_UNIX` 的 `connect(2)` 翻译，guest 在正常
  `rootfs-full` 根内直接连接 `/run/husk/video.sock`；app 在
  `files/rootfs-full/run/husk/video.sock` 接受连接。abstract / unnamed / 非 `AF_UNIX`
  地址保持原样透传，不关闭 rootfs 隔离。
- 新 loader 上机回归：功能矩阵 31/31、越界测试 0 escapes、路径可靠性 15/15、
  `faccessat` 8/8、dirfd 31/31。
- `libld.so` sha256 `0f3eca76af7cac8258258ea1e6d8932f070034dbd8e913cc19d29797bd13c229`；
  `libhuskapp.so` sha256 `b09997c6e3d0cb67c7dbe3ae66ee43929a697b0267f2c8685edc35bf508eee7f`；
  debug APK sha256 `8501304e58a9ba43834f056ae9a27863d7f9da2715b4a6c0f13d344b2fd9f7d8`。
- **仍未完成**：完整 AVC 媒体面在 guest-visible socket 上的复跑、AHB `MEM_IMPORT` 回导与
  解码后上屏、HEVC/VP9/AV1、VA-API/GStreamer/Chromium、AV 同步和 10 分钟播放。

## 2026-11-23 — Guest 压缩视频桥 v1 实机打通

- **evidence 69**：正常 `rootfs-full` 根内的 glibc guest 用纯 Rust MP4 reader 解复用
  1920x1080@30 AVC，150 个 access unit 经 4×2 MiB memfd ring + AF_UNIX 控制通道送到 app；
  app 用真实 `c2.mtk.avc.decoder` 解出 150 个 AHardwareBuffer，`EOS`、stop/delete、断连均成功。
  本次 guest 150/150 样本、`RESULT PASS`、1599 ms；app 日志累计 14 个相同会话全部
  `frames=151 images=150`。
- 控制 socket 已移到 guest 可见的 `rootfs-full/tmp/husk-video/bridge.sock`，不再需要关闭
  rootfs 隔离；但 shim 尚未翻译 `connect(2)` 路径，当前 harness 传私有物理 socket 路径。
- **仍未完成**：AHB `MEM_IMPORT` 回导与解码后上屏、HEVC/VP9/AV1 bridge、GStreamer/VA-API、
  Chromium、AV 同步、丢帧统计、10 分钟播放和硬软解 CPU 对比。
- APK sha256 `4bc472f33a88d7c0599b80ff91cfb2100bf8610de61f6db76a99baa2c2c3c386`；
  `libhuskapp.so` sha256 `62226163c5e259d9a774296d7e2736e82a96069c8c4f0298defe938d9d5643f6`；
  `video-bridge` sha256 `a7c869f0e695b0a9757f639dbaba191e4f48aefb969e63d9501da32e8e5bd08f`。

## 2026-11-23 — PanVK 真实 SYNC_FD 接入 SurfaceControl ring

- **evidence 58**：FristOneRR kbase 后端补齐 JM out-fence stream，并让
  `vkGetFenceFdKHR(VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT)` 返回真实
  `mali-mali.timeline*` sync_file；不再 CPU wait 后返回 `fd=-1`。
- 3-buffer SurfaceControl ring 连续 120 帧全部 `kind=SYNC_FD`：11 帧导出时 pending，
  109 帧已 signal；120 帧 completion 后同一 fd 均为 `status=1`，120 次 9 点 GPU 回读 PASS，
  eventfd fallback = 0，guest/app `FAIL` = 0，app `submitted=120 completed=120`，
  最终回收 3 个 buffer。
- 7 文件 Mesa/kbase patch 已固定为
  `docs/design/patches/fristonerr-kbase-syncfd-export.patch`。
- **边界**：没有 SurfaceFlinger 内部等待 trace；持久 Vulkan 资源池、`VK_KHR_swapchain` /
  Wayland / X11 WSI、窗口生命周期和 10 分钟长稳仍未完成。
- probe sha256 `28095769e2313e6fa65a9bad7748dce94fe1d05381cb2bec468bbe24a2ca6ac0`；
  driver sha256 `e6d1b6fed4b09ff1e84474b160b8898b8adcc4effd01d08c87d129091c838f5e`。

## 2026-11-23 — r32p1 kbase 生成真实 pending sync_file

- **evidence 57**：guest 直连 `/dev/mali0`，用 64-byte `base_jd_atom_v2` 提交
  `SOFT_EVENT_WAIT -> SOFT_FENCE_TRIGGER(ORDER dep)`。内核回写真实
  `mali-mali.timeline*` sync_file；事件设置前 `SYNC_IOC_FILE_INFO.status=0` 且 poll 250 ms
  超时，设置后同一 fd readable、status=1，两个 atom 均为 DONE。
- 纠正两个 ABI 假阴性：不能混用 64-byte v2 与 72-byte atom 的字段偏移；`SAME_VA` 分配在
  `mmap` 前返回的是 cookie，mmap 后的映射地址才是 GPU VA。
- 边界：目前只证明内核层 producer fence；尚未接进 FristOneRR PanVK
  `vkGetFenceFdKHR` / `SYNC_FD`，也尚未证明 SurfaceFlinger 在 GPU 完成前等待该 fd。
- guest probe sha256 `c043e256ceecb34d90ed1b21041aca230905286c4fdf77f2df90f5e32bf954c3`；
  guest log sha256 `987f02a9c82dc06ccebefe6aac5b55804c2fa35a742d23537712fd576fa780b3`。

## 2026-11-23 — eventfd ring 5,600 帧短时 soak

- **evidence 56**：同一 3-buffer AHB / eventfd completion bridge 连续运行 5,600 帧，耗时
  134 秒；app `submitted=5600 completed=5600 guest_done=true`，guest `rendered=5600/5600
  releases=5597 done=true`，两边 `FAIL` 计数为 0，结束时 3 个 buffer 全部释放。
- guest 日志有 5,600 条 `image_complete`；最后一帧截图为绿色。
- **边界**：这不是 10 分钟门禁；每帧仍创建、导入、绑定并销毁 Vulkan image/memory/fence，
  持久资源池和真正 pending producer fence 都未完成。probe 同时修正了成功 `sendmsg` 后读取
  陈旧 errno 的日志问题。
- guest probe sha256 `fc395bbe037617ed658a7a751bfbf38a1ea15102baba203e345f572bab7912e6`；
  APK sha256 `78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d`。

## 2026-11-23 — eventfd acquire bridge 连续 120 帧（非 SYNC_FD）

- **evidence 55**：guest `vkQueueSubmit` 后由 Rust completion 线程等待 `VkFence`，GPU 完成后写
  `eventfd`；`READY <slot> <frame> <expected>` 与 eventfd 通过同一次 `sendmsg(SCM_RIGHTS)` 交给
  app，app 将 fd 交给 `ASurfaceTransaction_setBuffer`。
- 真机 3-buffer ring 连续 120 帧通过：app `submitted=120 completed=120 guest_done=true`，
  guest `rendered=120/120 releases=117 done=true`，最终 `buffers_released=3`；SurfaceFlinger
  完成 transaction，release fence 背压和 fd 回收通过。app fd 数 ring 前 141、cleanup 后 129。
- **边界**：eventfd 是 pollable completion bridge，不是 Vulkan `SYNC_FD`，也没有
  `sync_file` 的 signal-time / `SYNC_IOC_FILE_INFO` 语义；本次多数帧在 app 收到 READY 前已经完成，
  不能写成“等待了 pending producer fence”。
- **仍未完成**：kbase `STREAM_CREATE` + soft-fence trigger 或 PanVK 真实 dma-fence；持久资源池、
  GBM / Wayland / X11、WSI / swapchain、旋转 / DPI / 多窗口、SurfaceView 重建和 10 分钟泄漏。
- guest probe sha256 `42a1cbd79086a4bb3199cc21383ccf8c8212a43fc9b74487d2adc25c20b304e5`；
  APK sha256 `78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d`。

## 2026-11-23 — 3-buffer AHB ring 连续 12 帧与 release-fence 背压

- **evidence 54**：app 一次分配 3 个 AHardwareBuffer，经 `SCM_RIGHTS` 交给 guest；guest 连续 GPU
  渲染 12 帧并按 slot 等待 app 的 RELEASE。实测 app `submitted=12 completed=12`，guest
  `rendered=12/12 releases=9`，3 个 slot 各复用 4 次。
- app 用 `ASurfaceTransaction_setOnComplete` 取得上一 buffer 的 SurfaceFlinger release fence，
  实测 fd `143/153/150` 均 signal；signal 后才回 guest RELEASE，避免覆盖仍在合成的 buffer。
- 最终第 12 帧为蓝色，截图正文按 8 像素采样 `37,800/37,800` 点纯蓝。APK sha256
  `ef6d88c866494aa3f12e151c1a515a1a8e3109f08aed2805806473892c36573f`；guest probe sha256
  `d939442c51c77f77845d9cc303d25e5f80063040c68c363130a3b2e499af22ae`。
- **仍未完成**：producer/acquire fence 仍为 `-1`（guest 先 `vkWaitForFences`），Vulkan 资源每帧
  重建，尚无性能基准；10 分钟泄漏、旋转 / DPI、SurfaceView 重建、GBM / Wayland / X11 与 WSI
  未验证。

## 2026-11-23 — 同一 guest AHB 经 SurfaceControl 零拷贝上屏

- **evidence 53**：纯 Rust JNI 从 `ActivityThread.mActivities` 找到当前 Activity，创建平台
  `SurfaceView`，读取 BLAST `mBlastSurfaceControl.mNativeObject`，再用
  `ASurfaceControl_create` + `ASurfaceTransaction_setBuffer` 将 guest 已写红的同一 AHB 直接提交。
- 截图正文 24,192 / 24,192 个采样点为纯红；SurfaceFlinger 层树确认 `husk-ahb` BufferStateLayer
  有 `64x64 RGBA_8888` activeBuffer，显示区域 `[0,120,1080,2359]`，合成为 `DEVICE`。
- 修正窗口层级：`setZOrderMediaOverlay(true)` 会被 NativeActivity 主窗口盖住；改为
  `setZOrderOnTop(true)` 后系统栏之间的正文区域正确显示 guest buffer。
- 仍为零 Java/dex/C；JNI 只调用 Android 平台已有类。`ASurfaceControl_createFromWindow` 对
  NativeActivity BLAST window 仍返回 null，SurfaceView handle 是当前可用路径，EGLImage 保留为降级。
- APK sha256 `e7e5da9ab3dcf7f0fee84afef48fc4c04b94074cecec181af7a950e1ca9e827c`；
  `libhuskapp.so` sha256 `8644002c82f50317553e72f514b35c8b7327d56fe8fe76f2441566c99d1f42be`。
- **截至 evidence 53 时未完成**：acquire/release fence、连续帧与背压、buffer 复用、GBM / Wayland /
  X11、WSI / swapchain、旋转 / DPI / 多窗口、SurfaceView 重建和 10 分钟泄漏。

## 2026-11-23 — AHardwareBuffer dma-buf 已跨进程导入 PanVK 并由 GPU 访问

- **evidence 50**：app 分配的真实 AHardwareBuffer 经 native handle 扫描出 dmabuf fd，
  通过 `SCM_RIGHTS` 跨进程交给 glibc guest；guest 成功用 PanVK
  `VK_EXT_external_memory_dma_buf` + `VK_KHR_external_memory_fd` 创建 external buffer、
  导入 memory 并 bind。
- PanVK 报告 DMA_BUF buffer 能力 `compatibleHandleTypes=0x201`；导入后 GPU compute
  写入并回读 `0x4855534b` 为 **PASS**，证明 GPU 确实访问了 dma-buf 后备存储。
- **仍未完成**：AHB 2D image / DRM modifier 导入、ANativeWindow queueBuffer，
  SurfaceFlinger、present fence、旋转 / DPI / 多窗口 / 10 分钟泄漏。
- APK sha256 `4996f71a59b46ba729b55e76bff5b854d12372f45109bef5f6b12a4d8d3dbce2`；
  `vulkan-probe` sha256 `a9f4cc15f324743a44e094ac79296d1fbb4c5e3292d3929c7f3475fd2cdb61d5`。

## 2026-11-23 — AHardwareBuffer 导出 dma-buf fd 已实测

- **evidence 49**：纯 Rust NativeActivity 动态解析 `libandroid.so` 的 AHardwareBuffer API，
  分配 `64x64x1 RGBA8` GPU buffer；`AHardwareBuffer_getNativeHandle()` 返回 3 个 fd，
  其中一个是 `/dmabuf:G:pid [...]`。
- `AHardwareBuffer_sendHandleToUnixSocket()` / `_recvHandleFromUnixSocket()` 在
  `AF_UNIX/SOCK_SEQPACKET` 上均返回 0，接收端得到 3 个新的有效 fd，dmabuf 身份保持。
- 同一轮启动保持正常 guest 会话、Surface 1080x2400 与 resize；APK sha256
  `dfe3dccee9851b4561ff60b043d391f09bf6f427ddb79d87d9cf0f2d23690899`。
- **仍未完成**：跨进程把 fd 交给 glibc guest、PanVK dma-buf 导入、AHB queueBuffer、
  render-done fence 和实际窗口显示。native-handle fd 顺序不是公开稳定 ABI，代码必须扫描
  `/proc/self/fd` 找 dmabuf。

## 2026-11-23 — PanVK 表面 / dma-buf 扩展扫描

- **evidence 48**：纯 Rust `vulkan-probe` 新增 instance/device extension 枚举；真机报实例扩展 18 项、
  设备扩展 171 项。
- **FACT**：支持 `VK_KHR_surface`、`VK_KHR_swapchain`、`VK_KHR_external_memory_fd`、
  `VK_EXT_external_memory_dma_buf`、`VK_KHR_external_fence_fd`、`VK_KHR_external_semaphore_fd`、
  `VK_EXT_image_drm_format_modifier`、`VK_KHR_timeline_semaphore` 等。
- **FACT**：缺失 `VK_KHR_android_surface` 与 `VK_ANDROID_external_memory_android_hardware_buffer`。
  因此 Android 窗口路线改为 app 侧 AHB → dma-buf fd → guest 导入，不能用 guest 直接建
  Android Vulkan Surface 的假设。AHB native handle 是否含可导入 fd 仍为 UNKNOWN。
- 离屏 compute / graphics / GPU readback 回归仍全部 PASS；产物 sha256
  `e7b9b6936cef9cf354e81200450b82ea6a5b888930f74814e06ad761f92b2175`。

## 2026-11-23 — Zink/PanVK surfaceless renderer 通过；壳应用注入 GPU 环境

- **evidence 47**：FristOneRR glibc PanVK + 发行版 Zink 在 guest 内通过 surfaceless EGL 1.5；
  `eglinfo -B` 与纯 Rust `gl_probe` 都报出
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；`vulkaninfo --summary` 报
  `driverID=DRIVER_ID_MESA_PANVK`、deviceID `0x90800011`。
- 壳应用改为优先使用私有 `files/rootfs-full`，并把 PanVK ICD、Zink 与 EGL vendor 环境注入 guest；
  真机 `/proc/<bash>/environ` 与 logcat 已确认生效。新增两个 `gpu_env` 单元测试。
- **仍未完成**：GBM / Wayland / X11 platform 均 `eglInitialize failed`；swapchain、WSI、
  Android Surface、SurfaceFlinger queueBuffer 与实际窗口显示未验证；硬件 vs llvmpipe 基准未做。
- 目标升到 **v17**。参考仓库 120/120 逐条审计也已在同日落盘并推送。

## 2026-11-23 — 参考仓库 120/120 逐条审计落盘

- 参考清单审计（笔记在仓库外）：12 类、120 条全部核对，
  每条给出真实来源、核验状态、A/B/C 优先级与 Husk 用法；不再用 63 组合并表代替逐条结论。
- 审计结果：
  69 条 GitHub、45 条官方上游/组件、6 条 UNKNOWN；A/B/C = 28/44/48。
- 纠正了几处清单与真实来源对不上的条目（详见仓库外笔记）。
- 有 6 项清单名称过于泛化、找不到可信唯一来源，一律标 UNKNOWN，不猜链接。
- README、STATUS、设计索引、PROJECT-MEMORY 已同步。

## 2026-11-23 — PanVK 离屏 graphics draw 在 Mali-G77 上通过

- **evidence 46** 扩展：纯 Rust `vulkan-probe` 新增 `graphics.rs` 与 vertex/fragment SPIR-V，
  创建颜色附件、render pass、framebuffer、真实顶点缓冲和 graphics pipeline，执行
  `vkCmdDraw(3,1,0,0)`，再 `vkCmdCopyImageToBuffer`，fence 后回读红色像素 `0xff0000ff`。
- 真机连续两次 PASS；compute job 与离屏 graphics 共用同一条 guest 内 PanVK / kbase 路径。
- 首次 combo（仅 `gl_VertexIndex`、无顶点缓冲）未产生可见 fragment；改为真实顶点缓冲后通过。
  没有把第一版失败武断归因成 driver bug，仍标 UNKNOWN。
- **仍未完成**：swapchain / WSI / SurfaceFlinger、Zink、`eglinfo/glxinfo` renderer；
  目标升到 **v16**。

## 2026-11-23 — PanVK 最小验证：Mali-G77 compute job 已由 GPU 执行

- **evidence 46**：从 `FristOneRR-Admin/FristOneRR-Panvk-Source`
  commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`（Mesa 26.3.0-devel）编出 glibc 版
  `libvulkan_panfrost.so`，产物 sha256
  `fbfa3b43e69d3fd7f4088fe6b9e94a621e19ca3f95502265281f92ba908b6147`。
- 新增纯 Rust `runtime/crates/vulkan-probe`：动态加载 Vulkan loader，完成 instance、physical
  device、queue family、logical device、queue、4 KiB host-visible storage buffer，
  并提交 SPIR-V compute pipeline + `vkCmdDispatch` + `vkQueueSubmit`，fence 后回读 GPU 写入。
- 真机实测：`vulkaninfo --summary` 报 `Mali-G77 MC9`（deviceID `0x90800011`，PanVK）；
  纯 Rust probe 的 compute submit + fence + readback `0x4855534b` 全 PASS。
- 修复 `KBASE_IOCTL_MEM_EXEC_INIT` 顺序：EXEC_VA 必须在 JIT 前初始化，并使用 4G VA；
  修复后 warning 消失。可复现 patch 已入库。
- **未完成**：graphics draw、swapchain、surface 和 `eglinfo/glxinfo` renderer 未验证；
  目标升到 **v15**；`funnymdzz/mesa` 保留为 `drmSyncobjCreate()` 失败对照。

## 2026-11-23 — 长期记忆归档：目标 v13、120 条参考清单、代理规则

- 新增 `docs/PROJECT-MEMORY.md`：把产品边界、硬约束、GPU/显示/视频/音频路线、
  2.0 替换方案、操作纪律和下一次接手顺序固化成文件，不依赖聊天上下文。
- 用户提供的 12 类、120 个参考条目已核对归档（笔记在仓库外）。
- 目标升到 **v13**：evidence 45 证伪了“Ubuntu panfrost + DRM 语义层是唯一实现”的假设；
  当前主动 GPU 路线改为 **panvk kbase fork（glibc）+ Zink**，DRM 语义层保留为后备。
- 记录代理规则：GitHub 不通时临时使用 `127.0.0.1:7897`，用完立即 `unset`。
- README / STATUS / 设计索引同步最新状态，避免旧结论继续误导后续工作。

## 2026-11-23 00:40 — kbase 完成通道：证伪一条、钉死三条

- **evidence 40**（新增）：`gpu-probe` 加 12 个模式（`ddkver`/`badptr`/`stride_probe`/`fence0-6`/`cr`/`sevt`/`sevt2`/`sevtjit`）。
- **FACT**：内核 kbase 自报 `K:r32p1-00bet5(GPL)`，与厂商 `libGLES_mali.so` 同代。
- **FACT**：atom 72 字节、`stride` 必须 72（56 → EINVAL）；坏指针 → EFAULT，证明数组被 `copy_from_user`。
- **FACT（推翻旧结论）**：`STREAM_CREATE` 的 timeline fd **不是完成信号** ——
  任何被接受的 submit（含 `core_req=0`、含本该阻塞的 `SOFT_EVENT_WAIT`）都让它 POLLIN，
  `read()` 返回 EINVAL。evidence 38 §6 的"可 poll = 完成通道"作废。
  本轮开头我自己报的 `fence1 PASS` 是假阳性，已被 `fence0` 对照打掉。
- **FACT**：软作业在本机不产生效果（`SOFT_EVENT_SET` 写显存 0 次，6 种偏移 + compat 双写 + 先 JIT init 全试过）；
  内核也不把 sync_file fd 写回 `base_fence`。
- 下一步：拿 r32p1 的 `base_jd_atom_v2` 定义（MTK GPL 内核源），或改用"GPU 写显存"作为唯一完成口径。

## 2026-11-22 23:20 — 目标 v10：视频补全

- **evidence 39**：把「guest 能不能直连硬解」钉死。`/dev/video0-4` 是 `0660 media:system` 的
  相机采集节点（不是编解码器），`untrusted_app` 打不开；厂商解码 `.so` 带 bionic DT_NEEDED，
  glibc guest 内加载不了 → **硬解只能在 bionic 侧调 NDK `AMediaCodec`**。
- 本机硬解器（取自 `/vendor/etc/media_codecs_c2.xml`）：`c2.mtk.{avc,hevc,vp9,av1}.decoder`
  及 mpeg4/h263/mpeg2/vc1/heif；厂商自报 1080p 帧率 avc 113–248 / hevc 125–276 / vp9 114–251 fps（未实测）。
- 目标升到 **v10**：总目标从「GPU / 显示 / 音频」改为「GPU / 显示 / **视频** / 音频」四件同级；
  第 4 条视频从一句口号扩成可验收规格（独立 socketpair + 共享内存环 → app 内 Rust 解码线程 →
  MediaCodec → AHardwareBuffer → MEM_IMPORT 回 guest；验收含 1080p30 H.264/HEVC 十分钟不掉帧、
  硬解 vs 软解 CPU 对比、AV 同步 <40ms、Chromium `<video>`）。
- 新增 `docs/goal-objective.txt`（粘贴块纯文本版），`docs/GOALS.md` 附三锚点表补 4 行视频事实。

## 2026-11-22 深夜 — 生命周期收口 + SoC 直连打通第一段

### 运行时壳（app-rs）
- NativeActivity 回调补齐：`onStop` / `onSaveInstanceState` / `onLowMemory` / `onContentRectChanged`。
- 修 4 个真 bug：BACK 被吞、`AKEYCODE_BACK` 变成 match 通配、onDestroy 不 teardown、重建后不再起会话。
- 修 2 个实测泄漏：`Term` 缺 `Drop`（每次重生漏 1 个 ptmx + 1 个 pty.log）、`Gfx` 缺 `eglTerminate`
  （每次旋转漏 sync_file/ashmem）。`session:` 日志加 `fds=` 便于长跑观测。
- 旋转实测：`67x58 -> 142x26 -> 67x58`，同一 pid，guest `stty size` 一致。
- 规格文档：`docs/design/lifecycle.md`（15 条不变量，逐条给代码位置与验证方法）。

### SoC 直连（GPU）
- **evidence 35**：从厂商 `libGLES_mali.so`（r32p1）反汇编取出 26 条真实 kbase 命令字。
  `JOB_SUBMIT = 0x40108002`（W,16），guest 内实测 **EINVAL 而非 ENOTTY** —— 旧结论"提交接口待查"是测量错误。
- **evidence 36**：上游 Mesa（25.2 与 main）**没有** kbase 后端（只有 panfrost_kmod / panthor_kmod），
  rootfs 也无 panvk ICD → 路线改为 Rust 侧 DRM 语义层。
- `mem_alloc(flags=0xf)` + `mmap(prot=3, off=VA)` 实测成功，CPU 写读一致；
  规则：`prot` 必须等于 flags 低两位，否则 EPERM。

### 目标与文档
- 目标重写为 v7/v8：已完成项（SIGSYS 长稳、生命周期、页大小）降为回归门禁；
  新增【D】段——用 Husk 替换 2.0 的 proot、一次性消除 proot 补丁、回滚包
  `LinuxHub-Myth-Prometheus.apk` sha256 `c4951876…9adc2b7`。
- README / RESULTS / FUTURE-WORK / ENGINEERING 按实测刷新，撤销"厂商内核节点一律不可直连"的错误结论。

## 2026-11-22 夜 — 壳应用改为纯 Rust

- **去掉 Java 与 C**：APK 改为声明 `android.app.NativeActivity`，入口是 `app-rs/` 里的
  `ANativeActivity_onCreate`。Java Activity、JNI 外壳、`pty.c` 全部删除。
- **壳应用现在包含**：PTY 与 `fork`/`execve`（直接跑 guest loader）、EGL + GLES2 渲染、
  运行时字体光栅化（`fontdue`，字模取自系统等宽字体）、ANSI 子集解析、光标块、
  生命周期（surface 销毁/重建、窗口尺寸→`TIOCSWINSZ`、失焦不画光标、会话随进程存活、
  子进程退出后下一次点击自动重启）。
- **单元测试**：`app-rs` 的终端解析与屏幕缓冲有 9 个主机侧测试（`cargo test`）。
- **构建工具**：Python 参考实现删除（产物与回归测试 sha256 一致，可从 v0.1.0 取回）；
  loader 重构后重新构建，sha256 仍为 `1600a850…1cbee`。
- **基准**：新增 `runtime/crates/guest-bench`（Rust 版客舱基准）；`bench.c` 保留为无预编译
  二进制时的兜底。
- 语言构成：Rust 5,681 行 / 汇编 271 行；Java 0、Python 0。


## 2026-11-22 — v0.1 原型（一天内完成）

**结论：在无 root、无虚拟化、无 ptrace 的 `untrusted_app` 域里跑通了 bash + coreutils +
完整 gcc 13.3 / make 工具链。**

### 运行时
- 路径 shim 从 `LD_PRELOAD` 下沉进 loader：成为 `ld.so` 的一个额外 `R|E` `PT_LOAD`，
  由 `_dl_init` 钩子完成符号介入（76 个名字的 GOT 槽）。运行时**零 syscall 拦截**。
- `set_robust_list` / `rseq` 两处 `svc` 构建期 `nop`：app seccomp 政策会把它们判成
  `SECCOMP_RET_TRAP`，且 glibc 在 loader 内部就调用它们。
- 起始 cwd 改为 `/`；`..` 在 rootfs 边界截断；符号链接逃逸测试 0 escapes。
- `execve` 认出动态解释器后改走同一个 loader，子进程天然带 shim。

### 构建工具（全部 Rust，零依赖）
- `nr-tools scan-syscalls`：按 `mov x8,#N; svc #0` 模式定位调用点，读 `.gnu.version_d`
  自检 glibc 版本，四种异常一律拒绝写文件。
- `nr-tools embed`：把 shim 与服务表嵌进 `ld.so`，所有重定位在构建期烘好。
- `nr-tools provision`：替代 sh+python 的工具链供应脚本。
- 三个工具都有 sha256 回归基线，产物稳定。

### 验证
- 31/31 功能矩阵；11 项越界测试 0 escapes；gcc+make 全链 exit 0。
- 成本：`getpid` 196 ns、`stat` 单组件 5.4 us（+2.5 us）、`fork+execve+wait` 2.49 ms
  （同机原生 bionic 6.8–8.0 ms）。

### 已知缺口
- `execl/execle/execlp` 拦不到；`freopen/glob/nftw/dlmopen` 未覆盖；
  `posix_spawn` 用 `fork` 而非 `vfork`；16K 页内核未测。
