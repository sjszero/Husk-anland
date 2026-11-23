# V4L2 prior-art audit (camera + sensors + GPS)

Date: 2026-11-23
Scope: before writing any guest-side `/dev/videoN`, IIO or `/dev/gps0` emulation,
read what already exists. This file is the record of that read; the module code
must not diverge from it without updating it.

## 0. Where the prior art actually lives (FACT)

`DroidNative/DroidNativeApp` only contains the **app side** (`CameraBridge.kt`) and a
**PC test harness** (`tests/host/v4l2/{feed.c,run.sh}`). Its guest-side V4L2 emulation
is **not** in that repo — it is a PRoot extension in a fork:

| Source | Revision | Size | What it is |
|---|---|---|---|
| `DroidNative/proot` | `4ac94ecbb567608725e1d68d5ea617800a76c9c4` (pinned in `native-host/build.sh`) | `src/extension/dn_v4l2/dn_v4l2.c` 848 lines + `dn_v4l2_proto.h` 55 lines | The actual `/dev/videoN` emulation |
| `DroidNative/DroidNativeApp` | `fa591010d6e33adbf5a79178b874fd4abdc58931` | `CameraBridge.kt` 313, `Fifo.kt` 58, `Devices.kt` 148, `SensorBridge.kt` 127, `LocationBridge.kt` 128, `MicBridge.kt` 82 | Host side + sysfs layouts + acceptance tests |

Everything below marked FACT is read from those files at those revisions.

## 1. dn_v4l2: how a FIFO becomes a V4L2 capture device (FACT)

Design: `/dev/videoN` in the guest is a **FIFO owned by the app**. PRoot answers the
V4L2 ioctls on it and maps `VIDIOC_DQBUF` onto `read()` of one 24-byte token, so
blocking, `O_NONBLOCK` (`EAGAIN`), `poll`/`select`/`epoll` all behave exactly as the
kernel does for pipes — no readiness bookkeeping of their own.

Concrete mechanics worth copying:

1. **Device table**: scan `$PROOT_DN_VIDEO_DIR` for `videoN` FIFOs (`N` = 0..255), keep
   `{index, st_dev, st_ino, fifo, sock}`. A tracee fd is recognised by `fstat`-ing
   `/proc/<pid>/fd/<fd>` and matching `(st_dev, st_ino)` — i.e. **identity is the FIFO
   inode**, not the path.
2. **Host link**: connect to `videoN.sock`, receive `dn_v4l2_hello` **with the frame
   memory attached via `SCM_RIGHTS`**, mmap it shared. Hello carries
   `{magic, version=1, shm_size, n_slots, slot_size, slot_offset, n_sizes, sizes[16], name[32]}`.
   Sizes are the camera's real YUV sizes; the app picks the delivery size.
3. **Shared header**: first word of the shm is `consumed` — frames the guest dequeued.
   The app stays at most 2 frames ahead of it and skips producing when the FIFO has no
   reader (`fifo.hasReader()`), so an idle guest costs nothing. **This is the backpressure
   model**; there is no other flow control.
4. **Defaults**: `640x480` if offered, else the first size; default format `YUYV`.
5. **`fit_format()`** rounds G/S/TRY_FMT to the *nearest offered size* (squared distance);
   an unknown fourcc silently becomes `YUYV`; sets `field=NONE`,
   `bytesperline = w*2` (YUYV) or `w` (NV12), `sizeimage = w*h*2` or `w*h*3/2`,
   `colorspace = V4L2_COLORSPACE_JPEG` (full-range BT.601 — Android YUV_420_888).
