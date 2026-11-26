# V4L2、mmap 与 GStreamer 相机链路实测

日期：2026-11-26

## 目的

不依赖真实 Camera2 权限，验证 guest 侧 `/dev/video0` facade 能否完成
QUERYCAP、格式协商、REQBUFS、mmap、QBUF、DQBUF，并被 GStreamer 的
`v4l2src` 插件按同一套接口消费。

这不是“真实相机已通过”的证据。fixture 只复用 App 的
`camera.sock` 协议，验证 Linux 运行层和 V4L2 shim；真实 Camera2、
权限和传感器仍需回设备后验收。

## 环境

| 项目 | 值 |
|---|---|
| 设备 | RMX3357 |
| Android | 13 |
| 内核 | 4.19.191+ |
| ABI | arm64-v8a |
| rootfs | Ubuntu 24.04 / glibc 2.39 |
| loader | sha256 `02ca9ced7fcda2cfad5b6087a970eadba98e1eaada1954733794a8246e8e44ac` |
| loader interpose 数 | `206` |

## 结果

### 原始 V4L2 mmap

同一台设备、同一份 rootfs，分别用 640x480 和 1280x720 跑 10 帧：

| 分辨率 | 帧数 | 字节/帧 | 总计 | fps |
|---|---:|---:|---:|---:|
| 640x480 NV12 | 10 | 460800 | 4 608 000 | 97.25 |
| 1280x720 NV12 | 10 | 1382400 | 13 824 000 | 41.40 |

两条路径都完成 `streamon -> DQBUF -> streamoff`，输出
`v4l2test|OK`。640x480 的较低帧数不是失败；fixture 每次 `FRAME`
都生成并发送完整 NV12 帧，fixture 侧日志确认 10/10 帧送达。

### GStreamer v4l2src

没有使用 `gst-launch-1.0`。测试程序在 guest 内 `dlopen`
`libgstreamer-1.0.so.0`，运行：

```text
v4l2src device=/dev/video0 num-buffers=3
  ! video/x-raw,format=NV12,width=W,height=H
  ! fakesink sync=false
```

| 分辨率 | fixture 送达 | GStreamer 结果 |
|---|---:|---|
| 640x480 | 3/3 | `GST_AUDIT/EOS`、`PASS` |
| 1280x720 | 3/3 | `GST_AUDIT/EOS`、`PASS` |

`GST_DEBUG=2` 仍有以下非致命警告，保留原样：

```text
VIDIOC_CROPCAP failed
Uncertain or not enough buffers, enabling copy threshold
Timestamp does not correlate with any clock, ignoring driver timestamps
```

这些警告说明插件可工作，但不代表所有 GStreamer 版本、所有格式或真实
Camera2 时间戳行为都已验证。

### 实际命中的是核心 shim

用 `NR_V4L2_AUDIT=1` 记录一次性审计行，实际观察到：

```text
[husk v4l2] core open
[husk v4l2] core ioctl
[husk v4l2] core mmap
```

因此本机 GStreamer 路径是通过 loader 的 libc `open/ioctl/mmap` facades
进入 Rust V4L2 shim，而不是通过 `libv4l2.so` 内部的 `v4l2_*` GOT 槽。
loader 仍导出 `v4l2_open`、`v4l2_fd_open`、`v4l2_ioctl`、`v4l2_mmap`、
`v4l2_munmap`、`v4l2_close`、`v4l2_dup` 作为兼容接口；本次没有观察到
这些导出被 GStreamer 调用。`v4l2_read` 未实现，本次验证的是 mmap 模式；
read 模式仍是未验证边界。

## 代码边界

- NV12 的 DQBUF 在字节数不超过 mmap capacity 时直接读入 guest 的 mmap
  地址，去掉旧的 `scratch -> mmap` 第二次复制。
- YUYV 仍经过格式转换和必要复制。
- `v4l2-test` 新增第 5、6 个参数，可指定宽度和高度；默认仍为 640x480。
- fixture 的 `--gst` 模式使用 `NR_FIXTURE_SOCKET` 覆盖 socket 路径，默认
  guest 路径为 `/run/husk/camera.sock`。

## 未验证

- FACT：本次没有真实 Camera2 权限、真实传感器、真实 ISP/编码器。
- UNKNOWN：不同 GStreamer 版本、其他 Android 版本、16 KiB/64 KiB 页真机。
- UNKNOWN：`v4l2_read` 直读模式。
- UNKNOWN：并发多客户端和长时间相机流稳定性。

## 回归

本轮把最终 loader（上面的 sha256）显式传给测试入口，避免设备上正在使用的旧
loader 掩盖结果：

- `make check`：通过。
- `DEV=... LD=<本轮 loader> make gate-a`：门禁 A `PASS=38 FAIL=0`；
  coverage-probe、coverage-sweep、V4L2-ref、DRM、fd identity、
  supplementary groups、locale、identity-init race、groups race、session guard、
  rootfd 和 path race 全部 PASS。
- `DEV=... LD=<本轮 loader> make gpu-audit`：`PASS=3 FAIL=0`。现有 GNOME
  shell、默认 GL 客户端和嵌套 GTK 应用均观察到 zink/Mali 加速与设备 fd。
