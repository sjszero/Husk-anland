# 目标（Husk）

> 本文件是目标的**权威规格**。会话里的 goal 对象不可编辑，重建 goal 时以本文件为准；
> 改目标就改这里，再把下面的「粘贴块」整段贴进去。纯文本版：`docs/goal-objective.txt`。

最后更新：2026-11-23（v16：evidence 46 —— FristOneRR PanVK 在 guest 内枚举 Mali-G77，compute job 与离屏 graphics 三角形均完成；GPU 回读 `0x4855534b`，像素回读 `0xff0000ff`。swapchain / WSI / `eglinfo` / `glxinfo` renderer 仍未验收）

---

## 粘贴块（可直接复制进 goal 字段）

总目标：Husk（纯 Rust 原生 Linux 运行时，仓库 /Users/l/Documents/Codex/2026-08-10/husk）在 RMX3357（MT6893 / Dimensity 1200 / Mali-G77，Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64）上端到端跑通，做到能替代 LinuxHub 2.0 的产品化程度；原生直连 SoC 的四件事——GPU / 显示 / 视频 / 音频——与运行时核心同级，必须做完，缺一条不算完成。

硬约束：不 root、不解 BL、不开虚拟化；运行时零 syscall 拦截（shim 只在必要处 interpose，不再扩大拦截面）；能 Rust 就 Rust（零 Java、零 Python、零 C，只有内核 ABI 强制的裸汇编 stub 例外）；绝不读写 com.linuxhub.myth 的任何数据，只用自己私有目录里的副本；单台旧设备的问题不外推到所有设备；结论只按实测下，推断必须标注。每次正式改动都提交本地并推 GitHub。

【已完成 · 不再是任务，只保留回归门禁】
· SIGSYS 全链路：interpose + ENOSYS 仿真可用，10 分钟长稳通过（600s / 11360 次 fork-exec / 零僵尸，evidence 32）。
· 壳应用生命周期：全部回调接上，旋转 / Surface 重建 / 切后台不重启会话，唯一 reaper + subreaper，零僵尸零重启风暴（docs/design/lifecycle.md + 实测）。
· 页大小无关：构建期 over-align + `nr-tools check-elf`（4K/16K/64K）+ embed 同余断言（evidence 30）。
· 终端渲染 ANSI（SGR 颜色 / UTF-8 宽字符）：**不做**。终端壳只是演示载体，不属于运行时范围，不作为验收项。

【A. 真实场景验证】
1）2.0 完整 rootfs 已能起 GNOME Shell 46（evidence 33），只剩两件：① libc 内部调用槽位方案（docs/design/libc-interpose.md，未实现）——否则 apt/dpkg/locale 仍读宿主文件；② 浏览器 / 文件管理器 / 终端三个可用性验收。

