# Camera prior-art audit

Date: 2026-11-23
Scope: phone camera → guest Linux. Written before treating the Husk camera path as final.

## 1. Sources actually read

| Source | Revision | What was read |
|---|---|---|
| DroidNative/DroidNativeApp | `fa591010d6e33adbf5a79178b874fd4abdc58931` | `devices/CameraBridge.kt` (313 lines), `docs/device-passthrough.md` §Cameras, `tests/host/v4l2/feed.c`, `tests/host/v4l2/run.sh`, `env/Passthrough.kt` |
| Android NDK headers | NDK 29 + NDK 28 (`media/NdkImageReader.h`) | `NdkCameraManager.h`, `NdkCameraDevice.h`, `NdkCameraCaptureSession.h`, `NdkCaptureRequest.h`, `NdkCameraMetadata.h`, `android/looper.h` |
| On-device libs | pulled from RMX3357 `/system/lib64` | `libcamera2ndk.so`, `libmediandk.so`, `libandroid.so` symbol tables |

## 2. What DroidNative does (FACT)

- Guest sees **`/dev/videoN`** (character device 81:N), *not* a custom IPC client.
- `CameraBridge.kt` per camera: `Camera2` + `ImageReader(YUV_420_888, 3)` →
  `TEMPLATE_RECORD` repeating request → `acquireLatestImage` → convert to **NV12 or YUYV**
  → write into a **shared memory slot** (`SLOT_OFFSET=4096`, `N_SLOTS=4`, slot = w*h*2)
  → write a **24-byte token** `{magic, slot, bytes, seq, ts_us}` into a FIFO.
- Backpressure: the app may be at most **2 frames ahead** of the guest's dequeue counter,
  and it skips producing when the FIFO has no reader (`fifo.hasReader()`).
- Discovery extras they needed because Android forbids directory listing:
  `/sys/class/video4linux/videoN`, `/sys/subsystem/video4linux/devices/videoN`,
  and `/run/udev/data/c81:N` (else udev/GStreamer never see the device).
- Their PC-side test harness (`tests/host/v4l2/run.sh`) is effectively the acceptance list:
  `stat` shows `character special file 51:0`, `v4l2-ctl --info`, `--list-formats-ext`,
  `--set-fmt-video`, `--stream-mmap --stream-count=10` (file size + a known byte),
  `--stream-user`, and "host saw stop".
- Formats: `YUYV` + `NV12`, sizes ≤ 1920x1080 (≤16 of them). Not supported: `read()` I/O,
  `DMABUF` memory, controls, crop/selection, events.
- Permission: `CAMERA` requested by their UI when the user flips the switch; camera only
  works while their window is visible (their env service is `specialUse`).

## 3. What Husk takes

1. **Same guest interface** (`/dev/videoN`, V4L2 semantics) — the whole point of
   `docs/design/device-interface-choices.md`.
2. **Same app-side shape**: Camera2 + `ImageReader(YUV_420_888)` + repeating request +
   YUV→NV12 conversion, with the ring/token backpressure idea.
3. Their udev/sysfs discovery trio, because the guest's udev will hit the same wall.

## 4. What Husk changes

- **Pure Rust NDK** instead of Kotlin Camera2: `libcamera2ndk.so` + `libmediandk.so` via
  `dlopen`, no JVM. Verified working (evidence 89).
- **Our own wire protocol** will be Husk-shaped (socket + `OPEN/FRAME/CLOSE` like the audio
  and video bridges) rather than copying `dn_v4l2_proto.h`, because our emulation layer sits
  at libc interposition, not in a ptrace extension.
- **No `mmap`-based shm via fd passing in v1**: first version copies frames over a unix
  socket; revisit only if preview bandwidth demands fd passing. (DroidNative passes the shm
  fd with `setFileDescriptorsForSend`, which our guest side cannot rely on.)

## 5. Landmines already measured on this ROM (evidence 89)

`ACameraManager_create` returns a pointer instead of using its out-parameter;
`ACaptureRequest_setEntry_u8` is `(request, tag, count, data)`;
`ALooper_pollOnce` takes **no** looper argument;
exported spelling is `..._add`, `..._setEntry_u8`.
Unverified assumption to keep in mind: `ACameraCaptureSession` `onReady` never fired here.