6. **ioctls implemented**: QUERYCAP (`driver="droidnative"`, `device_caps =
   CAPTURE|STREAMING`, `version=6.1`), ENUM_FMT (YUYV, NV12), ENUM_FRAMESIZES,
   ENUM_FRAMEINTERVALS (1/30), G/S/TRY_FMT, G/S_PARM (`readbuffers=2`, 30 fps),
   ENUMINPUT (`Camera`), G/S_INPUT, G/S_PRIORITY, REQBUFS (≤8, MMAP|USERPTR),
   QUERYBUF, QBUF, STREAMON, STREAMOFF. Everything else → `-ENOTTY`. No
   `read()` I/O, no DMABUF memory, no controls/crop/events.
7. **mmap**: MMAP buffers report `m.offset = index << 24` (`BUFFER_OFFSET_SHIFT`);
   mmap on the device is rewritten to an **anonymous** mapping and the resulting address
   is remembered per buffer. Frames are `memcpy`'d into it at DQBUF.
8. **DQBUF**: requires `streaming` and at least one queued buffer (`-EINVAL` otherwise),
   then becomes a `read(2)` of one token `{magic, slot, bytesused, sequence, timestamp_us}`
   into the (larger) `v4l2_buffer`. On exit: bump `consumed`, copy
   `shm + slot_offset + slot*slot_size` into the first queued buffer (clamped to buffer
   length / `sizeimage`), clear `queued`, fill `bytesused/sequence/timestamp`.
9. **STREAMON**: `drain_fifo()` (drop stale tokens from a previous stream), reset
   `consumed = 0`, send `CMD_START` with `{w,h,pixelformat}`. **STREAMOFF**: `CMD_STOP`.
10. **S_FMT while streaming**: they do **not** return `EBUSY` — a client that vanished
    without STREAMOFF is indistinguishable from a live one, so *newest request wins*:
    stop streaming, adopt the new format. (Deliberate deviation from the kernel.)
11. **stat**: FIFO inode is reported as `S_IFCHR` with `st_rdev = makedev(81, N)` on
    `stat/fstat/lstat/fstatat/statx` — `stat -c '%F %t:%T' /dev/video0` prints
    `character special file 51:0` (81 decimal = 0x51).
12. **Socket timeouts**: 2 s on connect/recv/send, so a hung app can never stall every
    tracee; a broken link returns `-ENODEV` and a later ioctl reconnects.

## 2. Guest-visible discovery (FACT, and we hit the same wall)

Android forbids listing `/dev` and `/sys/class`, so globbing finds nothing; opening by
name works. The app therefore materialises (their `Devices.kt`):

- `/sys/class/video4linux/videoN/` with `uevent`, `dev` (`81:N`), `name` (camera label),
  `subsystem -> ../../class/video4linux`;
- `/sys/subsystem/video4linux/devices/videoN` (eudev prefers the `/sys/subsystem` layout
  and Android has none to hide);
- `/run/udev/data/c81:N` with `ID_V4L_CAPABILITIES=:capture:`, `ID_V4L_PRODUCT`, …
  Without it udev counts the device as uninitialised and GStreamer skips it.

Also: `/sys/dev/char/81:N` is expected by some tools.

## 3. Their acceptance list = our acceptance list (FACT)

`tests/host/v4l2/run.sh` is effectively the spec:

| check | expectation |
|---|---|
| `stat -c '%F %t:%T' /dev/videoN` | `character special file 51:0` |
| `v4l2-ctl --info` | card name from the camera |
| `v4l2-ctl --list-formats-ext` | at least `1280x720` |
| `--set-fmt-video=w=320,h=240,pixelformat=NV12 --get-fmt-video` | `sizeimage=115200` |
| `--stream-mmap --stream-count=10 --stream-to=f` | `10 * 640*480*2` bytes, and byte at `614400*9` matches |
| `--stream-user --stream-count=5` | `5 * 640*480*2` bytes |
| host saw `STOP` | the app tears the camera session down |

## 4. Sensors: IIO sysfs (FACT, `SensorBridge.kt`)

One `iio:deviceN` directory per sensor, bound at `/sys/bus/iio/devices`, **files rewritten
in place** while values change:

- `name` = channel name (`accel_3d`, `gyro_3d`, `magn_3d`, `als`, `pressure`, `proximity`),
  `label` = the Android sensor name; attribute files pre-created so a reader that opened
  them early still sees data.
- Fixed-width right-aligned values (`padStart(15)` + `\n`) so a reader never sees a mix of
  an old long and a new short number.
- Units: accel `in_accel_{x,y,z}_raw` with `in_accel_scale=0.001` (m/s²), gyro
  `in_anglvel_*_raw` scale `0.001` (rad/s), magn `in_magn_*_raw` scale `0.0001`
  (Android µT → *100* / 0.0001), `in_illuminance_input`, `in_pressure_input`
  (hPa → /10 = kPa), `in_proximity_raw`. Each 3-axis device carries an identity
  `*_mount_matrix`.
- Rate: `SensorManager.SENSOR_DELAY_UI`, throttled to 50 ms; starts only when a guest
  program uses the sensors, stops when it goes away (sensors are the one device they
  keep running for, with `prepare()` creating the tree even when the bridge is off).

## 5. GPS: NMEA over a FIFO (FACT, `LocationBridge.kt`)

`/dev/gps0` is a FIFO; the app writes `GGA` + `RMC` once per fix (checksummed,
`\r\n`), `ddmm.mmmmm,N` / `dddmm.mmmmm,E`, speed in knots, course in degrees.
Android location updates start only while the FIFO has a reader (polled every 2 s),
and stop when the reader disappears. Provider preference: `gps`, `fused`, `network`.

## 6. What Husk takes, and what Husk changes

Takes:

1. The **guest-visible contract** exactly: `/dev/videoN`, char device 81:N, the ioctl
   subset, YUYV+NV12, nearest-size rounding, `colorspace=JPEG`, `offset = i<<24`,
   anonymous mmap, 8 buffers, MMAP+USERPTR, the udev trio, and their acceptance list.
2. The **token + shared-frame-slot idea** as the shape of the data path (sequence,
   timestamp, bytesused are exactly what `v4l2_buffer` needs).
3. The **"newest S_FMT wins"** deviation, for the same reason (no way to reap a dead
   client on Android).

Changes (our runtime has no ptrace and no bind mounts):

1. **No FIFO, no fd-passing shm.** The guest-facing fd is a `memfd` (same trick as the
   DRM shim), so `poll/select/epoll` report readable and `close/dup` are real. `DQBUF`
   is a **synchronous `FRAME` transaction** on the existing `camera.sock` protocol
   (`OPEN`/`FRAME`/`STOP`, NV12 only); the frame is copied into the queued buffer.
   Consequence, recorded honestly: backpressure is request-driven instead of
   ring-driven — a frame is only produced when the guest asks for one, so the app
   can never run ahead of the guest.
2. **NV12 is the transport**, `YUYV` is converted in the shim when a client asks for it
   (their app converts; ours has one wire format for camera, and it is already NV12).
3. **Sizes come from the app at first use** (`INFO` command) instead of a hello message
   at connect, since our socket is line-oriented and has no fd passing.
4. Discovery files are **created inside the rootfs** and the three `/sys/...` paths are
   redirected by the shim (`/sys` is passed through to Android's real sysfs, which has
   no video4linux class).

## 7. Open risks

- `v4l2_buffer`/`v4l2_format` are large (`v4l2_format` is 208 bytes); every field we
  write must match Ubuntu's `linux/videodev2.h`, so the structs are transcribed from
  `/usr/include/linux/videodev2.h` in the guest sysroot, not from memory.
- One `camera.sock` client at a time today ⇒ second camera open must return `EBUSY`
  rather than block.
- `poll` always-readable means a broken stream shows up as `DQBUF` returning `EAGAIN`
  after the 1 s app timeout, not as a hung `poll`. Tools that trust `poll` will spin;
  `v4l2-ctl` and GStreamer both tolerate it, but it must be written down.
