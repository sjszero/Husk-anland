# 实测结果（RMX3357 / Android 13 / 4.19.191 / 4 KiB / untrusted_app）

> 只列**设备上跑出来的数字**；每条后面能指回 `docs/design/evidence/`。
> 未测的写在文末 UNKNOWN，不写 PASS。

## 1. 运行时

| 项 | 结果 | 证据 |
|---|---|---|
| loader 内嵌 shim 启动 glibc `bash` | 正常，路径全部落 guest 根 | 27 |
| `nativeLibraryDir` 里的 loader 被 app 域 exec | 正常（`files/` 不可执行由 29 确认） | 28 / 29 |
| 动态解释器与 shebang 递归 exec | 正常，子进程不再需要外部包装 | 34 |
| SIGSYS 拦截收敛 + 长稳 | 600 秒 / 11,360 次 fork-exec / 零僵尸 | 32 |
| 页大小静态体检 | 4K/16K/64K 三种假设都通过 `nr-tools check-elf` | 30 |
| dlopen 进来的库也要走 guest 路径 | 已实现（重扫 link map，只补新对象） | 100 |

## 2. 桌面

| 项 | 结果 | 证据 |
|---|---|---|
| 2.0 完整 rootfs 起 GNOME Shell 46 | 起成 Wayland 显示服务器，GTK4 客户端可连 | 101 |
| mutter 46 headless on PanVK | 正常起，ScreenCast / RemoteDesktop 接口在 | 97 |
| 输入注入 | app 侧事件经 mutter `RemoteDesktop` 进入 guest 合成器 | 95 / 98 |
| ScreenCast 虚拟监视器 | session / stream 创建成功，`state=running` | 99 / 102 |

## 3. GPU 与显示

| 项 | 结果 | 证据 |
|---|---|---|
| PanVK 列举 Mali-G77 MC9 | logical device、queue、host-visible buffer、`vkCmdDispatch` 回读通过 | 46 |
| 离屏 graphics | 真实顶点缓冲 + SPIR-V + `vkCmdDraw`，像素回读正确 | 46 |
| Zink surfaceless | `eglinfo` 报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` | 47 |
| AHB → dma-buf → PanVK 导入 | external 2D image，GPU clear 后回读一致 | 49 / 50 / 73 |
| AHB → `SurfaceControl` 直接上屏 | 静态帧通过，SF 层树可见 `husk-ahb` | 53 |
| 3-buffer ring | 连续 12 帧、slot 复用、SF release fence 背压 | 54 |
| eventfd completion bridge | 连续 120 帧；短 soak 5,600 帧 / 134 秒 | 55 / 56 |
| 真实 pending `sync_file` | guest 内生成，`status 0→1`，同一 fd pending→readable | 57 |
| PanVK `vkGetFenceFdKHR(SYNC_FD)` 接入 ring | 120/120 帧真实 sync_file，其中 11 帧导出时 pending | 58 |
| 3-slot 持久资源池 | 10,000 帧 / 665.385 秒，fallback=0，fd/RSS 无增长 | 59 |

## 4. 视频

| 项 | 结果 | 证据 |
|---|---|---|
| MediaCodec 三平面 YUV AHB → PanVK | 三平面 GPU 哈希校验一致 | 74 |
| 逐帧交接 | 150 帧逐帧完成 | 75 |
| 持久单连接（最多 3 个在途 AImage） | 收敛为持久流 | 76 |
| H.264 1080p30 长稳 | 连续 600 秒，显式 dropped=0 | 80 |
| VA-API 解码 | `vaapih264dec` 出真实帧，150 帧交接 | 85–87 |

## 5. 音频

| 项 | 结果 | 证据 |
|---|---|---|
| 设备枚举与音量 | guest 枚举 7 个 Android 输出设备，控制 `STREAM_MUSIC` | 60 |
| PulseAudio `module-pipe-sink` → 纯 Rust relay → AAudio | 标准 `paplay` 可播放 | 62 |
| 连续会话 | 610 秒无中断 | 63 |
| 麦克风回环 | 声学 440 Hz 回环通过 | 88 |

## 6. 相机与输入

| 项 | 结果 | 证据 |
|---|---|---|
| guest `/dev/videoN` V4L2 | `QUERYCAP` / `ENUM_FMT` / `REQBUFS` / `QBUF` / `DQBUF` 可用 | 91 |
| GStreamer 取流 | `v4l2src` 正常出帧 | 92 |
| 拍照 / 录像 | 静态图有效，录像可播放 | 93 |

| PanVK 发行打包 | 1.2.0 快照随 bundle 安装；干净树 GPU 默认值 9/0、门禁 A 34/0/1、读回 17/0 | 306 |

## 7. 未验证（UNKNOWN）

- WSI 的**零拷贝**方向：窗口呈现已通（X11 EGL/GLX，600 秒；evidence 165、245、278），未通的是 dma-buf 直传（DRI3 无 render node），现在是回读 + SHM 拷贝。
- SurfaceFlinger 内部等待行为的 trace。
- 16 KiB / 64 KiB 页内核上的实机运行。
- HEVC / VP9 / AV1 解码；硬件编码。
- 相机帧的零拷贝路径（当前是拷贝）。
- GPS、USB、蓝牙的实际传输链路；传感器已在中转路径上验证 5 类默认传感器，但 `iio-sensor-proxy` 的 D-Bus 输出未验；NFC、指纹归“不需要”。
- ScreenCast 流的 buffer 类型已按源码定论（消费者带 modifier 就是 dmabuf），实测被
  「有 session bus 时 PipeWire 客户端卡住」挡住，见 evidence 103。
