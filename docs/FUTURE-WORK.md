# 后续工作备忘

记录方向与结论，供后续排期时直接取用。**每一条都还没开始，状态标在标题后。**
当前主线仍是运行时本体与产品壳；本文件里的事项一律排在产品壳之后。

---

## 1. GPU 加速桥（未开始，可行度高，工作量大）

### 1.1 三层可达性

| 层 | 内容 | 可达 | 说明 |
|---|---|---|---|
| 调厂商驱动 | Vulkan / GLES，由厂商 ICD 执行 | 是 | 与普通 App 渲染路径完全相同，硬件加速 |
| 共享显存 | `AHardwareBuffer` + dma-buf fd 跨进程 | 是 | NDK 正式接口，零拷贝 |
| 直连设备 | `/dev/kgsl-3d0`、`/dev/mali0`、`/dev/dri/renderD*` | **是（2026-11-22 修正）** | 原先写"否"是错的：`untrusted_app` 能打开 `/dev/mali0`（0666），ioctl 也能用（evidence 31/35/36）。`/dev/dri/*` 仍打不开 |

### 1.2 为什么以前在容器里只能软件渲染（四道墙）

1. **权限墙**：域按进程判定，PRoot/namespace 都不改变"这是普通 App"这一事实；
2. **ABI 墙**：Android 的 EGL 与厂商驱动链到 bionic，guest 是 glibc，同进程两套 libc 符号冲突；
3. **窗口墙**：EGL 需要 `ANativeWindow`（Surface），容器里的进程没有；
4. **视图墙**：guest 的 `/dev` 里没有 `dri/`，手工建节点回到第 1 条。

Husk 天然没有第 3 道（UI 与 Surface 在手上）；第 1、2、4 道依然存在。

> **2026-11-22 更新（重要）**：下面这张"bionic 侧中转"的架构已被 evidence 35/36 取代。
> MT6893 上 guest 进程能直接和 `/dev/mali0` 说话，所以路线改成：**guest 内的 Rust DRM 语义层**
> （拦 `open("/dev/dri/renderD128")` + `ioctl` + `mmap`，翻译成 kbase 命令字），发行版 Mesa 的
> panfrost/panvk 不改不重编。中转进程、virgl、SIG 传递全部不需要。以下内容保留作对照。

### 1.3 目标架构（已被上条取代，保留作对照）

```
guest 桌面（Wayland 合成器，多窗口）
   │ 窗口渲染到 dmabuf（Vulkan，零拷贝）
   ▼
guest 内合成器 → 一帧
   │ SCM_RIGHTS 传 fd + VK_KHR_external_fence_fd 同步
   ▼
bionic 侧进程 → EGL → Android Surface → 屏幕
```

### 1.4 难点排序

1. **合成器**：N 个窗口合成成 1 帧，是最大工作量；没有它"下载 Linux 桌面直接用"不成立；
2. **fence 同步**：跨进程 buffer 不带 fence 会撕裂；最容易出现难复现的缺陷；
3. **Surface 生命周期**：切后台、息屏、旋转、分辨率变化导致重建，整链要能重建；
4. per-SoC 格式 quirk：仅像素格式与 modifier 接受范围不同，运行时查询
   `VK_EXT_image_drm_format_modifier` 能力即可，不需要为高通/天玑分别写分支。

### 1.5 已实测的设备能力

| 探测 | 结果 |
|---|---|
| `/dev/ptmx` | **可打开** → PTY 可用，产品壳可以做真终端 |
| `/dev/kgsl-3d0` | **可打开**，`/sys/class/kgsl/kgsl-3d0/gpu_model` = Adreno825 |
| `/dev/dri/renderD128`、`/dev/dri/card0` | **拒绝** → 无显示控制通路 |
| guest 视图中的 `/vendor`、`/system`、`/apex` | **不存在** → guest 看不到厂商图形栈 |
| `/vendor/lib64/egl/*`、`vulkan.adreno.so` | 在 guest 中不可见 |

两条修正：

1. **"厂商内核节点对 untrusted_app 一律关闭"不成立**——本机 kgsl 是 `crw-rw-rw-` 且可打开。
   **2026-11-22 在 MT6893 上补测完成**：`/dev/mali0` 不仅能打开，kbase ioctl 也真的能用
   （`VERSION_CHECK` / `SET_FLAGS` / `mem_alloc` / `mmap` 全通，`JOB_SUBMIT` 命令可达），
   见 evidence 35 / 36。原"只当探测项"的保守假设可以撤销。
