# Evidence 60 — AAudio guest-to-app bridge minimum

Device: RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
Time: 2026-11-23 05:59:28–05:59:34 CST
Conclusion: **minimum one-shot path PASS; audio target not complete**.

## 1. What was proven

Fact: a glibc guest process in `untrusted_app` connected to a dedicated Unix socket in the Husk app,
enumerated seven Android output devices, changed and restored `STREAM_MUSIC` volume, opened a real
AAudio PCM16 output stream, wrote 144,000 frames / 288,000 bytes, and closed it successfully.

Guest result:

```text
PONG bridge=audio-v1
DEVICES_BEGIN count=7
DEVICE index=1 id=3 type=2 type_name=speaker sink=true name="RMX3357" rates=48000,44100 channels=2
DEVICES_END
VOLUME OK index=3 max=16
VOLUME OK index=5 max=16 percent=30
VOLUME OK index=5 max=16
STREAM_READY rate=48000 channels=1 format=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1
DONE status=PASS input_bytes=288000 frames=144000 requested_frames=144000 frames_written=144000 rate=48000 channels=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1 latency_ms_avg=125.784 latency_ms_max=128.000 xruns=1 elapsed_ms=5978
RESULT PASS
```

App result:

```text
audio|client|connected|fd=136
audio|stream|open|rc=0|rate=48000|channels=1|format=1|device=3|burst=1026|capacity=2052|buffer=2052|perf=10|sharing=1|xruns=0
audio|stream|progress|frames=144000/144000|input_bytes=288000/288000
audio|stream|done|pass=true|input_bytes=288000|frames=144000/144000|frames_written=144000|xruns=1|latency_avg_ms=125.784|latency_max_ms=128.000|elapsed_ms=5978|state=4
audio|client|disconnected
```

`dumpsys audio` independently recorded:

```text
11-23 05:59:28:688 setStreamVolume(stream:STREAM_MUSIC index:5 flags:0x0) from com.linuxhub.husk
11-23 05:59:34:838 setStreamVolume(stream:STREAM_MUSIC index:3 flags:0x0) from com.linuxhub.husk
```

## 2. Low-latency policy result

Fact: the app requested `AAUDIO_PERFORMANCE_MODE_LOW_LATENCY=12`. AudioFlinger rejected the
low-latency policy and the opened stream returned `perf=10` (`NONE`):

```text
AAudioService: aaudio denied with imcompatible policy such as peformance noise
audio|stream|open|...|perf=10|sharing=1
```

Therefore 125.784 ms average queue depth is a measured fallback number, **not** a low-latency
success. It is calculated from `framesWritten - timestamp.framePosition`; it is not end-to-end
speaker latency.

## 3. Reproduction

Build the guest probe:

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu \
-C link-arg=--sysroot=/tmp/gccroot \
-C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
-C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
cargo build --release --target aarch64-unknown-linux-gnu -p audio-probe
```

Deploy the probe and marker, then run with the **installed app's current loader**, not the stale
`files/libld39.so`:

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c 'cd /data/user/0/com.linuxhub.husk/files && NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full /data/user/0/com.linuxhub.husk/files/rootfs-full/usr/bin/audio-probe /data/user/0/com.linuxhub.husk/files/audio-bridge.sock 3 2>&1'"
```

## 4. Artifacts

```text
audio-probe sha256       : 456929ef315e5c73a9f713f97f7621ee933eece5d9186ddba8844114b6bf9983
app APK sha256           : 9e10a41d3ad32ebdf2c20eed9db17ba35eb7c42f4c245d3742c343c6bdf904c2
embedded libld sha256    : 92000b676169f39509d8cb53b9e20b6cf4cc0935477f0d1d5a5bf4601380a6fc
guest log sha256         : 445f0636fe82c9eaa6d84a86c79891464f57730e51572417eb179ad0d64874e9
app log sha256           : 325375e6628617be7a8cc9652416b6af60803cf1c44add23188e9e3f5e2da24b
dumpsys excerpt sha256   : d35baae16f587b64c710895b3a65a06400f96710965df9de1c9c96ffbc773ccc
```

- `docs/design/evidence/60-audio-aaudio-bridge-guest.log`
- `docs/design/evidence/60-audio-aaudio-bridge-app.log`
- `docs/design/evidence/60-audio-aaudio-bridge-dumpsys.log`
- `docs/design/audio-bridge.md`

## 5. Boundaries

- **UNKNOWN**: human audibility was not independently confirmed in this run.
- **NOT COMPLETE**: continuous stream, PulseAudio relay, standard player playback, AV sync.
- **NOT COMPLETE**: low latency and xrun-free 10-minute soak. The current run has one xrun.
- **NOT COMPLETE**: product provisioning. PulseAudio helper-library copies and runtime setup were
  performed manually during investigation and must be moved into Rust tooling.
