# Husk 项目长期记忆与接手手册

> 最后更新：2026-11-23
> 用途：上下文被压缩或换人接手时，先读本文件，再读 `GOALS.md`、`STATUS.md` 和证据文件。
> 这里记录**长期有效的目标、约束、决策、路线与操作纪律**；瞬时调试细节放到 `docs/design/evidence/`。

## 0. 一句话目标

Husk 是一个 Android 用户态 Linux 运行时：让完整 glibc 发行版以普通应用进程的身份，由 Android 内核直接执行；
不 root、不解 BL、不开虚拟化、不用 ptrace 拦截系统调用。最终必须做到能替代 LinuxHub 2.0 的 proot，
并让 GPU / 显示 / 视频 / 音频四条 SoC 直连链路真正可用。

权威验收标准在 [`GOALS.md`](GOALS.md)；当前事实进度在 [`STATUS.md`](STATUS.md)。

## 1. 产品边界（USER_STATED）

- 目标仓库：`/Users/l/Documents/Codex/2026-08-10/husk`，公开远端 `github.com/xishanp/Husk`。
- 产品名：**Husk**；一次运行中的发行版实例称为一个 husk。
- Husk 是运行时、载体，不是终端模拟器，也不是虚拟机或 root 容器。
- 终端只是演示前端；不要为了终端渲染、桌面 UI、输入法外观等非运行时问题扩大战线。
- 要解决的是：在不 root、不解 BL、不开虚拟化的普通 Android 应用权限下，运行完整 Linux 用户态，
  并直接使用 SoC 能力。
- 最终运行位置是 LinuxHub 2.0 应用本体；Husk 不是要求用户先装 Termux，也不是要求额外服务端。
- Shizuku / ADB 只作为设备测试和部署手段；当前运行时不得依赖它们，也不把权限提升当作产品前提。
- 适配按 SoC / GPU 家族做（Mali kbase、Adreno KGSL、DRM/MediaCodec/AAudio 等），
  不能只按某一台手机做一次性硬编码；单台旧设备的失败不能外推到所有设备。

## 2. 硬约束（USER_STATED）

1. 不 root、不解锁 Bootloader、不开硬件虚拟化。
2. 运行时零 syscall 拦截：不引入 ptrace / SIGSYS 仿真作为常规执行路径；
   只在启动前或必要 ABI 边界做最小 interpose。
3. 能 Rust 就 Rust：产品代码零 Java、零 Python、零 C；
   只有内核 ABI 强制的裸汇编 stub 例外，且必须单独标注原因。
4. 绝不读写 `com.linuxhub.myth` 的任何数据；2.0 资产只复制到 Husk 私有目录使用。
5. LinuxHub 2.0 的既有分支和数据不能动；所有正式工作只在 Husk 仓库和独立私有副本中做。
6. 事实、推断、未知必须分开写；没上机验证就不能写“完成”。
7. 每次正式改动都要提交本地并推 GitHub；不能只留在工作区或聊天里。
8. 收尾时设备息屏，不留下僵尸进程、调试服务或临时中转进程。

## 3. 当前已完成的回归门禁（FACT）

- 普通应用域 `untrusted_app` 里能跑 glibc bash 和常用工具；功能矩阵 31/31，越界测试 0 escapes。
- gcc 13.3 + make 全链通过；交互式 PTY 会话可用。
- 生命周期：旋转、切后台、Surface 重建不重启会话；唯一 reaper + `PR_SET_CHILD_SUBREAPER`；
  10 分钟长稳通过（600s / 11360 次 fork-exec / 零僵尸）。
- 页大小无关：构建期 over-align + `nr-tools check-elf`（4K/16K/64K）+ embed 同余断言。
- 2.0 的 Ubuntu 24.04 rootfs 能起 GNOME Shell 46、mutter、gjs、dbus、Xwayland。
- guest 进程能打开 `/dev/mali0`，kbase UAPI 11.38 可建上下文；
  `VERSION_CHECK` / `SET_FLAGS` / `MEM_ALLOC` / `mmap`（CPU 读写显存）已验证。

对应证据从 [`docs/design/evidence/`](design/evidence/) 读取；不要只引用本文件。

## 4. GPU 路线（2026-11-23 更新）

### 4.1 不变的验收目标

