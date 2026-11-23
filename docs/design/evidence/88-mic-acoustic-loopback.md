# Evidence 88 — 麦克风打通：声学回环证明真机输入，不是静音占位

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
域：`untrusted_app`（`com.linuxhub.husk`）／rootfs `files/rootfs-full`

## 结论

**FACT：手机麦克风已经能进 guest，且是硬件路径。** 用"guest 放 440 Hz 正弦 → 扬声器 →
空气 → 手机麦克风 → guest 录音"的声学回环验证：440 Hz 能量只在放音窗口出现，
放音前后为 0，二次谐波可忽略。

```text
sec 0 peak=  264 |440Hz|=     0.0     # 放音前
sec 1 peak=   55 |440Hz|=     0.2     # 放音前
sec 2 peak= 3186 |440Hz|=   101.8     # 放音开始
sec 3 peak= 3045 |440Hz|=   413.3
sec 4 peak= 1043 |440Hz|=   467.1
sec 5 peak= 1030 |440Hz|=   467.7
sec 6 peak= 1044 |440Hz|=   467.2
sec 7 peak= 3737 |440Hz|=   466.4
sec 8 peak=  221 |440Hz|=     0.3     # 放音结束
sec 9 peak=   68 |440Hz|=     0.3
```

全程链路：

```text
gst audiotestsrc(440Hz) → PulseAudio module-pipe-sink → audio-relay(guest, Rust)
  → AAudio 输出(app) → 扬声器 → 空气 → 手机麦克风
  → AAudio 输入(app, GENERIC preset) → audio-capture(guest, Rust)
  → PulseAudio module-pipe-source → parec 落盘 1,921,024 字节
```

## 三个必须先修掉的坑（都是实测定位）

### 1. `module-pipe-source` 不接受已存在的 FIFO（`module-pipe-sink` 接受）

PA 16.1 `src/modules/module-pipe-source.c:241` 直接 `mkfifo()`，`EEXIST` 即
`initialization failed`；`module-pipe-sink.c:461` 显式忽略了 `EEXIST`。
所以 `audio-setup` 之前"预建 mic.pcm"是错的：FIFO 必须由模块自己建，
我们只能在每次启动前把上一轮遗留的清掉（`audio-setup --enable-capture` 现在做这件事，
并打印 `audio-setup|UNCLEAN|<path>`）。

### 2. `AAUDIO_INPUT_PRESET_UNPROCESSED` 在这台机器上是全 0 静音

open 成功、`CAPTURE_READY` 正常、`AAudioStream_read` 正常返回帧数，但样本全 0
（`peak=0`、`silent_chunks` 与 chunk 数相等）。换成 `GENERIC`（等价于
`MediaRecorder.AudioSource.MIC`，也就是 prior art 用的那个源）后立刻有真实电平。

```text
preset=9 (UNPROCESSED) → peak=0        # 假成功
preset=1 (GENERIC)     → peak=640      # 环境噪声，97.6% 非零样本，RMS 116
```

`UNPROCESSED` 现在只是 open 失败时的兜底，不再优先。

### 3. 息屏/锁屏时麦克风返回静音

`mWakefulness=Asleep`、`mCurrentFocus=NotificationShade` 的两次运行里，
AAudio 输入照样 open 成功，读出来的全是 0；亮屏解锁并让 Husk 在前台后立刻正常。
这与 prior art（DroidNative `docs/device-passthrough.md`）写的
"后台只有静音"一致，是平台行为，不是我们的 bug。

## 权限：这台 ROM 上只有一条路能自动化

```text
adb shell pm grant com.linuxhub.husk android.permission.RECORD_AUDIO
  → SecurityException: Neither user 2000 nor current process has GRANT_RUNTIME_PERMISSIONS
adb shell cmd appops set com.linuxhub.husk RECORD_AUDIO allow
  → uid 2000 does not have MANAGE_APP_OPS_MODES
adb install -r -g app-debug.apk
  → Success；dumpsys package: RECORD_AUDIO: granted=true, flags=[USER_SENSITIVE_*]
adb shell dumpsys appops --package com.linuxhub.husk | grep RECORD_AUDIO
  → RECORD_AUDIO: mode=foreground / RECORD_AUDIO (allow)
```

