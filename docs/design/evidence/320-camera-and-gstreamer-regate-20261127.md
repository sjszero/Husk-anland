# 320 · 相机策略解除后的 A-hw、前后摄与 GStreamer 复验（2026-11-27）

设备：`UKHMMB8PYTGECMNV` / RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / `untrusted_app`。

活动树：

```text
files/rootfs-full -> release-tree-libc812-20261127/root
loader sha256:  4216ad0cd98f41e8f7b8f366658fe80b9231d60d49a94f90382cf26f6ae8d768
libc sha256:    2ed8771f220ba55ed0c224e114c83a30cb9d34fb333d375fc690f7267bd138bb
PanVK sha256:   599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c
```

## 0 · 结论

- **FACT**：`17-gate-a-hw.sh` 在当前正式树通过 `PASS=11 FAIL=0`。CAMERA 与
  RECORD_AUDIO 都是已授权状态，`USER_FIXED` 不存在；真实 Camera2 返回 3 帧。
- **FACT**：`20-tree-integrity.sh` 为 `PASS=23 FAIL=0`，`14-gate-a-acceptance.sh` 为
  `PASS=34 FAIL=0 SKIP=1`，`18-gpu-loader-defaults.sh` 为 `PASS=9 FAIL=0 SKIP=0`。
- **FACT**：后摄 `id=0` 与前摄 `id=1` 各通过 Camera2 获取 3 帧 1280×720 NV12，
  每帧 `1,382,400` bytes；不是只做设备枚举。
- **FACT**：V4L2 fixture 的 640×480 和 1280×720 两档都完成 `open/ioctl/mmap/DQBUF`；
  GStreamer 合成 fixture 两档各取 3 帧，均 `GST_AUDIT|PASS`。
- **FACT**：本次 `dumpsys media.camera` 不再出现 `Camera "0"/"1" disabled by policy`；
  evidence 307 记录的宿主策略阻塞在当前设备状态下已解除。
- **UNVERIFIED**：本轮 GStreamer 使用确定性 fixture，不是真实手机传感器；真实前后摄的
  GStreamer 结果仍以 evidence 309 为历史独立证据，不能把两条混成一次运行。
- **UNVERIFIED**：独立 Husk X11 桌面、相机 DMABUF、V4L2 controls、Adreno/KGSL 和
  16/64 KiB 真机仍不在本轮结论内。

## 1 · A-hw（普通用户授权路径）

```text
$ DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
    ROOTFS_REL=files/rootfs-full LOADER_REL=files/linux/libld.so \
    sh tools/device-tests/17-gate-a-hw.sh
PASS  CAMERA granted=true
PASS  CAMERA is not USER_FIXED
PASS  RECORD_AUDIO granted=true
PASS  RECORD_AUDIO is not USER_FIXED
PASS  app reports all hardware permissions granted
PASS  captured 3 frame(s) through Camera2 (1382400 bytes each)
PASS  app opened a Camera2 session
PASS  capability matrix readable
PASS  audio capture device present
PASS  microphone returned 196608 PCM bytes (peak=423)
PASS  no crash records for the app process
PASS=11 FAIL=0
```

本轮只复验当前安装的权限状态和硬件链路；没有重新执行一次“卸载后不带 `-g` 安装”的完整
流程，也没有运行时改写 CameraService 策略。普通用户首次安装/拒绝/重新授权的完整序列仍以
evidence 310 及 `17-gate-a-hw.sh` 的前置说明为准，本证据不把它冒充成本轮重新安装。

## 2 · 前后摄真实帧

在 App 重新启动、`files/rootfs-full/run/husk/camera.sock` 由 App bridge 重建后，分别请求
`id=0` 和 `id=1`：

```text
=== camera id=0 ===
camera-client|OPEN|OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=0
camera-client|FRAME|seq=1|...|1280x720|bytes=1382400|min=0|max=128|mean=42.86
camera-client|FRAME|seq=2|...|1280x720|bytes=1382400|min=0|max=128|mean=42.88
camera-client|FRAME|seq=3|...|1280x720|bytes=1382400|min=0|max=128|mean=42.89
camera-client|PASS|frames=3|elapsed_ms=357|bye=BYE|seqs=1,2,3

=== camera id=1 ===
camera-client|OPEN|OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=1
camera-client|FRAME|seq=1|...|1280x720|bytes=1382400|min=0|max=128|mean=42.85
camera-client|FRAME|seq=2|...|1280x720|bytes=1382400|min=0|max=128|mean=42.71
camera-client|FRAME|seq=3|...|1280x720|bytes=1382400|min=0|max=128|mean=42.71
camera-client|PASS|frames=3|elapsed_ms=324|bye=BYE|seqs=1,2,3
```

帧不是全黑，`min=0 / max=128 / mean≈42.7–42.9`。这里的 ID 是 Camera2 枚举返回的
guest 协议 ID；服务端回包中的 `id=0` / `id=1` 与请求一致。

## 3 · V4L2 与 GStreamer fixture

命令：

```sh
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
  ROOTFS=/data/user/0/com.linuxhub.husk/files/rootfs-full \
  LD=/data/user/0/com.linuxhub.husk/files/linux/libld.so \
  sh tools/device-tests/run-camera-fixture.sh UKHMMB8PYTGECMNV
```

关键输出：

```text
v4l2test|setfmt|640x480|3231564e|sizeimage=460800
v4l2test|reqbufs|4
v4l2test|done|frames=10|bytes=4608000|fps=89.34
FIXTURE|PASS|clients=1

v4l2test|setfmt|1280x720|3231564e|sizeimage=1382400
v4l2test|done|frames=10|bytes=13824000|fps=41.27
FIXTURE|PASS|clients=1

GST_AUDIT|PASS|640x480|frames=3
GST_AUDIT|PASS|1280x720|frames=3
camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

`VIDIOC_CROPCAP` 和“不确定 buffer 数量”的 GStreamer 警告仍存在；它们是当前 facade 的已知
协议缺口，不影响这两档取帧通过，不能写成“所有 V4L2 control 已支持”。

## 4 · 同轮门禁

```text
TREE_INTEGRITY PASS=23 FAIL=0
GATE_A PASS=34 FAIL=0 SKIP=1
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

Gate A 的唯一 SKIP 是离线树没有 apt package index，不是通过项。

## 5 · 边界

- 相机 DMABUF、V4L2 controls、跨进程假 fd：**UNVERIFIED**。
- GStreamer 真实手机传感器链路：历史上由 evidence 309 验证；本轮的 fixture 不能替代它。
- 独立 Husk X11 桌面：**UNVERIFIED**；本证据没有借用其他 App 的 X socket。
- Adreno/KGSL、16/64 KiB 页真机：**UNKNOWN / UNVERIFIED**。