- guest 进程自己打开 `/dev/mali0`，直连厂商 kbase。
- 不经独立 GPU 服务、不经 virgl/virpipe、不留中转进程。
- `eglinfo` / `glxinfo` 必须报出 Mali-G77 真实 renderer；硬件 vs llvmpipe 有帧率对比；
  失败必须可降级且写明原因。

### 4.2 已发生的事实

- 上游 Mesa 25.2 / main 没有 kbase 后端；Ubuntu 25.2.8 的 panfrost 模型表也没有 G77/G78。
- 我们已用 Rust DRM 语义层让 Mesa panfrost 在 guest 内加载，并从 kbase 读到真实 GPU 参数
  （PROD_ID `0x9080` 等，evidence 45）。
- 但 `panfrost_open_device()` 在发出任何 `CREATE_BO` 之前失败，`egl: failed to create dri2 screen`。
  把产品号伪装成 G57 仍失败，所以不能继续把它当作唯一实现路线。
- 前人已有 Mali-G77 + kbase + PanVK 的实机实现：
  `faux123/panvk-faux123-driver-rp4pro`（Mali-G77 MC9，r32p1，Android 13，kernel 4.19.191，
  与本机高度接近）以及通用 panvk kbase 工程 `zenithblue-oss/panvk-kbase-android`。
  这是 JUDGMENT 下的优先复用对象。
- evidence 47：FristOneRR PanVK + 发行版 Zink 已通过 **surfaceless EGL**，
  `eglinfo -B` / 纯 Rust `gl_probe` 均报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；
  壳应用已自动注入 PanVK/Zink 环境并使用 `rootfs-full`。GBM/Wayland/X11/Surface/WSI 仍未通。
- evidence 48：PanVK 报实例扩展 18 项、设备扩展 171 项；支持 raw dma-buf 的
  `VK_EXT_external_memory_dma_buf` / `VK_KHR_external_memory_fd`，但缺
  `VK_KHR_android_surface` 与 `VK_ANDROID_external_memory_android_hardware_buffer`。
  窗口路线必须是 app 侧 AHB → dma-buf fd → `SCM_RIGHTS` → guest 导入；AHB 句柄导出 fd 仍未知。

### 4.3 当前主动路线（JUDGMENT，未完成）

1. 用 `FristOneRR-Admin/FristOneRR-Panvk-Source`（commit
   `0c9dce35094dbe89c412b79bf57c6067c7b97e59`，Mesa 26.3.0-devel）交叉编出 **glibc 版**
   `libvulkan_panfrost.so`。选择它的原因是源码同时具备 G77 型号行和 kbase 的
   CPU-backed sync（替代 kbase fd 上不可用的 DRM syncobj）。
2. **已完成（evidence 46）**：`vulkaninfo --summary` 和纯 Rust `vulkan-probe` 都能枚举 Mali-G77；
   probe 已提交 SPIR-V compute job，fence 后读回 GPU 写入 `0x4855534b`，并完成离屏 graphics
   三角形 draw + 像素回读 `0xff0000ff`。
3. **已完成（evidence 46）**：把 `MEM_EXEC_INIT` 移到 `MEM_JIT_INIT` 之前，并把 EXEC_VA 设为
   4G；本机 warning 消失。可复现 patch 见 `docs/design/patches/fristonerr-exec-va-order.patch`。
4. **已完成（evidence 47）**：发行版自带 Zink 已走这颗 PanVK，surfaceless `eglinfo` 与纯 Rust
   `gl_probe` 报出 Mali-G77 renderer。下一步只剩真正的 GBM/Wayland/X11、swapchain、WSI、
   Android Surface 和窗口显示；headless renderer 不等于窗口系统验收。
5. `funnymdzz/mesa` 的 `kbase-v0.1.2` 已实测卡在 `drmSyncobjCreate()`，保留为失败证据和对照，
   不再作为唯一主动路线。
6. DRM 语义层代码保留：它是已经完成的 M1/M2 资产，也是 panfrost 路线的后备，不删除。

### 4.4 禁止重复的坑

