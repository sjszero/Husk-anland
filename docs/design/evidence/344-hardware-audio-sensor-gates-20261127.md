# 344 · 传感器、音频输出与测试隔离闭环

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：五类默认传感器都能通过 `/sys/class/iio`、`/sys/bus/iio/devices`、`/sys/subsystem/iio/devices` 三条标准路径读取。
- **FACT**：加速度计、磁力计、陀螺仪的 sysfs mtime 在 1 秒内持续前进；光照和距离是 Android on-change 传感器，不把“静止时没有新事件”误判成故障。
- **FACT**：A-hw 修后为 `PASS=15 FAIL=0`，新增五类传感器检查，原有 Camera2、麦克风、定位和崩溃检查保持通过。
- **FACT**：直接 AAudio 输出完成 7 个设备枚举、音量改写/恢复和 96,000 frames 写入。
- **FACT**：标准 guest `paplay` → PulseAudio → Rust `audio-relay` → App AAudio 全链路通过，87,035 frames / 348,140 bytes，两个进程退出码均为 0。
- **FACT**：`audio-setup`、`audio-relay`、`audio-capture`、`audio-probe` 已进入正式构建、发行包、安装器和 runtime-boundary 门禁。
- **UNVERIFIED**：没有独立人耳听音；AAudio 仍报告低延迟模式被厂商拒绝，实测平均队列延迟约 104.68–125.16 ms，不能宣称低延迟。
- **UNVERIFIED**：宿主 X11/GNOME、多 GUI 并发和 Surface 呈现仍不在本轮证明范围。

## 1 · 传感器现场结果

正式探针：`tools/device-tests/husk-sensor-probe.sh`

```text
SENSOR_PATH|/sys/class/iio/iio:device0|accel_3d
SENSOR_PATH|/sys/bus/iio/devices/iio:device0|accel_3d
SENSOR_PATH|/sys/subsystem/iio/devices/iio:device0|accel_3d
SENSOR_VALUE|accel_3d|10022
SENSOR_MTIME|accel_3d|2026-11-27 07:32:08.933010964 +0000|2026-11-27 07:32:09.969010964 +0000
SENSOR_VALUE|magn_3d|14475
SENSOR_MTIME|magn_3d|2026-11-27 07:32:10.521010964 +0000|2026-11-27 07:32:11.521010964 +0000
SENSOR_VALUE|gyro_3d|1
SENSOR_MTIME|gyro_3d|2026-11-27 07:32:12.013010964 +0000|2026-11-27 07:32:13.045010964 +0000
SENSOR_VALUE|als|0
SENSOR_MTIME|als|on-change
SENSOR_VALUE|proximity|5
SENSOR_MTIME|proximity|on-change
SENSOR_AUDIT|PASS|classes=5|paths=3|stream=live
```

说明：临时探针最初把单轴光照/距离当成三轴读取，并错误要求 on-change 类持续刷新；这是测试脚本问题，已修正。最终门禁只要求连续类保持更新。

## 2 · H31：相机 fixture 污染后续硬件门禁

### 修前负向结果

先运行 `make camera-audit`，随后 A-hw：

```text
PASS=13 FAIL=2
FAIL capture failed:
FAIL no Camera2 session was opened
```

现场：

```text
camera-client|FAIL|connect rootfs-full/run/husk/camera.sock: No such file or directory
```

根因：raw fixture 会 unlink App 正在监听的 `camera.sock`。Unix listener 继续挂在已删除 inode 上，App 不会自动重绑，因此后续 Camera2 测试拿到死路径。

### 修复

`tools/device-tests/run-camera-fixture.sh` 在 EXIT/HUP/INT/TERM 上强制重启 App，并等待新的 `camera.sock`：

```text
srwx------ ... camera.sock
```

修后相机 fixture 和 A-hw 均正常。A-hw：

```text
PASS=15 FAIL=0
PASS captured 3 frame(s) through Camera2 (1382400 bytes each)
PASS microphone returned 196608 PCM bytes (peak=223)
PASS gpsd delivered a last-known fix
```