`-g` 走的是安装期授予，不需要 shell 自己持有 `GRANT_RUNTIME_PERMISSIONS`。
这条只用于我们自己的开发机循环；**产品路径必须是在 native 生命周期回调里
（主线程）调 `Activity.requestPermissions`**，见 `docs/design/audio-input-prior-art.md` §4。

## 另一个实测到的结构性 bug：guest 打不开 rootfs 之外的 socket

`audio-relay` 连 `files/audio-bridge.sock`（rootfs 外）报
`No such file or directory`，而同一路径在 `run-as sh` 里 `ls` 得到的是
`<rootfs>/data/data/.../audio-bridge.sock`（不存在）。guest 的路径解析把
绝对路径搬到 rootfs 里，所以 **app 侧的 socket 必须建在 rootfs 内**。

已修：`serve()` 现在同时监听 `files/audio-bridge.sock`（兼容旧脚本）和
`rootfs-full/run/husk/audio-out.sock`（guest 可达），采集侧本来就在
`rootfs-full/run/husk/audio-in.sock`。relay 用后者后一次通过。


## 按需开关麦克风（借用 prior art 的 `mic.active` 语义）

DroidNative 用 `pactl subscribe | while read` 写 `mic.active`，app 只在有录音流时开
Android 麦克风。Husk 复刻同一语义但不用 shell：`audio-capture ... gate=1 pactl=/usr/bin/pactl`
每 300 ms 调一次 `pactl list short source-outputs`，只统计挂在我们 `husk_mic`（不是
`husk_pipe.monitor`）上的流；没有就发 `CLOSE` 并停手，有就重新 `OPEN`。

实测（`88-mic-gate.log`，一次运行内先空闲、再录 5 秒、再空闲）：

```text
capture|GATE|idle|no recorder on husk_mic          # 前 4 秒：app 侧没有开流
capture|OPEN|CAPTURE_READY rate=48000 ...          # parec 开始后
capture|PROGRESS|frames=106496|...|peak=47
capture|GATE|close|CAPTURE_OK frames_read=121856 ... # parec 结束后
capture|GATE|idle|no recorder on husk_mic
```

app 侧同一时间线（`88-mic-gate-app.log`）：`audio|capture|open|preset=1...` 只在
parec 运行期间出现，`audio|capture|close|frames_read=121856` 在 parec 结束后立刻关闭流。
**麦克风不再常开。**

## 复现

```sh
# app 侧（必须已授予 RECORD_AUDIO）
adb -s UKHMMB8PYTGECMNV install -r -g app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV shell 'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard;
  am start -n com.linuxhub.husk/android.app.NativeActivity'
# guest 侧（脚本见文件头，已在设备上 files/mic-loop.sh）
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "sh files/mic-loop.sh > files/loop.out 2>&1 &"'
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/loop.out'
```

## 证据文件

- `88-mic-loop-run.log` — 一次完整运行（PULSE/TONE/capture/relay/RAW 字节数）。
- `88-mic-loop-capture.log` — `audio-capture` 的 PROGRESS 行（peak=3737）。
- `88-mic-loop-relay.log` — 输出侧 relay 同时在工作（OPEN/PROGRESS）。
- `88-mic-loop-setup.log` — `audio-setup --enable-capture`。
- `88-mic-loop-app.log` — app 侧 `audio|capture|open|preset=1...` 日志。

## 边界（FACT）

- 只验证了 48 kHz / 2ch / s16le；单声道、其他采样率没测。
- 只验证了前台可见状态；息屏/后台是平台静音，没有绕过。
- 没有做长时间连续录音（10 分钟级）与并发（边放边录超过 16 s）的稳定性统计。
- 没有 `VOICE_RECOGNITION`/`CAMCORDER`/回声消除等源选择。
- 输入设备固定为 device=16（默认输入），没有做设备枚举与切换。
