# Evidence 62 — PulseAudio → Rust relay → AAudio continuous playback

Device: RMX3357 / MediaTek MT6893 / Android 13 / kernel 4.19.191 / arm64
Time: 2026-11-23 06:05:28–06:05:38 CST
Conclusion: **PASS for a 10-second continuous session and standard guest `paplay`**.
This is not the ten-minute soak and not an AV-sync measurement.

## 1. Implementation

The app bridge now supports a persistent AAudio session:

```text
OPEN rate=48000 channels=2
OPEN_READY rate=48000 channels=2 format=1 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1
DATA frames=N
<N * channels * 2 bytes PCM16LE>
DATA_OK frames=N input_bytes=... frames_written=... xruns=...
CLOSE
CLOSE_OK input_bytes=... frames=... frames_written=... ... xruns=... elapsed_ms=...
```

The new pure Rust `runtime/crates/audio-relay` opens PulseAudio's `module-pipe-sink` FIFO and
forwards complete PCM frames over the dedicated `files/audio-bridge.sock`. It does not reuse input,
clipboard or video channels.

The app also calls `AAudioStream_setBufferSizeInFrames(burst)` before starting the stream. On this
device it changed the actual buffer from 2052 frames to 1026 frames (one burst). The platform still
returned `perf=10 (NONE)`; low-latency mode was not granted.

## 2. Standard player path

Guest-side setup:

- PulseAudio 16.1 from the Ubuntu rootfs;
- minimal config with `module-native-protocol-unix` and `module-pipe-sink` (`s16le`, 48 kHz, stereo);
- `pactl info` reported `Default Sink: husk_pipe`, server protocol 35;
- `paplay` played `/usr/share/sounds/freedesktop/stereo/audio-test-signal.oga` and exited 0.

Relay output:

```text
relay|OPEN|OPEN_READY rate=48000 channels=2 format=1 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1
relay|BEGIN|fifo=/data/user/0/com.linuxhub.husk/files/rootfs-full/tmp/husk-pulse/audio.pcm|rate=48000|channels=2|seconds=10
relay|CLOSE|CLOSE_OK input_bytes=348140 frames=87035 frames_written=87035 rate=48000 channels=2 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1 latency_ms_avg=104.680 latency_ms_max=106.667 xruns=1 elapsed_ms=10003
relay|PASS|chunks=43|frames=87035|elapsed_ms=10014|last_ack=DATA_OK frames=1019 input_bytes=348140 frames_written=87035 xruns=0
RESULT PASS
```

App log:

```text
audio|client|connected|fd=3
audio|stream|open|rc=0|rate=48000|channels=2|format=1|device=3|burst=1026|capacity=2052|buffer=1026|perf=10|sharing=1|xruns=0|set_buffer=1026
audio|session|close|input_bytes=348140 frames=87035 frames_written=87035 rate=48000 channels=2 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1 latency_ms_avg=104.680 latency_ms_max=106.667 xruns=1 elapsed_ms=10003
audio|client|disconnected
```

PulseAudio warnings were non-fatal: `/proc/self/exe` does not point at pulseaudio, `/dev/shm` is
absent, and D-Bus is unavailable. The server still answered pactl and carried the standard playback.

## 3. What this changes

This closes the implementation gap that the previous evidence 60 left open:

- before: guest probe sent a finite baked-in sine wave;
- now: standard guest `paplay` → PulseAudio mixer → `module-pipe-sink` FIFO → pure Rust relay →
  persistent AAudio session in the app process.

The measured queue-depth latency is lower than evidence 60 because the buffer was reduced from 2052
to 1026 frames: 104.680 ms average / 106.667 ms max in this run. It is still not end-to-end speaker
latency and it is still `perf=NONE`.

## 4. Artifacts

```text
audio-relay sha256 : a4897e92c997c75b1d66e8338f4ca7a2047582814d87a54758ffa2bde0640769
app APK sha256     : 22328c8768f1225f66ae536bc0e4936a7258e6f86c260cc5e16444e14b12e0d0
libhuskapp sha256  : 0675b0b0e2f3f45ca9eb8bddd15133a683ed1f5d0a00c7ddbd9739fe58b183de
embedded libld     : 92000b676169f39509d8cb53b9e20b6cf4cc0935477f0d1d5a5bf4601380a6fc
guest log sha256   : 3e8e38b2a2eae6b14b8a9fea6c0258883dee1cbef06c6484696c66519ffa5f55
app log sha256     : 550fa9b1b27f93de2a69bd87c01731240346e4efa8c6bf9e8d65760ded5c0a41
```

- `docs/design/evidence/62-audio-pulse-aaudio-continuous-guest.log`
- `docs/design/evidence/62-audio-pulse-aaudio-continuous-app.log`

## 5. Boundaries

- FACT: standard `paplay` exited 0 and the relay wrote 87,035 frames (348,140 bytes) to AAudio.
- FACT: one xrun was reported by the final AAudio statistics; the last per-chunk ack still showed 0.
- UNKNOWN: no independent human-ear check; evidence is the platform stream and volume events.
- NOT DONE: ten-minute soak with fd/RSS curves, GStreamer / desktop player, AV sync with video.
- NOT DONE: low-latency claim. The vendor policy still rejects the requested low-latency mode.
- NOT DONE: product provisioning remains a manual in-guest setup and must move into Rust tooling.
