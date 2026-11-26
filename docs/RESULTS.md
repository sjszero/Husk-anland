# 实测结果

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页 /
`untrusted_app`。历史型号只是参考环境，不是产品绑定。

本文只列设备上实际跑出来的结果。未验证的写在末尾，不把编译成功或旧证据当成本轮通过。

## 1. 最终发行树

| 项目 | 结果 | 证据 |
|---|---|---|
| 空目录安装 | `INSTALL_RC=0` | 308 |
| Tree Integrity | `PASS=23 FAIL=0` | 308、310 |
| Gate A | `PASS=34 FAIL=0 SKIP=1` | 310 |
| A-hw | `PASS=11 FAIL=0` | 310 |
| GPU loader defaults | `PASS=9 FAIL=0 SKIP=0` | 310 |
| dpkg | 807 installed / 807 `.list` | 308 |
| 中文 locale | 6,222,864-byte locale archive，413 个 zh_CN `.mo` | 308 |

Gate A 的 SKIP 是离线树没有 apt package index。没有把它算成 PASS。

## 2. 运行时

| 项目 | 结果 | 证据 |
|---|---|---|
| loader 启动 glibc bash | 正常，路径落在 guest 根 | 27、310 |
| 动态解释器、shebang、嵌套 exec | 正常 | 34 |
| PTY、信号、进程视图 | 门禁 A 通过 | 310 |
| apt / dpkg 元数据 | 807 包记录完整；离线 apt index 缺失 | 308、310 |
| 用户与组管理 | `useradd` / `groupadd` / `chpasswd` / `id` 通过 | 310 |
| 默认中文 locale | `zh_CN.UTF-8`，显式英文不会被覆盖 | 310 |
| 最终树 App bridge | Camera2 与 AAudio 都能到达活动树 socket | 310 |

## 3. PRoot 同机对比

2026-11-27 单次现场采样：

| 工作量 | Husk | PRoot | 结果 |
|---|---:|---:|---|
| `stat` × 20,000 | 741.2 ms | 2,338.7 ms | Husk 快 3.16× |
| `open` + `close` × 20,000 | 980.9 ms | 5,676.3 ms | Husk 快 5.79× |
| shell 循环 × 1,000 | 18.2 ms | 51.5 ms | Husk 快 2.84× |
| `/bin/true` × 50 | 708.5 ms | 323.6 ms | **Husk 慢 2.19×** |

这不是 5 轮热漂移修正基准，不能替代 [`BENCHMARKS.md`](BENCHMARKS.md) 的轮转数据。短进程启动
成本是真实结果，照实记录。

## 4. GPU

| 项目 | 结果 | 证据 |
|---|---|---|
| Mali-G77 MC9 枚举 | 通过 | 156 |
| PanVK compute + 离屏 graphics | 通过 | 156 |
| Zink/EGL/GL 默认渲染器 | `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` | 310 |
| 软件负向控制 | `NR_GPU_DEFAULT_OFF=1` 与 `HUSK_GPU=llvmpipe` 都落到 llvmpipe | 310 |
| 4 路并发 × 300 秒 | 每路 579 fps，fd/RSS 不涨，GL 错误 0 | 292 |
| 固定管线 | 320 fps | 292 |
| 2400×1080 着色器负载 | GPU 41.9 fps vs llvmpipe 3.7 fps | 174 |
| Adreno/KGSL | 无真机证据，UNKNOWN | 233、243 |

## 5. 硬件

| 项目 | 结果 | 证据 |
|---|---|---|
| 后摄 `/dev/video0` | 5 帧，6,912,000 bytes，11.89 fps | 309 |
| 前摄 `/dev/video1` | 5 帧，6,912,000 bytes，12.19 fps | 309 |
| Camera2 V4L2 `read` / `readv` | 前后摄通过 | 309 |
| GStreamer `v4l2src` | 前后摄各 3 帧 NV12 640×480，rc=0 | 309 |
| A-hw 普通用户授权路径 | `PASS=11 FAIL=0`；前后摄各 3 帧 1280×720 NV12 | 320 |
| V4L2 fixture + GStreamer | 640×480、1280×720 两档均 PASS | 320 |
| V4L2 crop/selection + monotonic PTS | 两档契约通过；GStreamer 无 `CROPCAP`/时间戳相关性警告 | 321 |
| AAudio 采集 | 196,608 PCM bytes，`peak=285` | 310 |
| 传感器 IIO | 5 类默认传感器 `READY`，guest 可读 | 302 |
| 传感器 D-Bus | `iio-sensor-proxy` 出现；Accelerometer/Light/Proximity 三项 Claim 成功；`monitor-sensor` 读到三类设备 | 322 |
| MediaCodec 解码 | H.264 1080p30，600 秒，dropped=0 | 80 |
| AHB → SurfaceControl | 3-buffer ring / SYNC_FD，10,000 帧 | 59 |

## 6. 宿主显示边界

Husk 运行资产不包含 X server。最终树没有 `Xorg` / `Xvfb`，也没有自建 X11 socket；独立启动
GNOME/X11 桌面为 **UNVERIFIED**。不得把其他 App 或旧 PRoot 树的 X socket 作为 Husk 结果。

详情：[evidence 312](design/evidence/312-x11-host-boundary-20261127.md)。

## 7. UNKNOWN

- 16 KiB / 64 KiB 真机运行；
- Adreno/KGSL 真机；
- 其他 Android 版本、厂商 seccomp/SELinux；
- 相机 DMABUF、通用 V4L2 controls、非整帧裁剪、真实 Camera2 的 GStreamer PTS、跨进程假 fd；
- 定位、USB、蓝牙完整链路；
- 窗口内容到 X server 的零拷贝；
- 负载型长时间稳定性和更多 GPU 应用。
