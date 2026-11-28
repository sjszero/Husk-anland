# Audio bridge

Status: **AAudio minimum + continuous PulseAudio→AAudio playback PASS, including a 10-minute
soak** (2026-11-23, evidence 60 / 62 / 63). AV sync and the final latency target are **not
complete**.

## 1. Why a bridge exists

Fact: in the `untrusted_app` domain on RMX3357, `/dev/snd` returns `EACCES`. The glibc guest
therefore cannot own the Android audio device directly. AAudio must be opened in the Android
application process, where the platform grants access to AudioFlinger and the vendor audio HAL.

The bridge is deliberately narrow:

```text
guest Rust probe / future PulseAudio relay
        |
        | dedicated AF_UNIX socket, raw PCM frames
        v
Husk app process (same UID)
        |
        | public AAudio C ABI loaded with dlopen("libaaudio.so")
        v
AudioFlinger -> vendor audio HAL -> speaker
```

The audio channel is separate from input, clipboard and video channels. No audio daemon,
VirGL-style relay process or `/dev/snd` emulation is introduced.

## 2. Components

| Component | Location | Responsibility |
|---|---|---|
| App bridge | `app-rs/src/audio.rs` | marker-gated Unix listener, AAudio stream, framing, volume and device commands |
| Platform device/volume JNI | `app-rs/src/surface.rs` | `AudioManager.getDevices()`, stream metadata and `STREAM_MUSIC` volume |
| Guest probe | `runtime/crates/audio-probe` | pure Rust device/volume checks plus a 440 Hz PCM16 one-shot stream |
| Guest relay | `runtime/crates/audio-relay` | pure Rust PulseAudio pipe FIFO → persistent AAudio session |
| Guest setup | `runtime/crates/audio-setup` | idempotent helper-library copy, private state dirs and minimal PulseAudio config |
| App manifest | `app/app/src/main/AndroidManifest.xml` | `MODIFY_AUDIO_SETTINGS` for volume control |

The bridge thread starts once with the NativeActivity. It does nothing until
`files/audio-bridge` exists; normal application startup does not open an AAudio stream. When the
marker exists, the app binds `files/audio-bridge.sock` and accepts clients.

## 3. Protocol v1

Text requests and replies are newline-delimited UTF-8. `STREAM` switches to a binary PCM phase
after the `STREAM_READY` reply: exactly `frames * channels * 2` bytes of little-endian PCM16 are
read by the app.

```text
PING
DEVICES
VOLUME GET
VOLUME SET <percent 0..100>
VOLUME INDEX <android-index>
STREAM frames=<n> rate=<hz> channels=<n>
OPEN rate=<hz> channels=<n>
DATA frames=<n>
<exactly n * channels * 2 bytes PCM16LE>
CLOSE
QUIT
```

`STREAM` is the original one-shot probe (`frames <= 48_000 * 30`). The long-lived `OPEN` / `DATA` /
`CLOSE` session is the new path: each `DATA` request carries at most 1,048,576 frames and waits for
one `DATA_OK` acknowledgement before the next chunk. `rate <= 384_000` and `channels <= 8`.

## 4. Measured path

The 3-second run on RMX3357 produced:

```text
DEVICES count=7
speaker id=3 rates=48000,44100 channels=2
STREAM_READY rate=48000 channels=1 format=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1
DONE status=PASS input_bytes=288000 frames=144000/144000 xruns=1 latency_ms_avg=125.784 latency_ms_max=128.000
```

`latency_ms` in the current probe is **queue-depth latency**, not end-to-end speaker latency. On
every successful `AAudioStream_write` the probe samples:

```text
queued_frames = AAudioStream_getFramesWritten() - timestamp.framePosition
latency_ms   = queued_frames * 1000 / actual_sample_rate
```

It excludes the DAC/speaker path and does not replace a loopback or human-ear measurement.

Fact: the app requested AAudio low-latency mode (`AAUDIO_PERFORMANCE_MODE_LOW_LATENCY = 12`), but
this vendor policy rejected it and returned `perf=10` (`AAUDIO_PERFORMANCE_MODE_NONE`). Android
logged `aaudio denied with imcompatible policy such as peformance noise` [sic]. The measured
125.784 ms average queue latency must therefore not be described as low latency.

### Continuous PulseAudio path