2. **桥必须放在 bionic 进程**——guest 连 `/vendor` 都看不见，无法自行加载厂商驱动。
   这不是推断，是实测。

### 1.6 可行度与相对工作量

| 目标 | 可行度 | 相对工作量 |
|---|---|---|
| 单窗口渲染到 Android Surface | 高 | 1 |
| 多窗口桌面（合成器） | 中 | 4 |
| per-SoC 格式 quirk | 高（收益小） | 0.3，按需 |

**顺序**：单窗口桥 → 合成器 → 按需 quirk。

---

## 2. 音频桥（最小闭环、持续路径与 10 分钟长稳已通，同步未完成）

- 设备节点 `/dev/snd/*` 标签 `audio_device`，`untrusted_app` 被拒，**不可能直驱 DAC**；
- 通路：guest 的 PulseAudio/PipeWire → PCM 经 fd 传递 → bionic 侧 AAudio → Android 混音 → 厂商 HAL；
- 2026-11-23 已实测：设备枚举、`STREAM_MUSIC` 音量、一次性 AAudio 写入（evidence 60），以及
  PulseAudio `module-pipe-sink` → 纯 Rust `audio-relay` → 持续 `OPEN/DATA/CLOSE` AAudio →
  标准 guest `paplay`（evidence 62）。10 秒 session 写入 87,035 frames / 348,140 bytes；
- 最终 loader 上的 610 秒连续会话也已通过（evidence 63）：28,818,432 frames /
  115,273,728 bytes，`PAPLAY_RC=0`、`RELAY_RC=0`，app/relay fd/RSS 稳定；最终统计仍有 1 次
  xrun，不能写成 xrun-free；
- 当前 48 kHz stereo、一个 burst（1026 frames）的 queue-depth latency 为 avg 104.680 ms /
  max 106.667 ms；厂商拒绝 low-latency 模式并返回 `perf=NONE`。目标 20–50 ms 仍未达到；
- GStreamer/桌面播放器、AV 同步仍是待办；Rust `audio-setup` 已完成两次幂等回归；
- **不需要 per-SoC 代码**，AAudio 已吸收差异；
- 相对工作量约为图形桥的三分之一；可与图形桥共用 fd 传递基础。

---

## 3. 发行形态（已定：独立 App）

只做独立 App 一种形态：自有界面、自有会话管理、自有图形与音频桥，可正常上架与更新。

不做 Termux 插件形态。原因：它把可执行性寄托在第三方的 targetSdk 政策上，
无法提供自己的界面与图形能力，也与"运行时是产品的地基"这一定位冲突。

---

## 4. 厂商"专门接口"可行度分层（结论已定，供产品决策）

| 类别 | 例子 | 可行度 |
|---|---|---|
| 厂商用户态驱动与扩展 | `EGL_QCOM_*`、`GL_QCOM_*`、Adreno/Mali Vulkan 扩展 | 高（本就用它们渲染） |
| 显存/缓冲接口 | gralloc、`AHardwareBuffer`、dma-buf | 高（NDK 正式接口） |
| 厂商内核设备节点 | `kgsl`、`mali0`、`dri`、`fastrpc-cdsp` | 接近零（域限制，非技术难度） |

结论：目标应写成"用厂商用户态驱动 + 标准扩展 + dmabuf 零拷贝"，而不是"绕过 Android 直连 SoC"。

---

## 5. 待验证的技术前提（阻塞后续设计）

| 项 | 影响 | 状态 |
|---|---|---|
| `open("/dev/ptmx")` 在 `untrusted_app` 是否可用 | 决定产品壳是 PTY 终端还是管道协议（后者失去 `vim`/`htop`） | **已测：可用**（evidence 31） |
| `/dev/dri/renderD128` 能否打开 | 决定图形走 dmabuf 共享还是更绕的路 | **已测：拒绝**；`/dev/mali0` 可开（evidence 31） |
| `/dev/fastrpc-cdsp` 访问权 | 决定 NPU 是否只剩 NNAPI | 本机该节点不存在（高通平台） |
| 16K 页内核 | 未实测（不是不支持） | 见 design/page-size.md |
| 多设备一致性 | 决定能否宣称"任意 aarch64 设备" | 仅一台验证 |
