# Audio prior-art audit

Date: 2026-11-23
Scope: guest PulseAudio output → Android app → phone speaker. This audit was done **before**
treating the current implementation as the final audio design.

## 1. Sources actually pulled and read

| Source | Fixed revision | What was read |
|---|---|---|
| LinuxHub 2.0 | `c5ffcf6a60856380c2f4e480665284835803f182` | `ContainerAudioBridge.kt`, `pulseaudio.pa`, session start/stop scripts, container lifecycle call sites |
| DroidNative | `fa591010d6e33adbf5a79178b874fd4abdc58931` | `AudioOutput.kt`, `assets/guest/pulse.pa`, `docs/architecture.md`, `T10AudioTest.kt` |
| Oboe | `ff15b63d7538019138db133fd56fd95a1aa4ee91` | `docs/FullGuide.md`, `include/oboe/AudioStream.h`, builder/latency-tuner behavior |
| Android AAudio header | sha256 `2ccfb08c250459681f1dc1dcb4f85183e68c903027bec0fc701f0fc17a21eaff` | callback ABI, performance mode, buffer sizing and xrun semantics |
| PulseAudio | `819f2c06ccdc105739e1c714de968f2af1d3e205` | `src/modules/module-pipe-sink.c`, `PULSE_LATENCY_MSEC` handling |

The revisions were shallow-cloned under `/tmp/refs2`; third-party source is not vendored into Husk.

## 2. What already existed

### LinuxHub 2.0

- Guest PulseAudio uses `module-pipe-sink` at fixed `s16le / 44100 / stereo`.
- The app starts a dedicated FIFO reader before the guest session to avoid the first-open race.
- The reader writes into Android `AudioTrack` with `MODE_STREAM` and blocking writes.
- Audio failure is isolated from the desktop; close interrupts the blocked reader and joins it.
- Session stop kills PulseAudio with the rest of the guest session.

### DroidNative

DroidNative has the same product-level architecture, but its audio implementation exposes useful
latency controls that Husk did not initially copy:

- fixed `s16le / 48000 / stereo` pipe sink;
- app reads the FIFO and writes an `AudioTrack` on a dedicated thread;
- `F_SETPIPE_SZ` shrinks the default 64 KiB pipe (about 340 ms) to 8 KiB;
- `PULSE_LATENCY_MSEC` is passed to guest programs (default 150 ms, user-selectable 50–400 ms);
- the Android feeder thread raises itself to urgent-audio priority;
- underrun count and peak samples are exposed for diagnostics;
- a 2-second 440 Hz test verifies bytes delivered, non-zero peak, and advancing playback head;
- the test suite covers `paplay`, a Flatpak sandbox, and the D-Bus session bus.

### Oboe / AAudio

- AAudio may return a performance mode different from the requested mode.
- Blocking writes can be valid, but Oboe documents that the lowest-latency path uses a
  high-priority AAudio data callback. A low-latency request combined with write-only mode may be
  downgraded.
- The callback is a real-time context: no file I/O, allocation, mutex, sleep, read or write from
  inside it. Data must cross the boundary through a non-blocking atomic FIFO/ring.
- `setBufferSizeInFrames()` combined with xrun observation is the documented runtime tuning point.

### PulseAudio

- `module-pipe-sink` is a FIFO writer; it reflects queued FIFO bytes in latency queries and can drop
  data when the FIFO is full.
- The default FIFO capacity is therefore part of the latency budget, not just an implementation
  detail.
- `PULSE_LATENCY_MSEC` affects PulseAudio client streams and is the correct guest-side control for
  programs that prefer small buffers.

## 3. Comparison with the current Husk implementation

FACT: Husk already proves the same basic route with a pure Rust app-side implementation:

```text
PulseAudio module-pipe-sink → pure Rust audio-relay → AAudio in app process
```

Differences from the prior art:

| Topic | LinuxHub 2.0 / DroidNative | Husk current baseline |
|---|---|---|
| App language | Kotlin | Rust only |
| Android output API | `AudioTrack` | AAudio NDK C ABI called from Rust |
| Guest → app control | FIFO only | dedicated socket for control/ack + FIFO for PCM |
| Format | 44.1 kHz or 48 kHz | 48 kHz stereo |
| Buffer control | DroidNative exposes pipe/track latency controls | AAudio buffer reduced to one burst; FIFO size not yet tuned |
| Low-latency path | not claimed | requested, but vendor returns `perf=NONE` with blocking writes |
| Long soak | external projects do not publish an equivalent 600 s result in the audited files | 610 s / 28,818,432 frames / final xrun=1 PASS (evidence 63) |

## 4. Decision

1. Keep the guest-side `module-pipe-sink` contract. Reinventing a guest audio protocol is not useful.
2. Keep direct Rust AAudio instead of introducing a Kotlin or C++ dependency.
3. Do not claim low latency on the current blocking-write path. The measured value is queue depth,
   and the vendor policy returned `perf=NONE`.
4. Next changes should borrow the proven controls, not rewrite the bridge:
   - shrink the PulseAudio FIFO with `F_SETPIPE_SZ` and handle `EBUSY` by draining once;
   - propagate `PULSE_LATENCY_MSEC` to guest media programs;
   - add a high-priority AAudio callback fed by a lock-free Rust ring if the target still requires
     lower latency;
   - expose peak, bytes, playback head, xrun and end-to-end/AV-sync probes.
5. Any future audio module must repeat this audit and append its conclusion here or in a sibling
   module-specific prior-art file before implementation.
