# Audio input (microphone) prior-art audit

Date: 2026-11-23
Scope: phone microphone → guest Linux (PulseAudio source). Written **before** the Husk
implementation is treated as final, per the mandatory gate in `references.md`.

## 1. Sources actually pulled and read

| Source | Revision | What was read |
|---|---|---|
| DroidNative/DroidNativeApp | `fa591010d6e33adbf5a79178b874fd4abdc58931` | `devices/MicBridge.kt`, `docs/device-passthrough.md`, `assets/guest/session.sh` (`mic_watch`), `env/Passthrough.kt` |
| termux/termux-api | `master` @ 2026-11-23 | `mic/apis/MicRecorderAPI.java` |
| LinuxHub 2.0 | `c5ffcf6a60856380c2f4e480665284835803f182` | `grep` for `RECORD_AUDIO` / `AudioRecord` / `pipe-source`: **no microphone support at all** |
| PulseAudio | `819f2c06ccdc105739e1c714de968f2af1d3e205` | `src/modules/module-pipe-source.c` |
| google/oboe + AAudio header | see `audio-prior-art.md` | input direction, callback vs. blocking read |

## 2. FACT — what the prior art actually does

### DroidNative (closest shape: same product, same "guest drives the device" model)

```text
guest session.sh:  pactl load-module module-pipe-source source_name=android_mic \
                     file=$RUN/mic.fifo format=s16le rate=48000 channels=1
                   pactl subscribe | while read line; do
                     case $line in *source-output*) update;; esac; done
                   update: writes 1/0 to $RUN/mic.active
app MicBridge.kt:  loop reads mic.active; only while it is "1" does it open
                   AudioRecord(MIC, 48000, MONO, PCM16), read 20 ms buffers and
                   write them into the FIFO. Thread priority URGENT_AUDIO.
```

- **FACT**: the Android microphone is opened **only while a guest stream is actually
  recording**. Gating is driven by `pactl subscribe` (source-output add/remove).
- **FACT**: format is s16le/48 kHz/**mono**; the guest source is made default.
- **FACT**: permission is requested by the app UI when the user flips the "microphone"
  switch (`Passthrough.MICROPHONE` → `RECORD_AUDIO`), and the device counts as
  passthrough only when `isEnabled && hasPermission`.
- **FACT (their docs)**: camera / microphone / location only work while a DroidNative
  window (or the main screen) is visible; in the background the microphone records
  silence. Their env service is `specialUse`, not a `microphone` foreground service.

### termux-api

- **FACT**: records to a **file** through `MediaRecorder` in a background `Service`,
  driven by intents (`termux-microphone-record -f out.m4a`). Not a stream, not a
  Linux device, and it needs a second app (Termux:API) with its own grant.

### LinuxHub 2.0

- **FACT**: no microphone path at all (no `RECORD_AUDIO`, no `AudioRecord`,
  no `module-pipe-source`). This is new ground for the 2.0 replacement, so 2.0 cannot
  be the reference here.

## 3. JUDGMENT — what Husk takes, and what it refuses

Take:

1. **`module-pipe-source` as the guest contract** (same rationale as the output path
   in `audio-prior-art.md`: do not invent a guest audio protocol). PulseAudio, ALSA's
   pulse plugin and Flatpak `--socket=pulseaudio` clients then all see the mic with no
   extra work.
2. **Demand gating**: the app must hold the Android input stream only while something
   in the guest records. Reimplemented *in Rust* by watching the PulseAudio source-output
   list from `audio-capture` instead of a shell `pactl subscribe | while read` loop.
3. **URGENT_AUDIO thread priority** for the capture loop, and an explicit "microphone
   unavailable" path that retries instead of dying.

Refuse:

1. **AudioRecord/Java**: the app is pure Rust; AAudio's input direction is the NDK
   equivalent and needs no JVM. (Cost: no `AudioSource` selection such as
   `VOICE_RECOGNITION`/`CAMCORDER`; AAudio uses the default input preset. Documented as
   a limitation, revisit only if the default preset misbehaves on this device.)
2. **MediaRecorder / file-based API** (termux-api): wrong shape for a live device.
3. **Mono-only**: keep 2 channels to match the output path and to avoid a resample;
   revisit if the HAL only reports mono.

## 4. FACT — runtime permission on this ROM (measured 2026-11-23)

This was the actual blocker, and the priors above do not solve it, because both of them
grant the permission from a **Java activity UI**.

| Attempt | Result |
|---|---|
| `adb shell pm grant com.linuxhub.husk android.permission.RECORD_AUDIO` | **FAIL**: `Neither user 2000 nor current process has GRANT_RUNTIME_PERMISSIONS` |
| `adb shell cmd appops set com.linuxhub.husk RECORD_AUDIO allow` | **FAIL**: `uid 2000 does not have MANAGE_APP_OPS_MODES` |
| `adb install -r -g app-debug.apk` | **PASS**: `RECORD_AUDIO: granted=true` in `dumpsys package` |

**JUDGMENT**: `-g` works because the install path grants runtime permissions with the
install flag, without the shell needing `GRANT_RUNTIME_PERMISSIONS`. That is fine for
our own device loop, but it is **not** a product answer: a real user installs the APK
normally.

Product path (to implement, pure Rust, no Java source):

- from the native `on_start`/`on_resume` callback (which runs on the Android main
  thread), if `checkSelfPermission(RECORD_AUDIO) != 0`, call
  `Activity.requestPermissions([RECORD_AUDIO], code)` through the existing `jni` crate;
- `requestPermissions` posts to the activity's own handler, so calling it off the main
  thread throws — the main-thread callback is the only safe place, which is why this
  belongs in the lifecycle code, not in the audio thread;
- the capture thread only ever *checks* the permission, never requests it.

## 5. Boundary of this design

- Mic while the app is in the background is expected to yield silence on Android
  11+ (prior-art claim above, not yet independently measured here — UNKNOWN).
- No `/dev/snd` capture device is presented to the guest (the node is EACCES to
  `untrusted_app`); the PulseAudio source is the only microphone interface.
- No `VOICE_RECOGNITION`/`CAMCORDER`/`UNPROCESSED` source selection yet.

## 6. 实测补充（evidence 88）

- **FACT**：`module-pipe-source` 的 `mkfifo()` 遇 `EEXIST` 直接 `initialization failed`
  （`module-pipe-source.c:241`）；`module-pipe-sink.c:461` 显式忽略 `EEXIST`。所以 FIFO 只能由模块创建，
  我们每次启动前清掉遗留项。
- **FACT**：`AAUDIO_INPUT_PRESET_UNPROCESSED`(9) 在本机 open 成功但样本全 0；`GENERIC`(1) 立刻有电平。
  与 prior art 选 `MediaRecorder.AudioSource.MIC` 一致。
- **FACT**：息屏/锁屏（`mWakefulness=Asleep`）时输入流 open 正常但全 0，与本文 §2 的
  "后台只有静音"相互印证。
- **FACT**：guest 里打不开 rootfs 之外的 socket，所以 app 侧音频 socket 必须建在 rootfs 内
  （`rootfs-full/run/husk/audio-out.sock` / `audio-in.sock`）。
- **NOT DONE**：按需求开关（本文 §3.2 的 demand gating）还没实现；当前只要 `audio-capture` 在跑，
  麦克风就一直开着。
