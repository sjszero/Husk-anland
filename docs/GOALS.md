# Husk 目标（v52，2026-11-23）

## 总目标

Husk（纯 Rust 原生 Linux 运行时）在 **RMX3357**（MT6893 / Dimensity 1200 / Mali-G77，
Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64）上端到端跑通，做到能替代 LinuxHub 2.0 的
产品化程度。

「全硬件」= 该机**普通应用（`untrusted_app`，无 root）能合法访问的每一类硬件**：
GPU、显示、视频解码、视频编码、音频输出、音频输入、相机、输入、传感器，
以及平台允许的 USB / 蓝牙 / GPS / NFC / 指纹。

能直接打开节点的（`/dev/mali0`）走 guest 直连；被沙箱拒绝的（`/dev/snd`、`/dev/video*`、`/dev/dri`）
必须走 app 内平台 API / HAL + AHardwareBuffer / dma-buf / socket 桥，**但仍必须是硬件路径**，
禁止用软解、软件渲染、假设备或静态单帧冒充完成。缺任何一类不算完成。
跨机型适配不属于本阶段目标；单台设备失败不外推成平台结论。

## 硬约束

- 不 root、不解 BL、不开虚拟化。
- 运行时零 syscall 拦截（shim 只在必要处 interpose，不扩大拦截面）。
- 能 Rust 就 Rust（零 Java、零 Python、零 C；只有内核 ABI 强制的裸汇编 stub 例外）。
- 绝不读写 `com.linuxhub.myth` 的任何数据，只用自己私有目录里的副本。
- 动手前先确认有没有现成实现（笔记留在仓库外）；**仓库只放我们自己的东西**。
- 结论只按实测下，FACT / INFERENCE / UNKNOWN 分开标注。
- 每次正式改动提交本地并推 GitHub。

## 已完成（回归门禁，改动后必须重跑）

| 项 | 证据 |
|---|---|
| 运行时核心：loader + path-shim + exec / PTY / 信号 / 生命周期 | 27 / 28 / 32 / 34 / 72 |
| SIGSYS 拦截收敛：10 分钟长稳、零僵尸、无重启风暴 | 32 |
| 页大小：`nr-tools check-elf` 支持 4K/16K/64K 静态体检（16K/64K 实机 UNKNOWN） | 30 |
| GPU 计算与离屏：PanVK logical device、compute、离屏 graphics、Zink surfaceless | 46 / 47 |
| 显示零拷贝主路径：AHB → dma-buf → SurfaceControl，3-buffer ring、真实 SYNC_FD、release fence 背压、10,000 帧 / 665 秒、fd/RSS 无增长 | 53–59 / 73 |
| 视频 H.264：`c2.mtk.avc.decoder` + MediaCodec + AHB，1080p30 连续 600 秒、dropped=0 | 74–80 |
| 音频输出最小闭环：枚举、音量、PCM 经独立通道到 AAudio，610 秒 | 60 / 62 / 63 |
| 音频输入：声学回环 440 Hz | 88 |
| 相机：guest `/dev/videoN` V4L2，取流 / 拍照 / 录像 | 91 / 92 / 93 |
| 输入：触摸 / 键盘 / 鼠标经 mutter `RemoteDesktop` 注入 | 95 / 98 |
| 桌面：2.0 rootfs 起 GNOME Shell 46（Wayland 显示服务器） | 101 |

## A. 统一硬件后端层

1）Rust 侧定义统一硬件后端接口与能力表，按**设备类**而不是按机型组织：
GPU、display、video-decode、video-encode、audio-out、audio-in、camera、input、sensors、
platform-peripherals。每个后端必须有：能力探测、打开/关闭、错误码、降级路径、日志、fd 生命周期。
探测失败必须写明原因，任何一条不能崩。
验收：guest 与 app 两侧打印同一份能力矩阵；不支持项显示 UNKNOWN 或 DEGRADED。

## B. GPU 与显示

2）GPU：保持 guest 直开 `/dev/mali0` + PanVK，补齐 WSI / swapchain；需要时由 guest 合成器
直接使用 dma-buf / AHB，不经中转服务、不留独立 GPU 进程。
验收：eglinfo / glxinfo 报 Mali-G77；窗口连续渲染 10 分钟；硬件 vs llvmpipe 帧率对比表；
失败可降级且日志说明原因。

