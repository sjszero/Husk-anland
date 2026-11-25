# 231 · 音频桥运行验证：并发入口、播放 DATA 与采集 READ

日期：2026-11-26 00:37–00:38（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页。
测试对象是 Husk 最小包 `com.linuxhub.husk`；没有改 2.0 包源码或数据。

## 1 · 结论

| 面 | 结果 | 证据等级 |
|---|---|---|
| 一个 idle 客户端是否还会堵死监听 | 播放口和采集口都 PASS | FACT，真机 |
| 播放协议 `OPEN → DATA → CLOSE` | PASS，48000/48000 帧写入 | FACT，真机 |
| 采集协议 `OPEN → READ → CLOSE` | PASS，49152 帧 / 196608 字节 | FACT，真机 |
| 实际扬声器听感、麦克风声学质量 | 未做人工听音/声压测量 | UNKNOWN |

所以 H-19 的并发修复已经从「代码已改、运行未验证」升级为真机运行已验证。音频链路的协议和
AAudio 交接通过，不等于完整桌面音频产品验收；延迟和 xrun 的数字照下面原样保留。

## 2 · 并发入口

探针先让客户端 A 连上但不发命令，再让客户端 B 发 `PING`。修复前 A 会占住同步 accept 循环，
B 只能等超时；现在两个监听口都立即回答：

```text
PASS  /run/husk/audio-out.sock: second client answered while the first was idle
PASS  /run/husk/audio-in.sock: second client answered while the first was idle
audio-bridge: PASS=2 FAIL=0
```

采集侧原来没有读超时，而且注释里的探针只发 `PING`，但 `handle_capture()` 当时只认
`OPEN/READ/CLOSE`，于是健康监听器也会回 `ERROR unknown-command`。本次给采集口补了
`PING → PONG bridge=audio-capture-v1`，让并发判据测的是 accept 是否被堵，而不是协议缺命令。

## 3 · 播放：真实 OPEN/DATA

guest 侧 `audio-probe` 连接 `/run/husk/audio-out.sock`，先枚举设备，再发送 1 秒 48 kHz、
单声道、440 Hz 的 PCM16。关键输出：

```text
PONG bridge=audio-v1
DEVICES_BEGIN count=7
DEVICES_END
VOLUME OK index=3 max=16
VOLUME OK index=5 max=16 percent=30
PCM bytes=96000 frames=48000 rate=48000 channels=1 hz=440 amplitude=0.18
STREAM_READY rate=48000 channels=1 format=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1
DONE status=PASS input_bytes=96000 frames=48000 requested_frames=48000 frames_written=48000 rate=48000 channels=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1 latency_ms_avg=121.667 latency_ms_max=128.000 xruns=1 elapsed_ms=2221
RESULT PASS
```

FACT：AAudio 打开成功，48000 帧全部写入；本轮有 1 次 xrun，平均延迟 121.667 ms，最大 128.000 ms。
这组数字比历史记录里的约 105 ms 差，按要求原样记录，不挑好看的数字。UNKNOWN：没有人在设备旁
确认听到 440 Hz，也没有测声压或扬声器输出功率。

## 4 · 采集：真实 OPEN/READ

新增 `tools/probe/audio-capture-probe.py`，它检查协议和字节数，也不把“房间很安静”误判成失败。
1 秒运行结果：

```text
PONG bridge=audio-capture-v1
CAPTURE_READY rate=48000 channels=2 format=1 device=16 burst=128 capacity=4096
PCM_OK frames=1024 bytes=4096
...（共 48 块）
CAPTURE_OK frames_read=49152 silent_chunks=3 peak=255
capture-probe|PASS|chunks=48|frames=49152|bytes=196608|silent_chunks=3|peak=255|elapsed_ms=1059
```

FACT：采集流打开成功，48 个 `PCM_OK` 块、49152 帧、196608 字节全部收回，`CAPTURE_OK`；峰值 255，
48 块中有 3 块全零。UNKNOWN：峰值非零不等于语音可懂，本轮没有回放录音、没有测信噪比，也没有
验证 GStreamer/桌面录音软件。

## 5 · 复现入口

- 并发入口：`tools/probe/audio-bridge-concurrency.py`
- 采集协议：`tools/probe/audio-capture-probe.py`
- 播放协议：设备 rootfs 里的 `usr/bin/audio-probe /run/husk/audio-out.sock 1`