- 不要盲扫 kbase ioctl；2026-11-22 的盲扫曾触发 kernel panic。
- 不要假设 `MEM_ALLOC_EX` 存在；MTK 已知可能返回 ENOTTY，要回退 legacy `MEM_ALLOC`。
- 不要用“提交返回 EINVAL”直接推断命令不存在；必须先从真实客户端取命令字和参数。
- 不要把 `COMPLETE` 当成成功证据；kinstr 的 `START` 才能证明进入硬件槽。

## 5. 显示 / 视频 / 音频长期形状（USER_STATED + JUDGMENT）

### 显示

- app 侧分配 AHardwareBuffer / dma-buf，交给 guest；
- evidence 49 已实测 app 侧 AHB native handle 含 dmabuf fd，且 Unix socket 收发成功；
  native-handle fd 顺序不固定，必须扫描 `/proc/self/fd` 找 dmabuf；
- guest 首选用 PanVK `VK_EXT_external_memory_dma_buf` / `VK_KHR_external_memory_fd` 导入；
  kbase `MEM_IMPORT` 作为后备；
- 同一块 buffer 由 app `queueBuffer` 给 SurfaceFlinger，目标是零拷贝；
- kbase 没有 export，所以方向必须是 app → guest 导入，不能假装有 kbase 导出。
- 退化路径可以 CPU 拷贝，但必须记录原因和实测开销。

### 视频

- guest 保留解复用、音视频同步、播放器/Chromium/GStreamer；
- 压缩帧通过独立 socketpair + 共享内存 ring 递到 app 进程；
- app 内纯 Rust 调 NDK `AMediaCodec` 硬解（本机 `c2.mtk.*`）；
- 输出优先 AHardwareBuffer / dma-buf 回传 guest，不能导出才退化 CPU；
- Chromium / GStreamer 通过 guest 内 VA-API 兼容层共用同一后端；
- 必须证明实际 decoder 名、1080p30 H.264/HEVC 长稳、硬解 vs 软解 CPU 对比、AV 同步 <40ms。

### 音频

- guest 内 `/dev/snd` 不可用；
- app 进程内用 AAudio 开流；
- guest ↔ app 使用独立 socketpair/共享 ring 传 PCM，不复用输入/剪贴板/视频通道；
- 验收输出设备枚举、能出声、音量可控、延迟数字和测量方法。

## 6. 2.0 替换与 proot 清除

GPU 验收后替换 2.0 的 proot 启动方式，UI/会话/桌面资产不动。一次性清除：

- `-0` 假 root、`--link2symlink`、bind mount、`PROOT_LOADER` / `PROOT_TMP_DIR` / TMPDIR 改写；
- `libtalloc.so.2`、`libandroid-shmem.so`、`LD_LIBRARY_PATH` 补丁；
- `libgetifaddrs_bridge.so` 及相关 profile 行；
- fake-root 注入的 `/etc/group`、`getgroups` 绕行；
- 为绕过 proot 造的 `/proc/stat`、`/proc/version`、`/proc/bus/pci/devices` 等假文件；
- `doro-gpu-run.sh` / virgl / virpipe / virgl server；
- `/run/doro-audio` 音频桥；
- 为 proot 妥协的软解 / LIBVA / GST 环境变量和参数通道。

验收：运行期 `ps -A` 中 proot = 0，无 virgl server、无独立解码进程；
guest 内 `env | grep PROOT` 为空、`LD_PRELOAD` 为空；GNOME、浏览器、文件管理器、终端可用；
eglinfo 报 Mali-G77；音频出声并有延迟数字；H.264/HEVC 硬解、AV 同步、Chromium `<video>` 通过。

## 7. 前人植树：参考仓库纪律

- 原始用户清单：`docs/design/reference-request-20261122.md`（逐字保留，仅作资料）。
- 功能归并结论：`docs/design/references.md`。
- 120/120 逐条矩阵：`docs/design/reference-audit-matrix.md`；机器快照：
  `docs/design/reference-audit-20261123.json`。
- 2026-11-23 审计口径：69 条 GitHub、45 条官方上游/组件、6 条 UNKNOWN；
  GitHub 身份用 `gh api` 核对，官方源用代理实测 HTTP 200，UNKNOWN 不猜链接。
