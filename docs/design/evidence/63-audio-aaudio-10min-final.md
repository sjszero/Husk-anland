# Evidence 63 — ten-minute guest audio session on the final loader

Date: 2026-11-23
Device: RMX3357 / MT6893 / Dimensity 1200 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
Repository: `$REPO`
Loader under test: `6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c`

## Conclusion

**FACT: PASS for a 610-second continuous guest audio session using a standard guest `paplay`,
PulseAudio `module-pipe-sink`, the pure Rust `audio-relay`, and the app-process AAudio bridge.**

This is a duration and resource-stability result. It is **not** an end-to-end speaker latency
measurement, a low-latency result, or a human-ear confirmation.

## 1. Final provisioning regression

`audio-setup` was run twice on the final loader with the guest logical root `/` and `NR_ROOTFS`
pointing at the private rootfs:

```text
audio-setup|ENABLE|/data/data/com.linuxhub.husk/files/rootfs-full/tmp/husk-pulse/enabled
audio-setup|PASS|root=/|helper_modules=11|module_dirs=1|state=/data/data/com.linuxhub.husk/files/rootfs-full/tmp/husk-pulse
HOST_RC=0
```

The second run produced the same result, so the setup path is idempotent.

A real provisioning bug was found and fixed during this pass: when the guest logical root is `/`,
`/dev` is intentionally a host passthrough tree. The old code's `create_dir_all("/dev/shm")`
therefore targeted Android's host namespace and returned `EACCES`. The setup now uses the physical
`NR_ROOTFS/dev/shm` path when the guest root is `/`. The same fix also makes the default state
directory physical (`NR_ROOTFS/tmp/husk-pulse`) so PulseAudio's absolute `mkfifo`/module paths stay
inside the private rootfs.

## 2. One-shot smoke revalidation

The final `audio-probe` enumerated seven output devices, changed `STREAM_MUSIC` from index 3 to 5
(30%), restored index 3, and wrote a 3-second, 440 Hz PCM16 signal:

```text
DEVICES_BEGIN count=7
DEVICE index=1 id=3 type=2 type_name=speaker sink=true name="RMX3357" rates=48000,44100 channels=2
VOLUME OK index=3 max=16
VOLUME OK index=5 max=16 percent=30
VOLUME OK index=3 max=16
STREAM_READY rate=48000 channels=1 format=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1
DONE status=PASS input_bytes=288000 frames=144000 requested_frames=144000 frames_written=144000 rate=48000 channels=1 device=3 burst=1026 capacity=2052 buffer=2052 perf=10 sharing=1 latency_ms_avg=125.784 latency_ms_max=128.000 xruns=1 elapsed_ms=5958
RESULT PASS
HOST_RC=0
```

App-side proof:

```text
audio|client|connected|fd=86
audio|stream|open|rc=0|rate=48000|channels=1|format=1|device=3|burst=1026|capacity=2052|buffer=2052|perf=10|sharing=1|xruns=0
audio|stream|done|pass=true|input_bytes=288000|frames=144000/144000|frames_written=144000|xruns=1|latency_avg_ms=125.784|latency_max_ms=128.000|elapsed_ms=5958|state=4
audio|client|disconnected
```

## 3. Ten-minute continuous path

The test used an ffmpeg-generated 600-second, 48 kHz stereo OGG sine file and the standard
guest `paplay`. The relay was configured for 610 seconds so it could receive the final frames and
issue `CLOSE` after the player finished. PulseAudio remained the guest mixer; the app process owned
the only AAudio stream.

```text
relay|OPEN|OPEN_READY rate=48000 channels=2 format=1 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1
relay|BEGIN|fifo=/data/data/com.linuxhub.husk/files/rootfs-full/tmp/husk-pulse/audio.pcm|rate=48000|channels=2|seconds=610
relay|CLOSE|CLOSE_OK input_bytes=115273728 frames=28818432 frames_written=28818432 rate=48000 channels=2 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1 latency_ms_avg=106.661 latency_ms_max=106.667 xruns=1 elapsed_ms=610003
relay|PASS|chunks=14072|frames=28818432|elapsed_ms=610015|last_ack=DATA_OK frames=1024 input_bytes=115273728 frames_written=28818432 xruns=0
RESULT PASS
PAPLAY_RC=0 RELAY_RC=0
```

App-side session close:

```text
audio|client|connected|fd=121
audio|stream|open|rc=0|rate=48000|channels=2|format=1|device=3|burst=1026|capacity=2052|buffer=1026|perf=10|sharing=1|xruns=0|set_buffer=1026
audio|session|close|input_bytes=115273728 frames=28818432 frames_written=28818432 rate=48000 channels=2 device=3 burst=1026 capacity=2052 buffer=1026 perf=10 sharing=1 latency_ms_avg=106.661 latency_ms_max=106.667 xruns=1 elapsed_ms=610003
audio|client|disconnected
```

The player and relay both exited zero. The relay's final per-chunk acknowledgement still reported
`xruns=0`; the final close statistic reported `xruns=1`. Do not describe this run as xrun-free.

## 4. Resource samples

The sampler read `/proc/<pid>/fd` and `VmRSS` every 60 seconds:

```text
SAMPLE=0 APP_FDS=120 APP_RSS_KB=121176 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=1 APP_FDS=112 APP_RSS_KB=121360 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=2 APP_FDS=112 APP_RSS_KB=121360 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=3 APP_FDS=112 APP_RSS_KB=121360 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=4 APP_FDS=112 APP_RSS_KB=121376 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=5 APP_FDS=112 APP_RSS_KB=121376 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=6 APP_FDS=112 APP_RSS_KB=121368 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=7 APP_FDS=112 APP_RSS_KB=121368 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=8 APP_FDS=112 APP_RSS_KB=121368 RELAY_FDS=6 RELAY_RSS_KB=1392
SAMPLE=9 APP_FDS=112 APP_RSS_KB=121368 RELAY_FDS=6 RELAY_RSS_KB=1392
```

The app fd count dropped from 120 to 112 at the first sample and stayed constant. Relay fd and RSS
were constant after start. App RSS changed by only 192 KiB across the sampled window; this is not a
linear leak signal.

## 5. Reproduction shape

Provision with the current installed loader:

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/data/com.linuxhub.husk/files/rootfs-full
adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files && NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full $R/usr/bin/audio-setup / --enable'"
```

The continuous path used the private state directory
`$R/tmp/husk-pulse` and the dedicated app socket `files/audio-bridge.sock`. The exact harness was
kept on the device during this run; the important invariant is that `PULSE_SERVER` points at the
private `native` socket, `audio-relay` reads the private `audio.pcm` FIFO, and `paplay` runs through
the same loader.

The generated test source was:

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=600' \
  -ac 2 -c:a libvorbis -q:a 3 /tmp/husk-10min.ogg
```

## 6. Artifact hashes

```text
audio-setup  : d5d7a54c2c112bfe6f0e37fa7a04e882409c34a94054d94740a40f64686cc9ca
audio-relay  : 970fac1b61a5dfe97f8c21aa49dc37593d17a3d9b1e3a19042b2ffb74df61bc2
audio-probe  : 456929ef315e5c73a9f713f97f7621ee933eece5d9186ddba8844114b6bf9983
libhuskapp.so: 47b9ed38c42fb9aa1095ee1b5130c92d13fbb25986489d0e639f161f6bc05c9c
libld.so     : 6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c
APK          : ab72a38dc592bfd4c08a986e399f6d0c271d04926c97dfd024ab54cbad0c309c
```

Raw logs:

- `63-audio-aaudio-10min-final-app.log`
- `63-audio-aaudio-10min-final-relay.log`
- `63-audio-aaudio-10min-final-sampler.log`
- `63-audio-aaudio-10min-final-pulse.log`
- `63-audio-aaudio-10min-final-paplay.log`
- `63-audio-aaudio-10min-final-setup.log`
- `63-audio-aaudio-10min-final-probe.log`
- `63-audio-aaudio-10min-final-probe-app.log`

## 7. Boundaries

- **FACT**: the standard guest `paplay` path completed a 600-second source through PulseAudio, the
  Rust relay, and a persistent AAudio session; both harness processes returned zero.
- **FACT**: the app and relay stayed alive for the 610-second session with stable fd/RSS samples.
- **UNKNOWN**: no independent human-ear or analog loopback confirmation was performed.
- **UNKNOWN**: the queue-depth latency is not end-to-end speaker latency.
- **NOT DONE**: low latency. The platform still returned `perf=10 (NONE)` and rejected the requested
  low-latency policy.
- **NOT DONE**: GStreamer/desktop-player compatibility, video-plus-audio AV sync, and the same
  session while the video path is active.
- **NOT DONE**: device-to-device portability. This evidence is only for the attached RMX3357.