3）显示：SurfaceControl 零拷贝继续作为 Android 窗口出口；补齐多窗口、DPI、旋转后代际替换、
SurfaceView / Surface 生命周期、SurfaceFlinger 内部等待 trace。
验收：拖动不掉帧、旋转正确、多窗口可用、连续 10 分钟无泄漏。

## C. 视频

4）解码：在 H.264 基础上补齐 HEVC / VP9 / AV1；完成 VA-API 系统层（`vaEndPicture` 不再
UNIMPLEMENTED）、MediaCodec AHB 回传、decoder → guest GPU 零拷贝或明确记录 CPU 拷贝开销。
验收：每种编码 1080p30 连续 10 分钟、dropped < 0.5%、日志有实际 decoder 名、
硬解 vs 软解 CPU 对比表、AV 同步 < 40ms。

5）编码：MediaCodec 硬件编码路径，guest 能采集并编码。
验收：guest 内录 1080p30 H.264，文件可播放，编码器名与帧率有日志，CPU 占用有数字。

6）播放器：Chromium `<video>`、GStreamer、GNOME 播放器走同一 Rust 后端；
不支持的编码退化软解并写日志，不许崩。

## D. 音频

7）输出：在 610 秒基础上做到低延迟 20–50 ms；记录 xrun。
验收：延迟测量方法、avg/max 数字、10 分钟 xrun 计数。

8）输入：app 侧 AAudio 输入流 → 独立通道 → guest 录音应用。
验收：guest 能枚举麦克风、录音文件可播放、延迟有数字。

9）AV 同步：音视频同时跑 10 分钟，偏差 < 40ms，测量方法写清。

## E. 相机、输入、传感器

10）相机：app 进程用 Camera2 / Camera HAL 打开真实相机，输出 AHardwareBuffer / dma-buf，
经零拷贝路径交给 guest。禁止假摄像头、静态图、软件模拟。
验收：guest 预览有画面、拍照文件有效、录像 30 秒可播放、前后摄像头切换正确、权限拒绝有明确降级。

11）输入：触摸、软键盘、物理键盘、鼠标事件从 app InputQueue 转到 guest 合成器。
验收：guest 桌面可点击、可输入、可拖动窗口。

12）传感器：app 侧读加速度、陀螺仪、光线、距离等，经独立通道送 guest。
验收：guest 内能读到真实变化，无传感器时明确报 UNKNOWN。

## F. 平台外设

13）USB / 蓝牙 / GPS / NFC / 指纹按普通应用可用 API 逐项探测；能接的做 Rust 桥，
不能接的写成平台限制和原因，禁止绕过沙箱。每类都要有 guest 可见接口或明确不可用结论。

## G. 2.0 集成与补丁清除

14）上述硬件类验收通过后，替换 LinuxHub 2.0 的 proot 运行时，UI / 会话 / 桌面资产不动。
基线包只读保留：`artifacts/rollback/LinuxHub-Myth-Prometheus.apk`，
sha256 `c4951876599b5a8a388170952039a724959f5c48004aec8f92e1de7379adc2b7`。

15）逐项删除 proot 专用补丁：`-0`、`--link2symlink`、`-b`、`PROOT_LOADER`、`PROOT_TMP_DIR`、
`libtalloc`、`libandroid-shmem`、`getifaddrs` bridge、伪 `/proc`、`doro-gpu-run.sh`、
virgl / virpipe、`/run/doro-audio`、强制软解环境变量。每删一项都要有替代能力已验收的证据。
验收：`ps` 中 proot=0、无 virgl server、无独立解码进程；guest 内 `PROOT` 变量为空、
`LD_PRELOAD` 为空；2.0 的 GNOME、浏览器、文件管理器、终端、相机、音频、视频全部可用。

## H. 工程纪律

16）每步留可复现命令与证据（`docs/design/evidence/`、`docs/STATUS.md`、
`docs/RESULTS.md`）；先上机验证再写完成；未验证写 UNKNOWN。
判定「内核/驱动不支持某能力」前，必须先从真实客户端（厂商 `.so`、sysfs、`/vendor/etc`）
取到命令字或参数，再用最小合法参数打一次，禁止用猜的组合下结论。
设备操作不用 `rm -f`；收尾息屏、精确清理，不误杀 guest 会话。