Fact: evidence 62 started PulseAudio 16.1 in the guest with a minimal `module-native-protocol-unix`
plus `module-pipe-sink` configuration, then used the pure Rust `audio-relay` to read the pipe FIFO
and forward it through the `OPEN` / `DATA` / `CLOSE` protocol. A standard guest `paplay` process
played `/usr/share/sounds/freedesktop/stereo/audio-test-signal.oga` and exited 0.

```text
relay|OPEN|OPEN_READY rate=48000 channels=2 format=1 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1
relay|BEGIN|fifo=.../audio.pcm|rate=48000|channels=2|seconds=10
relay|CLOSE|CLOSE_OK input_bytes=348140 frames=87035 frames_written=87035 ... latency_ms_avg=104.680 latency_ms_max=106.667 xruns=1 elapsed_ms=10003
relay|PASS|chunks=43|frames=87035|elapsed_ms=10014
RESULT PASS
```

Setting the AAudio buffer to one burst (`buffer=1026`) reduced the measured queue depth from the
earlier 125.784 ms average to 104.680 ms in this run. The platform still returned `perf=10 (NONE)`;
this is not a low-latency claim.

### Ten-minute final-loader run

Fact: evidence 63 ran a generated 600-second, 48 kHz stereo OGG through the standard guest
`paplay`, PulseAudio, the pure Rust relay, and the final loader. The relay was allowed 610 seconds
so it could drain and close after the player completed. Both `paplay` and the relay returned 0.

```text
relay|CLOSE|CLOSE_OK input_bytes=115273728 frames=28818432 frames_written=28818432 rate=48000 channels=2 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1 latency_ms_avg=106.661 latency_ms_max=106.667 xruns=1 elapsed_ms=610003
relay|PASS|chunks=14072|frames=28818432|elapsed_ms=610015|last_ack=DATA_OK frames=1024 input_bytes=115273728 frames_written=28818432 xruns=0
RESULT PASS
PAPLAY_RC=0 RELAY_RC=0
```

The app fd count moved from 120 to 112 at the first sample and stayed there; relay fd/RSS stayed at
6 / 1392 KiB. App RSS stayed in the 121,176–121,376 KiB range. The final close statistic reports one
xrun, so this is not an xrun-free claim.

## 5. Current boundaries

- Fact: device enumeration, `STREAM_MUSIC` volume get/set/restore, real AAudio open/write/close,
  `dumpsys audio` volume events, a persistent AAudio session, PulseAudio pipe output, the Rust relay,
  and standard guest `paplay` are proven.
- Unknown: human audibility was not independently confirmed in this run.
- Fact: standard guest playback through `paplay` now reaches AAudio through the long-lived session.
- Fact: the ten-minute continuous run passed on the final loader with the standard guest `paplay`;
  both the player and relay exited 0. One final xrun was reported.
- Not complete: GStreamer and desktop-player validation; only `paplay` is proven.
- Not complete: xrun-free operation and AV synchronization with video.
- Not complete: low latency. `perf=10` is a vendor-policy fallback even after buffer sizing.
- Provisioning is implemented in Rust by `audio-setup`: it copies the PulseAudio helper/module
  libraries, creates `tmp/husk-pulse/runtime` and the physical rootfs `dev/shm`, writes
  `minimal.pa`, and can create the guest-side `enabled` marker. The command was run twice on the
  final loader with identical `PASS` output. The app bridge accepts either that marker or the
  legacy host marker.

## 6. Provisioning command

Run from the host/run-as side with the current loader (the app's `files/libdir` value):

```sh
LIB=$(adb -s <serial> shell 'run-as com.linuxhub.myth.husk cat files/libdir' | tr -d '\r')
R=/data/data/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs
adb -s <serial> shell "run-as com.linuxhub.myth.husk sh -c 'cd /data/data/com.linuxhub.myth.husk/files && NR_LD=$LIB/libld.so sh gexec2.sh linux/distributions/ubuntu-24.04/rootfs $R/usr/bin/audio-setup / --enable'"
```

`audio-setup` is idempotent and only writes the guest audio state. When the logical root is `/`, it
uses `NR_ROOTFS` for the physical private `dev/shm` and state paths; this is required because `/dev`
is a deliberate host passthrough tree. The host-side legacy marker is not required when the
guest-side `enabled` marker exists.

## 7. Next implementation gate

The next gate is GStreamer/desktop-player compatibility and the same measurements while the video
path is active, including AV sync. If the vendor policy continues to refuse low-latency mode, the
final evidence must record that platform limitation instead of claiming a latency target was met.
