# Evidence 91 — `/dev/video0` and `/dev/video1` are real V4L2 devices in the guest

Date: 2026-11-23 15:0x · device RMX3357 (`UKHMMB8PYTGECMNV`) · app `com.linuxhub.husk`
· guest = Ubuntu 24.04 rootfs (`files/rootfs-full`) · loader = `files/libld39.so` (embedded shim)

## What changed

接口形态按「guest 看到标准 V4L2 设备」设计（见 `../device-interface-choices.md` §4）：
V4L2 ioctl 子集 + udev 发现所需的三处合成路径。
Implementation: `runtime/crates/v4l2-shim` (no_std, linked into the loader shim) +
`runtime/crates/v4l2-test` (guest harness). No ptrace, no FIFO, no fd passing:

- `open("/dev/videoN")` → memfd; the shim asks the app `INFO` for camera sizes/ids,
  then keeps the camera socket for the session.
- `DQBUF` = one synchronous `FRAME` transaction; NV12 is the wire format, `YUYV` is
  converted in the shim.
- `mmap` on MMAP buffers = anonymous memory; `m.offset = index << 24`.
- `stat`/`statx`/`access` report `S_IFCHR` + `rdev = makedev(81, N)`.
- `/sys/class/video4linux*` and `/sys/subsystem/video4linux*` are redirected to a
  shadow tree the app creates in the rootfs (Android's real sysfs has no V4L2).
  `/run/udev/data/c81:N` is written by the app.

## Results (all FACT, same run on the device)

```
$ ./usr/bin/stat -c "%F %t:%T" /dev/video0
character special file 51:0
$ ./usr/bin/stat -c "%F %t:%T" /dev/video1
character special file 51:1

$ v4l2-test /dev/video0 10 NV12 /tmp/husk-v4l2
v4l2test|stat|chr=true|rdev=81:0
v4l2test|querycap|driver=husk|card=BACK|caps=0x4000001
v4l2test|fmt|YUYV 4:2:2|56595559
v4l2test|fmt|Y/UV 4:2:0|3231564e
v4l2test|sizes|n=16|[(1920, 1080), (1872, 864), (1440, 1080), (1280, 960)]
v4l2test|setfmt|640x480|3231564e|sizeimage=460800
v4l2test|getfmt|640x480|3231564e|sizeimage=460800
v4l2test|reqbufs|4
v4l2test|buf|0|len=460800|off=0
v4l2test|buf|1|len=460800|off=16777216
v4l2test|buf|2|len=460800|off=33554432
v4l2test|buf|3|len=460800|off=50331648
v4l2test|streamon
v4l2test|frame|0|idx=0|bytes=460800|seq=1|ts=62915784.633000
v4l2test|frame|9|idx=0|bytes=460800|seq=10|ts=62916085.501000
v4l2test|done|frames=10|bytes=4608000|fps=16.14
v4l2test|streamoff
v4l2test|OK
```

Cross-checks (same harness):

| dev | format | frames | bytes/frame | fps |
|---|---|---:|---:|---:|
| `/dev/video0` (BACK) | NV12 | 10 | 460800 | 16.14 |
| `/dev/video1` (FRONT) | YUYV | 8 | 614400 | 15.81 |
| `/dev/video1` (FRONT) | NV12 | 8 | 460800 | 15.43 |
| `/dev/video0` (BACK) | YUYV | 8 | 614400 | 14.52 |

Real pixels, both formats, both cameras:
[`91-v4l2-front-nv12.png`](91-v4l2-front-nv12.png) (NV12 wire format, PPM from the
guest buffer) and [`91-v4l2-front-yuyv.png`](91-v4l2-front-yuyv.png) (same scene,
YUYV converted by the shim). The back camera is covered in this position, so its
frames are black; the front proves the data path.

## Deviations from kernel semantics (deliberate, recorded)

1. `poll/select/epoll` on the device fd always report readable (memfd). `DQBUF`
   then blocks up to the app's 1 s frame timeout and returns `EAGAIN` (5 attempts
   when blocking, 1 when `O_NONBLOCK`, which the shim tracks through `fcntl`).
2. Frames are produced **on demand**, one per `DQBUF`, so throughput is ~15–16 fps
   at 640x480 through the socket copy (sensor cadence is 30). 要满 30 fps 得改成
   共享内存环形缓冲；预览需要时再做。
3. Second stream on the same device fails at `STREAMON` with `EBUSY` (the app
   answers `ERR busy`), not at `open`.
4. 不支持 `read()` I/O、DMABUF 内存、controls/crop/events —— 第一版只做 MMAP/USERPTR。

## Reproduce

```sh
adb -s UKHMMB8PYTGECMNV shell 'input keyevent KEYCODE_WAKEUP; svc power stayon usb; \
  am start -n com.linuxhub.husk/android.app.NativeActivity'
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk echo 1 > files/camera'   # start the bridge
adb -s UKHMMB8PYTGECMNV shell 'timeout 120 run-as com.linuxhub.husk sh files/gexec2.sh rootfs-full \
  ./tmp/husk-tools/v4l2-test /dev/video0 10 NV12 /tmp/husk-v4l2'
```

Landmine hit while measuring: with the screen off the app is frozen (`LcdOff`) and
the guest blocks forever inside the first `INFO` — keep the device awake.

## GStreamer: blocked, and exactly why (FACT)

`gst-launch-1.0 v4l2src device=/dev/video1` fails:

```
Cannot identify device '/dev/video1'.
gst_v4l2_open (): system error: Permission denied
```

Cause chain, measured:

1. `libgstvideo4linux2.so` is **dlopen'd**, so the loader-embedded shim (which only
   patches the GOT of objects present at startup) never sees its calls.
2. It imports `open64@GLIBC_2.17` (verified with `readelf --dyn-syms`), not
   `v4l2_open`. A preload that exports an **unversioned** `open64` does not get
   bound to a versioned reference here: with `LD_PRELOAD=libhusk_v4l2.so` in place
   the shim's `openat` was instrumented and **was never called**, and the fd it ends
   up with is Android's `/dev/video1` (0660 `media:system`) → `EACCES`.
3. `libv4l2.so.0` (which the plugin also links) would answer `v4l2_ioctl` etc., but
   the device open happens before any of those calls.

`runtime/crates/v4l2-preload` is the WIP fix (hooks for `open/openat/open64/*_2`,
`ioctl`, `mmap`, `stat`, the `v4l2_*` API, and a `HUSK_V4L2_API` handshake).
It is **not** wired in. Two ways out, both recorded in that crate:

- give it a version script so the symbols carry the glibc versions the plugin
  references (`open64` 2.17, `statx` 2.28, `fstatat` 2.33 …), or
- patch the GOT of every object `dlopen` returns (hook `dlopen` in the shim).

Until then: **guest programs that call libc normally work** (our harness, any tool
built like it); GStreamer's v4l2 plugin does not, so "preview through GStreamer" is
still open. A second, independent `LD_PRELOAD` crash was also observed when the
loader shim forwarded into the preload's function table (segv inside the shim on the
first device open), which is why the handshake is disabled for now.
