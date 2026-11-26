# 321 · V4L2 crop/selection 契约与单调时间戳修正（2026-11-27）

设备：`UKHMMB8PYTGECMNV` / RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / `untrusted_app`。

活动树：`files/rootfs-full -> release-tree-libc812-20261127/root`。

## 0 · 结论

- **FACT**：修前 GStreamer 1.24.2 两条 `VIDIOC_CROPCAP failed`、一条
  `Timestamp does not correlate with any clock`；修后同一 fixture 两档都不再出现这三条警告。
- **FACT**：修前 `v4l2-test` 在 `CROPCAP` 得到 `ENOTTY(25)`。修后
  `CROPCAP`、`G_SELECTION` 的 bounds/default/current、`G_CROP` 与完整画幅
  `S_CROP` 全部通过。
- **FACT**：修前 DQBUF 时间戳把纳秒直接拆成微秒字段，`tv_sec` 放大 1000 倍。
  修后时间按 `CLOCK_MONOTONIC` 输出；640×480 样例
  `ts=278371.190533` 到 `278371.296968`，1280×720 样例
  `ts=278372.609692` 到 `278372.808910`。
- **FACT**：修后 640×480 与 1280×720 两档 raw + GStreamer 均通过；GStreamer 各 3 帧，
  两档都是 `GST_AUDIT|PASS`。
- **FACT**：`Uncertain or not enough buffers, enabling copy threshold` 仍会出现。它来自
  GStreamer 的固定策略：下游没有 allocation pool 时主动打开 copy threshold；不是 REQBUFS
  数量不对。本次 facade 对请求 4 个 MMAP buffer 返回 4 个。
- **UNVERIFIED**：本轮用确定性 fixture 验证 crop/selection 与时间戳归一化，没有重新跑真实
  Camera2 传感器的 GStreamer PTS。真实前后摄取帧仍以 evidence 309/320 为独立证据。
- **UNVERIFIED**：任意非完整画幅的硬件/软件裁剪没有实现；`S_CROP` / `S_SELECTION` 对非整帧
  矩形明确返回 `EINVAL`，不假装成功。

## 1 · 修前负向结果

旧 loader：`4216ad0cd98f41e8f7b8f366658fe80b9231d60d49a94f90382cf26f6ae8d768`。

GStreamer fixture：

```text
WARN gstv4l2object.c:4806 VIDIOC_CROPCAP failed
WARN gstv4l2object.c:4806 VIDIOC_CROPCAP failed
WARN gstv4l2bufferpool.c:852 Uncertain or not enough buffers, enabling copy threshold
WARN gstv4l2src.c:1260 Timestamp does not correlate with any clock, ignoring driver timestamps
GST_AUDIT|EOS|640x480
GST_AUDIT|PASS|640x480|frames=3
```

新增契约测试在旧 loader 上直接失败：

```text
v4l2test|setfmt|640x480|3231564e|sizeimage=460800
v4l2test|FAIL|CROPCAP: Inappropriate ioctl for device (os error 25)
```

## 2 · 运行时修改

新 loader：`d621d2678092863104a79dec2e5b9806153041da8351972006dd7d23cfe60a84`。

- `runtime/crates/v4l2-shim/src/lib.rs` 新增 `CROPCAP`、`G_CROP`、`S_CROP`、
  `G_SELECTION`、`S_SELECTION`。capture facade 的 bounds/default/current 都是当前协商整帧；
  `S_CROP` / `S_SELECTION` 只接受同一整帧矩形。
- DQBUF 现在把相机时间戳在 `CLOCK_MONOTONIC`、`CLOCK_BOOTTIME`、`CLOCK_REALTIME` 之间做相关性
  判断，再统一换算到 V4L2 的 monotonic 时基；没有可信时间戳时使用帧到达时刻。
- 修正纳秒到 `timeval` 的单位：`tv_sec = ns / 1_000_000_000`，
  `tv_usec = (ns % 1_000_000_000) / 1_000`。旧代码按微秒拆分，导致秒字段大 1000 倍。
- `runtime/crates/v4l2-test` 新增负向契约：缺 crop/selection 或时间戳不在 monotonic 10 秒窗口
  内直接失败。`tools/device-tests/run-camera-fixture.sh` 令 GStreamer 的 `CROPCAP` / 时间戳警告
  回归时直接失败。

## 3 · 修后正向结果

```text
v4l2test|setfmt|640x480|3231564e|sizeimage=460800
v4l2test|crop|bounds=yes|default=yes|selection=yes|full-crop=yes
v4l2test|frame|0|idx=0|bytes=460800|seq=0|ts=278371.190533
v4l2test|frame|9|idx=0|bytes=460800|seq=9|ts=278371.296968
v4l2test|done|frames=10|bytes=4608000|fps=79.62
v4l2test|OK

v4l2test|setfmt|1280x720|3231564e|sizeimage=1382400
v4l2test|crop|bounds=yes|default=yes|selection=yes|full-crop=yes
v4l2test|frame|0|idx=0|bytes=1382400|seq=0|ts=278372.609692
v4l2test|frame|9|idx=0|bytes=1382400|seq=9|ts=278372.808910
v4l2test|done|frames=10|bytes=13824000|fps=39.93
v4l2test|OK

GST_AUDIT|pipeline|640x480|frames=3
GST_AUDIT|EOS|640x480
GST_AUDIT|PASS|640x480|frames=3

GST_AUDIT|pipeline|1280x720|frames=3
GST_AUDIT|EOS|1280x720
GST_AUDIT|PASS|1280x720|frames=3

camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

原始日志：

```text
/tmp/husk-v4l2-crop-ts-regate-20261127.log
sha256: 8e90375a6f2c05f5f5f738f19465d317141aa487c1de89d733a519f5dda9e2c7
```

## 4 · 剩余 copy-threshold 警告的来源

GStreamer 1.24.2 中：

- `gstv4l2object.c` 在 downstream allocation query 没有 pool 时设置
  `gst_v4l2_buffer_pool_copy_at_threshold(..., TRUE)`；
- `gstv4l2bufferpool.c:851-855` 随后因为 `enable_copy_threshold` 为真打印该警告。

本次 pipeline 的 `fakesink` 不提供 downstream pool，所以这是 GStreamer 的预期回退策略。
它表示高压力时会复制 buffer，不代表 `REQBUFS` 少返回。把这条日志静音会掩盖真实策略，因此本轮
不把“警告消失”当作修复，只把它记录为后续呈现/缓冲池优化需要量化的成本。

## 5 · 门禁

本轮提交前执行：

```text
make check
make docs
```

结果见提交记录和 CI；这里不把代码检查等同于真机运行结论。

本机结果：

```text
make check: PASS
  cargo fmt --all --check: PASS
  cargo check --workspace (aarch64-unknown-linux-gnu): PASS
  cargo clippy --workspace (aarch64-unknown-linux-gnu): PASS
  shell syntax: 104 script(s) PASS
  release offline audit: PASS (version=0.0.36)
  session/GPU/installer/release contract checks: PASS
make docs: PASS (348 Markdown, 0 broken links)
```
