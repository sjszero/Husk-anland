# 真实 Camera2 的 read、readv 与 mmap 实测

日期：2026-11-26

## 目的

验证 guest 侧 `/dev/videoN` 不仅能走 REQBUFS + mmap，也能按 Linux V4L2
标准走 `read()` / `readv()`；同时核对短缓冲、I/O 方式混用、非阻塞无帧和
QUERYCAP 能力位。测试使用真实 Camera2 后端，不是 camera fixture。

## 环境

| 项目 | 值 |
|---|---|
| 设备 | RMX3357 / RE5469 / MT6893 |
| Android / SDK | 13 / 33 |
| ABI | arm64-v8a |
| App | `com.linuxhub.husk`，`versionCode=35`，`versionName=0.0.35` |
| loader | sha256 `c7d8577ed233fb95a6280d50b3ce891378e5a0e1f9a08daa0bb891ec2c0fabc4` |
| loader interpose 数 | `208` |
| 相机 | Camera2 NDK，后摄 id=0，前摄 id=1 |

相机权限通过 `adb install -g -r` 授予。这个 ROM 拒绝普通 `pm grant` 和
`cmd appops set`；正式 App 的运行时权限请求 UI 仍未完成，不能用 `install -g`
冒充用户安装流程。

## 代码变化

- `v4l2-shim` 新增 `read_frame()` / `readv_frame()`：camera memfd 上的
  `read`、`readv` 进入相机协议，不再落到 memfd 的 EOF。
- `path-shim` 导出 `read` / `readv`；普通 fd 原样走 AArch64 `read` / `readv`
  syscall。
- `v4l2-preload` 同步导出带 GLIBC 版本标签的 `read` / `readv`。
- QUERYCAP 增加 `V4L2_CAP_READWRITE`；当前 caps 为
  `V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_READWRITE | V4L2_CAP_STREAMING`。
- `v4l2-test` 新增 `read`、`readv`、`contract`、`nonblock` 四种验收模式。

## 真实 Camera2 结果

### 帧时间戳

App 自身 Camera2 自检的 10 帧使用纳秒时间戳计算：

| 相机 | 帧数 | 每帧字节 | FPS | 时间跨度 |
|---|---:|---:|---:|---:|
| 后摄 id=0，1280x720 | 10 | 1 382 400 | 29.91 | 300.87 ms |
| 前摄 id=1，1280x720 | 10 | 1 382 400 | 29.92 | 300.82 ms |

FACT：`AImage_getTimestamp` 的单位是纳秒；旧代码按微秒换算会把约 30 fps
显示成约 0.03 fps，现已修正并补两根时间戳单元测试。

### guest 取流

每轮 5 帧，输出完整帧后由 `v4l2-test` 打印 `OK`：

| 模式 | 格式与分辨率 | 帧数 | 总字节 | FPS |
|---|---|---:|---:|---:|
| `read` | NV12 640x480 | 5 | 2 304 000 | 9.13 |
| `readv`（3 个 iovec） | NV12 640x480 | 5 | 2 304 000 | 9.10 |
| `readv`（3 个 iovec） | NV12 1280x720 | 5 | 6 912 000 | 9.24 |
| `read` | YUYV 640x480 | 5 | 3 072 000 | 8.20 |
| `readv`（3 个 iovec） | YUYV 640x480 | 5 | 3 072 000 | 8.96 |
| mmap + DQBUF | NV12 1280x720 | 5 | 6 912 000 | 11.67 |

FACT：NV12 直读、YUYV 转换、mmap 复用路径都返回完整帧长度。

### 标准语义

| 检查 | 结果 |
|---|---|
| QUERYCAP 声明 READWRITE + STREAMING | `caps=0x85000001`（含 DEVICE_CAPS），PASS |
| `read()` 缓冲小于一帧 | `EINVAL`，PASS |
| `read()` 与 REQBUFS 混用 | `EINVAL`，PASS |
| O_NONBLOCK + fixture 返回 NOFRAME | 0.26 ms 返回 `EAGAIN`，PASS |

## 黑帧归因

FACT：真实后摄导出的 NV12 帧长度正确，但 Y 平面接近全零：

```text
1280x720: Y_min=0 Y_max=2 Y_mean=0.3538 UV_mean=128.0
```

FACT：随后启动系统相机 `com.oplus.camera`，其预览同样全黑，并显示
“请在光线充足的环境下拍摄”。

INFERENCE：当前测试环境没有可用入射光，或镜头被遮挡；黑帧不是 Husk
转换函数单独造成的。

UNKNOWN：换到正常光照场景后的画面质量、白平衡、曝光收敛时间和前摄画质。

## 回归与未验证

- FACT：`make check` 通过，含全量 AArch64 workspace check、README/Release
  命名审计、版本 100 进位测试、shell 语法和文档链接检查。
- FACT：新 loader 上重新跑过 mmap 回归；640x480 为 11.88 fps，1280x720
  为 11.67 fps。
- UNVERIFIED：本轮没有完整发行 rootfs，因此没有重跑门禁 A 和 GStreamer
  `v4l2src`；上一轮 mmap/GStreamer 证据仍为 evidence 281。
- UNKNOWN：DMABUF、V4L2 controls、并发多客户端、长稳，以及 Mali 以外设备。
- FACT：正式 App 仍缺运行时 CAMERA / RECORD_AUDIO 权限请求 UI。

## 产物

| 文件 | sha256 |
|---|---|
| `rootfs-full/root/libld.so` | `c7d8577ed233fb95a6280d50b3ce891378e5a0e1f9a08daa0bb891ec2c0fabc4` |
| `rootfs-full/root/v4l2-test` | `d5ce7f498fdbb74a1441775651f0b750937f94870aa3eb68e3895c91f8a44645` |