## 3 · H32：音频 Rust 工具没有进入发行资产

### 修前事实

源码中存在：

- `runtime/crates/audio-setup`
- `runtime/crates/audio-relay`（含 `audio-capture`）
- `runtime/crates/audio-probe`

但修前：

- `build-guest-tools.sh` 不构建这四个二进制；
- `make dist` 不复制、不打包；
- `install-husk.sh` 不安装；
- 新 rootfs 中没有 `audio-setup` / `audio-relay` / `audio-capture` / `audio-probe`；
- `audio-out.sock` 因没有标准配置入口而处于 marker 等待状态。

这证明旧的音频 PASS 依赖手工拷贝，不能作为新安装版本的可交付能力。

### 修复

- 构建：四个二进制进入 `build-guest-tools.sh` 的正式构建与 ELF 入口检查。
- 发行：四个二进制进入 `dist/husk-arm64.tar.gz`。
- 安装：`install-husk.sh` 复制到 guest `/usr/bin`。
- 门禁：runtime boundary 必须包含这四个文件，缺失即失败。

本轮发行包：

```text
423d39848edaac3f316447089a7cc79499fee312f8d2b1fdeea3390e7fc413dc  dist/husk-arm64.tar.gz
```

二进制：

```text
3b37b64af72afa54d5140d0ae49c5c3272ea423bc5be478c96d4cfc2158d2564  audio-setup
0f91d219ae5bdf13ce9045dbf7aa97c815e1d392de3da7cf8eb5979449ee4c44  audio-relay
9a8945d6c0b51d9bd0361352b1e6a7ecad9368a5514eb16eb4bd3befaa318387  audio-capture
d9713b8c085e70366891e0a15abb515667d061ef4662d99df3e261ac6a45429d  audio-probe
```

## 4 · 音频设备门禁

命令：

```sh
DEV=UKHMMB8PYTGECMNV make audio-audit
```

结果：

```text
AUDIO_AUDIT PASS=4 FAIL=0
```

### 直接 AAudio

```text
DEVICES_BEGIN count=7
DONE status=PASS input_bytes=192000 frames=96000 requested_frames=96000
 frames_written=96000 rate=48000 channels=1 device=3
 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1
 latency_ms_avg=125.163 latency_ms_max=128.000 xruns=1
RESULT PASS
```

这覆盖设备枚举、`STREAM_MUSIC` 音量改写/恢复和真实 PCM 写入。

### 标准 `paplay`

```text
AUDIO_PLAYER_RC|paplay=0|relay=0
relay|OPEN|OPEN_READY rate=48000 channels=2 format=1 device=3
 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1
relay|CLOSE|CLOSE_OK input_bytes=348140 frames=87035
 frames_written=87035 latency_ms_avg=104.680 latency_ms_max=106.667 xruns=1
relay|PASS|chunks=43|frames=87035
RESULT PASS
```

平台仍返回 `perf=10 (NONE)`，并出现 1 次 xrun。本轮不把“有声音”扩大成低延迟或无人耳验证的结论。

## 5 · H33：能力矩阵仍写传感器 D-Bus 未验证

`app-rs/src/caps.rs` 修前：

```text
iio-sensor-proxy D-Bus remains UNVERIFIED
```

但 evidence 322 已记录：

```text
ClaimAccelerometer() -> ()
ClaimLight()          -> ()
ClaimProximity()      -> ()
monitor-sensor: Has accelerometer / ambient light / proximity
```

因此这是文档事实漂移，不是功能失败。已改为五类 IIO 三路径、A-hw 连续流检查与 evidence 302/322/344 的 D-Bus 验证。

## 6 · 当前边界

- 传感器仍是 NDK → IIO 影子树的中转，不是 guest 直接读宿主 IIO 节点。
- 音频仍是 PulseAudio → Rust relay → AAudio 的中转；`/dev/snd` 在应用域不可用。
- 本轮不证明宿主 X11/GNOME 桌面链路。
- 本轮不升版，不 tag，不创建 Release。