【B. 原生直连 SoC】
环境事实（guest 侧实测）：只有 `/dev/mali0`（rw 可开，kbase UAPI 11.38）；`/dev/kgsl-3d0` 不存在；`/dev/dri` 打不开；`/dev/dma_heap` 不存在；`/dev/snd` EACCES；`/dev/ion` guest 侧 EACCES（待复核）；`/dev/video0-4` 为 `0660 media:system`（untrusted_app 打不开，且是相机采集节点、不是编解码器，guest 无法直连硬解）。厂商 libGLES_mali.so 带 25 个 bionic DT_NEEDED，不能在 glibc guest 内加载，此路线已排除。
2）GPU：唯一验收目标 = guest 进程自己打开 /dev/mali0，直连厂商 kbase，不经中转服务、不留独立 GPU 进程。上游 Mesa（25.2 与 main）没有 kbase 后端（FACT，evidence 36）。证据 45：Ubuntu 25.2.8 的 Mesa panfrost 已能在 guest 内加载并通过 Rust DRM 语义层读到 Mali-G77 的真实参数，但模型表无 G77/G78，且在 CREATE_BO 之前失败（egl: failed to create dri2 screen）；伪装成 G57 仍失败。当前主动实现路线（JUDGMENT）= `FristOneRR-Admin/FristOneRR-Panvk-Source`（commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`，Mesa 26.3.0-devel，glibc 交叉构建）+ 发行版 Zink；evidence 46 已实测 physical/logical device、queue、host-visible storage buffer，并通过 `vkCmdDispatch` + `vkQueueSubmit` + fence 读回 GPU 写入 `0x4855534b`；同一 probe 还用真实顶点缓冲、vertex/fragment SPIR-V、render pass 与 `vkCmdDraw` 完成离屏 graphics 管线，连续两次回读红色像素 `0xff0000ff`。`MEM_EXEC_INIT` 的 EPERM 已通过 EXEC_VA 先于 JIT 的顺序修复消除。**尚未验证 swapchain / WSI / Surface / eglinfo/glxinfo renderer**，不能把离屏 draw 当成 GPU 完成。Rust DRM 语义层作为已完成的 M1/M2 资产和后备保留。验收不变：guest 内 eglinfo/glxinfo 报出 Mali-G77 真实 renderer；硬件 vs llvmpipe 帧率对比表；不崩、可降级。
3）显示：guest 渲染面与 Android Surface 零拷贝。零拷贝方向已定（evidence 41）：r32 UAPI 只有 MEM_IMPORT、没有 export——所以是 app 侧分配 AHardwareBuffer/dma-buf → 交 guest → MEM_IMPORT 拿到 GPU VA → GPU 直接渲染进去 → app 把同一块 buffer queueBuffer 给 SurfaceFlinger；kbase 自建的中间 BO 走 CPU 拷贝回 app，开销实测并写进 evidence。包成 AHardwareBuffer → ANativeWindow queueBuffer + render-done fence 交 SurfaceFlinger，全部 Rust。验收：可交互、拖动不掉帧、旋转与 DPI 正确、多窗口正常、连续 10 分钟无泄漏。
4）视频：guest 里的播放器与 Chromium 都要出画出声，且必须是硬解，不许拿软解当完成。
   事实（evidence 39）：guest 直连硬解不可行——`/dev/video*` 是 `0660 media:system` 的相机采集节点，不是编解码器；厂商解码 .so 带 bionic DT_NEEDED，glibc guest 内加载不了。本机硬解器在 bionic 侧：`c2.mtk.avc.decoder` / `c2.mtk.hevc.decoder` / `c2.mtk.vp9.decoder` / `c2.mtk.av1.decoder`（取自 /vendor/etc/media_codecs_c2.xml），厂商自报 1080p 帧率 113–276 fps。
   形状：解复用 + 音视频同步留在 guest（ffmpeg/GStreamer 不动），压缩帧走独立通道（AF_UNIX socketpair + 共享内存 ring，不复用输入/剪贴板/音频通道）递到 app 进程里我们的 Rust 解码线程 → NDK `AMediaCodec`（dlopen libmediandk.so，纯 Rust 调用）→ 输出 AHardwareBuffer；优先经 MEM_IMPORT（0xc0188016）把 dma-buf 交回 guest，走第 3 条同一条零拷贝 Surface 路径；不能导出才退化 CPU 拷贝并写开销。不留独立解码进程、不留 gst 中转服务。
   接口（v11 补）：Chromium 与 GStreamer 都不直接调 AMediaCodec，它们说的是 VA-API。所以第 4 条的重心是**在 guest 内实现一个 Rust VA-API 后端（libva 驱动名 `husk`）**：`vaInitialize` / `vaQueryConfigProfiles` / `vaCreateConfig` / `vaCreateContext` / `vaCreateSurfaces` / `vaBeginPicture` / `vaRenderPicture` / `vaEndPicture` / `vaSyncSurface` / `vaExportSurfaceHandle`，把 bitstream 和参数经同一条通道送到 app 侧 AMediaCodec，解出的帧用 dma-buf 导出（走第 3 条零拷贝路径，导不出才 CPU 拷贝）。两条路共用这一个后端，只写一份：Chromium 侧不改不重编，只加 `--use-gl=egl --enable-features=VaapiVideoDecoder,VaapiIgnoreDriverChecks --ignore-gpu-blocklist`；GStreamer 侧用 `vah264dec` / `vah265dec` / `vavp9dec`（rootfs 缺这些元素就退回 `avdec_*` 软解并在日志写明原因）。VA 驱动同时提供 `vaQuerySurfaceAttributes`，让上层能拿到本机真实上限（分辨率 / 帧率 / profile）。
   验收：1080p30 H.264 与 HEVC 各连续 10 分钟不掉帧（丢帧 <0.5%，日志里有 dropped 计数）；CPU 占用（按核采样 /proc/pid/stat）硬解 vs 软解对比表；AV 同步偏差 <40ms 且写明测量方法；Chromium `<video>` 与 GNOME 播放器两条路径都出画；不支持的编码退化软解并写日志原因，任何一条都不许崩；退出后无残留解码线程与 fd；guest 内 `vainfo` 必须报出 `husk` 驱动与 H.264 / HEVC profile，日志里必须打出实际 decoder 名（c2.mtk.*）作为"确实硬解"的证据。
   依赖关系：不阻塞在第 2 条后面——dma-buf 回传没打通之前，先用 CPU 拷贝版本出画，数字照实写。
5）音频：/dev/snd 打不开，只能在 app 进程内用 AAudio 开流；guest ↔ app 用独立 socketpair 传 PCM（不与输入/剪贴板/视频争通道），Rust 实现。验收：guest 内可枚举输出设备、能出声、音量可控、延迟有实测数字并写明测量方法；与第 4 条同时跑时 AV 同步不劣化。
6）探测与降级：SoC 与节点能力写成代码里的表（/dev/mali0、/dev/kgsl-3d0、/dev/dri/renderD128、/dev/video*、ro.hardware.egl、ro.hardware.vulkan、ro.board.platform、MediaCodecList 里的 c2.mtk.\*）；适配器按芯片家族实现（Mali kbase / Adreno kgsl / DRM；MediaCodec 按 MIME + 厂商前缀选），不按机型做一次性适配；探测失败必须有明确降级路径（软件渲染 / 软解 + 日志说明原因），任何一条都不能崩。

【C. 工程纪律】
7）每步留可复现命令与证据（docs/design/evidence/、docs/STATUS.md、docs/GOALS.md），事实与推断分开标注；一项做完先上机验证再写"完成"，未验证就写未验证。判定「内核/驱动不支持某能力」前，必须先从真实客户端（厂商 .so、sysfs、/vendor/etc 配置）取到命令字或参数，再用最小合法参数打一次，禁止用猜的组合下结论（2026-11-22 JOB_SUBMIT 假阴性即此错）。收尾时设备息屏、会话不留僵尸。

【D. 落地：用 Husk 替换 LinuxHub 2.0 的 proot】
8）集成：Husk 达到第 2 条（GPU）验收后，替换 2.0 的 proot 运行时——`LinuxContainerManager.prootCommand()` 改成直接 exec 我们的 loader，2.0 的 UI/会话/桌面资产不动。基线包（可回滚，已校验）：`artifacts/rollback/LinuxHub-Myth-Prometheus.apk`，sha256 `c4951876599b5a8a388170952039a724959f5c48004aec8f92e1de7379adc2b7`，1,037,527,072 字节，与 `artifacts/release/` 逐字节一致。
9）一次性消除 proot 带来的补丁（FACT：以下均已在 2.0 源码中定位，消除后不得残留）：
   · 启动方式：`-0` 假 root、`--link2symlink`、`-b` 绑定挂载、`-w`、`-r`，以及 `PROOT_LOADER` / `PROOT_TMP_DIR` / 改写的 `TMPDIR`；
   · 依赖库：`libtalloc.so.2`、`libandroid-shmem.so` 及其拷贝与 `LD_LIBRARY_PATH` 指向；
   · 网络桥：`LD_PRELOAD=libgetifaddrs_bridge.so` 及 `/etc/profile.d/doro.sh` 里写死它的那行、`unset LD_PRELOAD` 补丁——由 Husk shim 的 interpose 直接提供 getifaddrs；
   · fake-root 副作用补丁：`ensureAndroidGroupNamesScript()` 往 `/etc/group` 注入 `android_gid_*`、靠 `/proc/self/status` 的 Groups 行绕开 proot 对 getgroups 的拦截——由 Husk 的 uid/gid 映射直接给出正确语义；
   · 伪 /proc 与设备补丁：`/proc/stat`、`/proc/version`、`/proc/bus/pci/devices`（Chromium 需要）、`/run/doro-audio` 之类为绕过 proot 而造的假文件；
   · GPU：`doro-gpu-run.sh` + virgl/virpipe + virgl server 进程——由原生 kbase 路径（第 2 条）替代，不留中转进程；
   · 音频：`/run/doro-audio` 桥——由 AAudio + socketpair 替代；
   · 视频：为在 proot 里凑合播放而加的东西（强制软解的 GST/LIBVA 环境变量、关闭硬解的配置、为绕行而设的软解参数通道）若存在，一并清除，改走第 4 条的 MediaCodec 硬解 + dma-buf 回传。
10）验收（全部可在设备上复现）：运行期 `ps -A` 里 proot 计数为 0、无 virgl server / 无独立解码进程；guest 内 `env | grep PROOT` 为空、`LD_PRELOAD` 为空；`/proc/<guest pid>/maps` 里无 `talloc` / `shmem` / `getifaddrs_bridge`；GNOME Shell 46 能起，浏览器 / 文件管理器 / 终端可用；`eglinfo` 报 Mali-G77 硬件 renderer；音频出声且延迟有实测数字；1080p30 H.264 与 HEVC 播放不掉帧、硬解 vs 软解 CPU 对比数字、AV 同步 <40ms、Chromium `<video>` 可播；回滚直接用上面那个 APK。

---

## 附一：怎么拿到 2.0 的 rootfs（不碰它的数据）

FACT：`com.linuxhub.myth` 不是 debuggable 包，`run-as` 直接拒绝（`package not debuggable`），
设备无 root，所以**读不到** `/data/user/0/com.linuxhub.myth/files/...`。这与签名无关，是 Android 沙箱。

**唯一资产来源是 LinuxHub 2.0（FACT，2026-11-22 实测目录）**：

```
/Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/app/src/main/assets/
  rootfs/ubuntu-base-24.04.4-base-arm64.tar                    31 MB
  offline/bootstrap-arm64.tar                                 349 MB
  offline/gnome-24.04-arm64.tar                               532 MB
  offline/vendor-apps-arm64.tar                               403 MB
  offline/mesa-for-android-container_27.0.0-devel-20261025-r2_ubuntu_noble_arm64.tar  12 MB
  offline/{bootstrap-packages.txt,gnome-packages.txt,vendor-packages.txt,vendor-dependencies.txt}
  desktop/            GNOME 默认值、图标、壁纸、zh_CN 翻译、会话总线配置、doro-fetch.sh
  desktop/profile/    2.0 的 GPU 分派脚本（doro-gpu-run.sh），作为对照基线
  x11/xkb-data.tar    X11 键盘数据
  runtime/arm64-v8a/{loader,proot,libandroid-shmem.so,libgetifaddrs_bridge.so,libtalloc.so.2}
  gpu/virgl/arm64-v8a/{libvirglrenderer.so,libepoxy.so,virgl_test_server_android}  ← 2.0 的 virgl 链，只作对照
