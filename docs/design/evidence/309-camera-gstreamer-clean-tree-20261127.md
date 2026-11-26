# 309 · 相机 read/readv/mmap 与 GStreamer 复验（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191
活动 App：`com.linuxhub.husk`
测试树：`files/rootfs-full -> files/release-tree-clean2/root`

## 0 · 结论

- **FACT**：Camera2 NDK 的真实前后摄节点通过 App 提供的 V4L2 facade 可用。
  后摄 `/dev/video0`、前摄 `/dev/video1` 均能完成 `mmap + DQBUF`、`read` 和 `readv`。
- **FACT**：1280×720、每路 5 帧的结果为：
  - 后摄：`6,912,000 bytes`，`11.89 fps`
  - 前摄：`6,912,000 bytes`，`12.19 fps`
- **FACT**：640×480 流实测：
  - `read`：后摄 `9.46 fps`，前摄 `11.13 fps`
  - `readv`：后摄 `9.35 fps`，前摄 `11.59 fps`
- **FACT**：NV12 帧不是全黑，样例 `min=0 max=128 mean≈42.8`。
- **FACT**：GStreamer `v4l2src` 对前后摄各取 3 帧 NV12 640×480，`rc=0`。
- **UNVERIFIED**：本次 GStreamer 工具包只解包进测试树，不是从空目录安装的正式桌面包；
  最终发行树如果需要 GStreamer，必须把它列为明确的附加包，而不是把测试工具留在树里。

## 1 · mmap + DQBUF

```text
后摄 /dev/video0: 5 帧，6,912,000 bytes，11.89 fps，OK
前摄 /dev/video1: 5 帧，6,912,000 bytes，12.19 fps，OK
```

## 2 · read / readv

```text
后摄 read： 9.46 fps，OK
前摄 read：11.13 fps，OK
后摄 readv：9.35 fps，OK
前摄 readv：11.59 fps，OK
```

## 3 · GStreamer

干净树没有 `gst-launch-1.0`。只把测试包 `gstreamer1.0-tools` 解包进测试树后：

```text
video0: 3 帧 NV12 640×480，rc=0
video1: 3 帧 NV12 640×480，rc=0
```

这条结果证明 V4L2 接口可被标准 GStreamer 消费；它不证明发行 rootfs 默认包含 GStreamer。
