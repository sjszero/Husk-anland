# Evidence 92 — GStreamer's `v4l2src` runs on `/dev/videoN` (dup-safe fd identity)

Date: 2026-11-23 15:2x · device RMX3357 (`UKHMMB8PYTGECMNV`) · app `com.linuxhub.husk`
· guest = Ubuntu 24.04 rootfs (`files/rootfs-full`) · loader `files/libld39.so` (embedded shim)
· preload `rootfs-full/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so`

## Why a preload layer exists at all

`libgstvideo4linux2.so` is **dlopen'd**, and it imports **versioned** symbols:
`open64@GLIBC_2.17`, `ioctl@GLIBC_2.17`, `mmap64@GLIBC_2.17`, `close@GLIBC_2.17`,
`stat64@GLIBC_2.33` (`readelf --dyn-syms`). An unversioned LD_PRELOAD definition does
not satisfy those references, so the plugin still reached Android's own
`/dev/video*` (0660 `media:system`) and got EACCES. Fixed with a version script
(`runtime/crates/v4l2-preload/exports.map`, defines `GLIBC_2.17` + `GLIBC_2.33`);
`readelf -V` confirms `2h(GLIBC_2.17)` / `3h(GLIBC_2.33)` on our definitions.

Second trap in the same layer: **exported wrappers calling each other go through the
PLT** and rebind to libc. Measured: `open64` reached us, the `openat` *inside* it did
not, and every device open became EACCES. Fix: exported symbols are thin wrappers
around private `do_*` implementations.

## The bug that mattered: a private fd table cannot see a `dup`

Symptom: QUERYCAP / ENUM_FMT / REQBUFS / mmap / QUERYBUF all worked, then

```
[preload STREAMON req=0x40045612  fd=11  known=0
   target=/memfd:husk-v4l2 (deleted)
ERROR: from element /GstPipeline:pipeline0/GstV4l2Src:v4l2src0: Failed to allocate required memory.
```

`fd=11` **is** our memfd: GStreamer `dup()`s the capture fd (the buffer pool keeps its
own descriptor) and issues `STREAMON`/`STREAMOFF` on the duplicate. A table keyed by
fd number is structurally blind to that.

This is exactly what DroidNative's prior art avoids, and it is why the audit was worth
reading line by line: `dn_v4l2.c:137 device_of_fd()` keeps **no table at all**. On every
ioctl it asks the kernel:

```c
snprintf(path, sizeof(path), "/proc/%d/fd/%d", tracee->pid, fd);
stat(path, &st) && S_ISFIFO(st.st_mode) && find_device(st.st_dev, st.st_ino);
```

## Fix (both layers, shared crate)

- the memfd is named `husk-v4l2-<n>`, so the kernel's answer carries the device index;
- `Ctx::dev_of_fd` = fast path (fd our own `open` returned) → fallback
  `readlinkat(AT_FDCWD, "/proc/self/fd/<fd>")` + parse `/memfd:husk-v4l2-<n>`;
- `close()` tears the session down only when it is the **last** reference
  (`dup` shares the file description; the fd being closed is still open, so `refs == 1`
  means last);
- the fallback is skipped entirely when no device is open, so processes that never
  touch a camera pay nothing for the hooks;
- if the fd resolves to our memfd but this layer holds no state for it, that is the
  other interposition layer's session and it is reported, never silently swallowed.

## Second bug: a no_std panic was a silent 100% CPU spin

Between "INFO parsed" and "fd returned" the process sat in state `R` for as long as it
was watched, with no output. Cause: the memfd name was copied with
`name[..12].copy_from_slice(b"husk-v4l2-")` — 10 bytes into a 12-byte slot → panic →
`panic_handler { loop {} }`. Both preloads now print `[husk v4l2] panic` and exit the
thread group with a distinct code (102 / 103), the same rule the loader shim already
used (101). A silent spin is the worst possible failure mode for a shim: it looks like
a hang in the consumer.

## Results (FACT, same device, after the fix)

```
$ gst-launch-1.0 v4l2src device=/dev/video1 num-buffers=3 ! fakesink
Got EOS from element "pipeline0".
Execution ended after 0:00:00.402235384          rc=0
[hv ioctl=0x40045613 fd=11 known=1 target=/memfd:husk-v4l2-1 (deleted)]   ← STREAMOFF on the dup

$ gst-launch-1.0 v4l2src device=/dev/video0 num-buffers=10 ! videoconvert ! video/x-raw,format=RGB ! fakesink
Got EOS from element "pipeline0".
Execution ended after 0:00:00.986082846          rc=0        (≈10 fps, 0 lines with known=0)

$ gst-launch-1.0 v4l2src device=/dev/video1 num-buffers=8 ! videoconvert ! fakesink
rc=0            (0.588 s)
```

Regression on the other layer (loader shim, no preload):

```
$ v4l2-test /dev/video0 5 NV12 /tmp/husk-v4l2b
v4l2test|done|frames=5|bytes=2304000|fps=11.94
v4l2test|OK
$ v4l2-test /dev/video1 3 NV12 /tmp/husk-v4l2c
v4l2test|done|frames=3|bytes=1382400|fps=9.10
v4l2test|OK
```

## Side finding (not our regression, recorded because it looks like one)

`filesink location=/tmp/x.png` fails with ENOENT **with and without** the preload;
`videotestsrc ! pngenc ! filesink` reproduces it. That is the documented gap
"libc 内部调用槽位方案（未实现）": libgstreamer's own file opens go through
libc-internal `open`, which no GOT patch reaches. It is not caused by, and not fixed
by, this layer.

## Still open for the camera goal

| acceptance item | state |
|---|---|
| guest preview shows a picture | ✅ v4l2-test + GStreamer both deliver real frames, front and back |
| still photo (JPEG) valid | NOT DONE |
| 30 s recording playable | NOT DONE |
| front/back switch | ✅ `/dev/video0` BACK, `/dev/video1` FRONT |
| permission-denied degradation | NOT DONE |
