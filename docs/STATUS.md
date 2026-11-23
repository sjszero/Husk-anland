# 当前状态

> 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64），
> 运行域 `untrusted_app`，无 root、无 Shizuku。
> 只写**已上机验证**的结论；未验证写 UNKNOWN。每条的原始输出在 `docs/design/evidence/`。

## 一句话

运行时核心（loader + path-shim + exec/PTY/信号/生命周期）、服务器表面（`/proc`、`/dev/log`、
装包）和 GPU、显示、视频、音频、相机、输入六类硬件都已在该机跑通；
**现在卡在最后一段：GNOME 桌面把帧交给 Android Surface**。
传感器、GPS、USB / 蓝牙 / NFC / 指纹尚未开始；2.0 的 proot 运行时尚未替换。

## 已完成（回归门禁，改动后必须重跑）

| 项 | 结论 | 证据 |
|---|---|---|
| loader + path-shim | 零 ptrace，路径一次改写完成；dlopen 进来的库也覆盖 | 27 / 100 |
| exec / PTY / 信号 / 生命周期 | 600 秒长稳、零僵尸、无重启风暴；旋转与 Surface 重建不重启会话 | 32 / 72 |
| 页大小 | `nr-tools check-elf` 支持 4K/16K/64K 静态体检（16K/64K 实机 UNKNOWN） | 30 |
| GPU | PanVK 列举 Mali-G77、compute、离屏 graphics、Zink surfaceless | 46 / 47 |
| 显示 | AHB → dma-buf → SurfaceControl；3-buffer ring + 真实 SYNC_FD；10,000 帧 / 665 秒 | 53–59 / 73 |
| 视频 | H.264 1080p30 连续 600 秒，dropped=0；VA-API 解码出真实帧 | 74–80 / 85–87 |
| 音频输出 | PulseAudio → Rust relay → AAudio，610 秒连续会话 | 60 / 62 / 63 |
| 音频输入 | 声学回环 440 Hz 通过 | 88 |
| 相机 | guest `/dev/videoN` V4L2，GStreamer 取流、拍照、录像 | 91 / 92 / 93 |
| 输入 | 触摸/键盘/鼠标经 mutter `RemoteDesktop` 注入 guest 合成器 | 95 / 98 |
| 桌面 | 2.0 rootfs 起 GNOME Shell 46（Wayland 显示服务器），GTK4 客户端可连 | 101 |
| 装包 / dpkg | `apt-get install` 全通（含 postinst 与 update.d 钩子），六个独立缺陷已修 | 106 |
| 服务器表面 | `/proc` 合成、`/sys/class/net`、`/dev/log` 落地；70 PASS / 6 PARTIAL / 6 FAIL | 107 |

## 正在进行：桌面帧出屏

- mutter 46 自带 `org.gnome.Mutter.ScreenCast`；已实测能建 session / stream，`state=running`（99 / 102）。
- 已排除：厂商 GPU 用户态库直接进 guest（25 个 bionic `DT_NEEDED`）；
  自研合成器替代 gnome-shell（与产品形态冲突）。
- 现在走协议层：ScreenCast → PipeWire 节点 → dma-buf → AHardwareBuffer → SurfaceControl。
- buffer 类型已有答案（evidence 103）：mutter 46 两条路都支持，**由消费者协商决定**——
  回传的格式带 `SPA_FORMAT_VIDEO_modifier` 就给 dmabuf，不带就给 memfd。零拷贝取决于我们自己。
- 已修掉「有 session bus 时 PipeWire 客户端卡死」（evidence 104）：pipewire 默认走 legacy
  access 模式，会用 `openat("/proc/<pid>/root")` 判断是不是 flatpak；我们的 shim 有意挡住
  `/proc/*/root`，内核回 `ENOENT` 而不是它预期的 `EACCES`，于是它把客户端当 flatpak 永久挂起。
  改用 `access.socket = unrestricted`（配置文件 `tools/guest-config/pipewire.conf`）后，
  `PipeWireStreamAdded` 正常到达、Produce 节点（`node=Some(32)`，`Stream/Output/Video`）可见，
  `EnumFormat` = BGRx/BGRA 1280x720。

## 下一步（按顺序）

1. 写 Rust 消费者：在 `EnumFormat` 里带 `SPA_FORMAT_VIDEO_modifier`，确认 mutter 按 dmabuf 分配。
2. 帧消费者：PipeWire → dmabuf → AHB → SurfaceControl（复用已验证的 3-buffer ring）。
3. GNOME 桌面可用性验收：浏览器、文件管理器、终端。
4. 传感器（IIO sysfs 合成树 + NDK `ASensorManager`，纯 Rust）、GPS（`/dev/gps0` NMEA）。
5. USB（usbfs ioctl 翻译）/ 蓝牙（org.bluez D-Bus 门面）/ NFC（读卡器姿态）；
   指纹按平台限制记为不可用。
6. 2.0 集成：替换 proot 运行时，逐项删除 proot 专用补丁。

## 已知环境细节（踩过的坑，别再踩）

- loader 只接受**相对**程序路径（`usr/bin/bash` ✓，`/usr/bin/bash` ✗）。
- `run-as` 下不能写 `/tmp`，临时文件放 `$R/tmp/`。
- 不要用 `dbus-daemon --fork`：同一 address 会出现多个 daemon，之后所有客户端收不到回应。
  用 `--nofork` 自己后台化，`XDG_RUNTIME_DIR` 必须 `chmod 700`。
- `--library-path` 是替换不是追加，必须显式带 `$R/lib/aarch64-linux-gnu` 与 `.../gvfs`。
- rootfs 缺包会导致与运行时无关的怪故障，清单见 `docs/design/rootfs-packages.md`
  （gdk-pixbuf 的「所有图片格式都不认识」就是缺 `shared-mime-info`）。