```

**Asset list 到此为止：只用上面这份 2.0 的 24.04 链，没有其它来源。**

用法：直接复制进 Husk 私有目录（例如 `/data/data/com.linuxhub.husk/files/rootfs-full`），
**不读也不写** `com.linuxhub.myth`。

## 附二：2.0 的 GPU 现状（对照基线，不是要照抄）

- 判定逻辑：`DeviceGpuProbe.kt` —— 高通 Adreno + `/dev/kgsl-3d0` 可读写 → `zink`（Turnip）；
  联发科 + guest 内有 `virtio_gpu_dri.so`/`virpipe_dri.so` → `virgl`；否则 `software`。
- 实际执行：`desktop/profile/doro-gpu-run.sh`，GNOME Shell 本体固定 llvmpipe，硬件只给应用进程。
- 本机（MT6893 / Mali）走的就是 `virgl`：guest → virpipe → 主机侧 virgl server → vendor EGL。
  这条链里有一个独立服务进程和一段 virtio 传输，正是 Husk C5 要拿掉的部分。

## 附三：C 段的实测锚点（写代码前先看这张表）

| 事实 | 值 | 出处 |
|---|---|---|
| GPU 设备节点 | 只有 `/dev/mali0`（0666 root:root） | evidence 31 |
| kbase UAPI | 11.38（vendor 不校验请求版本） | evidence 31 |
| VERSION_CHECK 编码 | `_IOC(RW, 0x80, 0, 4)`；size=0 是 ENOTTY | evidence 31 |
| SET_FLAGS 编码 | `_IOC(W, 0x80, 1, 4)`，guest 内返回 0 | evidence 31 |
| 命令面（26 条，取自厂商 .so） | `0xc0048000` VERSION_CHECK / `0x40048001` SET_FLAGS / **`0x40108002` JOB_SUBMIT** / `0xc0208005` mem_alloc / `0xc0108006` mem_query / 其余见 evidence 35 表 | evidence 35 |
| JOB_SUBMIT | **可达**：`0x40108002` 传 `nr_atoms=0` 返回 EINVAL(22)，不是 ENOTTY；提交结构 `{u64 addr; u32 nr_atoms; u32 stride}`，**stride = 72** | evidence 35 / 36 |
| mem_alloc | in/out：返回 0，字段0 回写实际 flags（`0x200f`），字段1 回写 GPU VA（`0x41000`）；flags=0 时 ENOMEM | evidence 36 |
| mmap 规则 | `prot` 必须等于 `mem_alloc` 输入 flags 的低两位，offset = 回写 VA，len = va_pages<<12；不符即 EPERM；写 PROT_READ 映射 → SIGSEGV | evidence 36 |
| 上游 Mesa | 25.2.0 与 main 的 `src/panfrost/lib/kmod/` 只有 panfrost(DRM)/panthor/virtio，**无 kbase**；rootfs 无 panvk ICD | evidence 36 |
| `/dev/ion` | guest 侧 EACCES；app 侧曾成功 → 待复核 | evidence 26 / 31 |
| `/dev/dma_heap` | 不存在 | evidence 31 |
| `/dev/dri` | card0 EACCES，无 renderD128 | evidence 31 |
| `/dev/snd` | EACCES | evidence 31 |
| 厂商 EGL | 25 个 bionic DT_NEEDED → guest 内不可加载 | evidence 31 |
| `/dev/video0-4` | `0660 media:system`，相机采集节点（不是编解码器）→ guest 无法直连硬解 | evidence 39 |
| 本机硬解器 | `c2.mtk.{avc,hevc,vp9,av1}.decoder`（另有 mpeg4/h263/mpeg2/vc1/heif） | evidence 39 |
| 硬解容量 | 厂商自报 1080p：avc 113–248 fps / hevc 125–276 / vp9 114–251（未实测） | evidence 39 |
| 硬解可达面 | 只能从 bionic 侧 NDK `AMediaCodec` 调；guest 侧无路 | evidence 39 |
| 内核 kbase 自报版本 | `K:r32p1-00bet5(GPL)`（`GET_DDK_VERSION` 0x4010800d） | evidence 40 |
| atom 结构 | 72 字节；stride 必须 72（56 → EINVAL）；数组被 copy_from_user | evidence 40 |
| timeline fd | **不是完成通道**：任何被接受的 submit 都触发 POLLIN，read() → EINVAL | evidence 40 |
| 软作业 | 本机无效果（SOFT_EVENT_SET 不写显存）；`base_fence` 也不回写 sync_file fd | evidence 40 |
| `ro.hardware.egl` / `.vulkan` | `meow` / `mali` | evidence 26 |
| CSF / panthor | `CS_GET_GLB_IFACE`（`0xc0188033`）= **ENOTTY** → 内核只编了 JM，panthor 排除 | evidence 41 |
| kbase 导出 dma-buf | **不存在**（UAPI 无 export）；只有 `MEM_IMPORT`（`0xc0188016`，实测可达） | evidence 41 |
| rootfs 的 Mesa | **25.2.8**；`dri/` 有 `panfrost_dri.so` / `panthor_dri.so` / `zink_dri.so` / `swrast_dri.so` | evidence 41 |
| kinstr 语义 | `COMPLETE` 对"成功"和 `JOB_INVALID` **同义**；只有 `START` 能证明进了硬件槽 | evidence 41 |
| 硬件 atom 现状 | 用真实 GPU VA 当 jc 仍只有 `QUEUE→COMPLETE`、无 `START`；手搓 atom 到此为止 | evidence 41 |
| kbase 的硬件参数 | `GET_GPUPROPS` = `0x40108003`（_IOW）；变长记录流，返回 723 字节；panfrost 要的 24+16 条**全覆盖** | evidence 42 |
