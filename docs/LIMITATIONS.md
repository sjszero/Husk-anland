# Limitations

What Husk does not do, or does not do yet. Everything below is either measured on the
one target device or explicitly marked UNKNOWN. A limit that is not listed here has
not been studied, which is not the same as "works".

Target device for every statement in this file: RMX3357 (MT6893 / Mali-G77 /
Android 13 / kernel 4.19.191 / 4 KiB pages / arm64), running as `untrusted_app`
without root.

## 1 · The desktop is still software-rendered (FACT, not fixed)

GNOME Shell 46 comes up under the 2.0 integration with
`MESA_LOADER_DRIVER_OVERRIDE=swrast` and `GALLIUM_DRIVER=llvmpipe`. The guest
GPU path exists — a guest process opens `/dev/mali0` directly and PanVK reports a
logical device, compute, offscreen graphics and Zink surfaceless all work
(evidence 97) — but the Mali kbase on this MTK 4.19 kernel is not detected by the
Mesa 25.2.8 PanVK build that ships in the rootfs, and guest EGL still fails with
`DRI3 error: Could not get DRI3 device` because the shim's synthetic `renderD128`
cannot answer the DRM version query.

Consequence for the benchmarks: the desktop dimensions (settings to first frame,
window drag fps, 1080p playback) compare **runner overhead**, not GPU. They are
labelled that way in `docs/BENCHMARKS.md` and must not be quoted as a GPU result.

## 2 · One device, one rootfs, one ABI (FACT)

Only arm64 Android, 4 KiB pages, glibc 2.39 (Ubuntu 24.04) has been run. The
loader patching is version-checked (`nr-tools scan-syscalls` asserts the glibc
version and the exact number of `set_robust_list` / `rseq` / `clone3` sites), so a
different distribution or glibc build fails loudly instead of producing a broken
runtime. 16 KiB / 64 KiB kernels were exercised on QEMU (evidence 125) but not on
a phone. Non-Mali GPU families have a probe table and a documented fallback, not
an adapter.

## 3 · No root, no unlocked bootloader, no virtualization (design, permanent)

Husk runs inside the ordinary app sandbox. Anything the app sandbox cannot reach
stays out of reach: kernel modules, other apps' data, `/proc` entries hidden by
`hidepid`, raw block devices, and system-wide policy knobs.

## 4 · Termux is not a delivery form (FACT, measured)

The guest tree needs binaries and libraries that Termux's `$PREFIX` does not
provide; running the installer there stops at missing dependencies. The supported
target is the app's own private directory (`run-as … files/…`) — see
`docs/RUNNING.md`. This is not a packaging gap that a script can close.

## 5 · `apt upgrade` on libc6 breaks the runtime (FACT, workaround shipped)

`apt` upgrading `libc6` replaces the patched guest libc with the stock one and
drops two runtime patches: the libc-internal slot page and `clone3 → ENOSYS`.
After that, `getent shadow` returns empty, the sudo repair step fails and the
desktop refuses to start with a misleading `/bin/sh: 1: Cannot fork`
(evidence 154). Re-apply with `tools/guest-config/install-libc-patch.sh`.

## 6 · Guest rootfs must not contain absolute symlinks (FACT, tool shipped)

The kernel resolves a symlink target against `/`, so an absolute link inside the
rootfs escapes to the Android filesystem. 552 of them existed in the Ubuntu
24.04 tree; `libblas.so.3` and every other `alternatives` link were affected.
`tools/guest-config/relative-symlinks.py --apply` rewrites them to relative paths;
this is a required rootfs preparation step (`docs/RUNNING.md`), not a one-off fix.

## 7 · Measurement is done through `adb shell`, whose stdout is a socket (FACT)

`/proc/self/fd/1` inside the guest is a socket. After `make` writes to it (309
bytes is enough), every later write in that session fails with `EACCES`. The same
happens under proot, so it is not a shim behaviour, but it does constrain how the
benchmark may be sampled: build steps redirect to `/dev/null` and only the wall
clock is recorded (evidence 155).

## 8 · Not started

Sensors, GPS, USB, Bluetooth, NFC, fingerprint, WSI/swapchain, hardware video
decode (HEVC / VP9 / AV1), hardware encode, camera zero-copy. Browser availability
inside the guest is UNKNOWN in this release: no browser package is part of the
benchmark rootfs, and none was run.