- 不是每个仓库都逐行读过。A 级 28 条是下一阶段优先源码阅读面，不能把“已定位”冒充“已采纳”。
- Mali kbase 源码级规格：`docs/design/mali-kbase-prior-art.md`。
- 规则：先审计别人怎么做，再决定自己写；清单里的名字不等于真实仓库；参考实现不等于可直接依赖。
- 只把**当前会用到的**仓库拉到 `/tmp/refs2`，记录 URL + commit；不要把大仓库 vendoring 进 Husk。
- 已拉取/使用的参考源（2026-11-23）：

| 仓库 | commit | 用途 |
|---|---|---|
| `faux123/panvk-faux123-driver-rp4pro` | `cb72a0ba2ec7f160b818917be0c826bf4d57750b` | G77 + kbase + Android 13 / r32p1 实机先例 |
| `BkaNeko/panvk-mtk-driver` | `594efbf6d2038df81ce0a75603cce72f4c866879` | MTK `MEM_ALLOC_EX` / AFBC / CSF 回退补丁 |
| `manoedinata/mesa-kbase-j4lte` | `3fd6fc3f0271d2538f93bc5a51a19cedf267b8dd` | libdrm-kbase / panfrost 语义规格 |
| `zenithblue-oss/panvk-kbase-android` | `cf70e02b918e1864cbcd746a7fefa57b96c673ce` | 通用 Mali kbase PanVK 构建、profile、验证 |
| `FristOneRR/FristOneRR-Panvk-Driver` | `e62405dbe3c9bea626a4afe534746180cea06586` | 发布二进制审计；bionic 依赖，不能直接用于 glibc guest |
| `FristOneRR-Admin/FristOneRR-Panvk-Source` | `0c9dce35094dbe89c412b79bf57c6067c7b97e59` | **当前 glibc PanVK 交叉构建源**；G77 + kbase CPU sync + JM |
| `coderredlab/proroot` | `c01039ab017448263a4ddbd9221ecb88e737553c` | **零 ptrace 原生 runtime 的产品旁证**；仅二进制、专有，不能作为依赖或抄代码 |
| `funnymdzz/mesa` tag `kbase-v0.1.2` | `4e5323ef35dc580a6140f8d433a6d13ae3724c07` | 已实测 `drmSyncobjCreate()` 失败；保留为对照 |

## 8. 网络与代理

- GitHub 直连失败时使用本机代理：`127.0.0.1:7897`（HTTP/SOCKS mixed）。
- 只在单条命令或当前 shell 临时启用；操作完成立即 `unset`，不要永久写入环境。
- 示例：
  `GIT_CONFIG_GLOBAL=/dev/null git -c http.proxy=http://127.0.0.1:7897 clone ...`
- 提交推送优先直连；失败再用代理，不把代理当作运行时依赖。

## 9. 设备与资产

- 当前测试机：RMX3357 / MT6893 / Dimensity 1200 / Mali-G77 / Android 13 /
  kernel 4.19.191 / 4 KiB / arm64 / `untrusted_app`。
- 2.0 资产唯一来源：`/Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/app/src/main/assets/`。
- 回滚 APK：`artifacts/rollback/LinuxHub-Myth-Prometheus.apk`，
  sha256 `c4951876599b5a8a388170952039a724959f5c48004aec8f92e1de7379adc2b7`。
- 设备操作不要用 `rm -f`；使用新文件名或显式路径，避免误删。
- 不要在这台旧设备上硬试已知不支持的节点；设备不支持某项能力时记录为设备事实，不推广成平台结论。

## 10. 下一次接手的顺序

1. 读本文件、`GOALS.md`、`STATUS.md`、`references.md`、`mali-kbase-prior-art.md`。
2. 查 `git status --short --branch`，确认没有未提交改动；没有用户授权不要动 LinuxHub 2.0 分支。
3. 继续 GPU 主线：FristOneRR glibc PanVK 已完成 compute job、离屏 graphics draw + GPU readback
   （evidence 46），发行版 Zink 的 surfaceless renderer 也已通过（evidence 47）；
   下一步接 GBM / Wayland / X11、swapchain / WSI / Android Surface 和实际窗口显示。
4. 每次失败也写清楚失败点和下一假设；证据放 `docs/design/evidence/`。
5. 窗口 renderer 与 Surface 路径通过后再进入显示零拷贝；视频、音频按第 5 节并行设计但不能跳过硬解验收。
6. 最后做 2.0 proot 替换与补丁清除。
