# Evidence 94 — camera permission denial reaches the guest as EACCES

Date: 2026-11-23 15:3x · device RMX3357 (`UKHMMB8PYTGECMNV`) · app `com.linuxhub.husk`

## What the guest sees now

```
$ python3 -c 'os.open("/dev/video0", O_RDWR)'
[husk v4l2] open: camera permission denied (app side)
/dev/video0 -> errno 13 Permission denied
/dev/video9 -> errno 19 No such device          # out-of-range index, for contrast
```

Three pieces make that true:

1. **app** (`camera.rs`): when the camera list is empty (nothing this process may
   open) or the test marker `files/camera-deny` exists, `INFO` is answered with
   `ERR no_permission` and the app logs `camera|DENY|reason=…`. It also opens
   `/dev/video0`'s session never — no fake frames are produced.
2. **shim** (`v4l2-shim`): `query_info` recognises `ERR …`, records whether it
   was a permission refusal, and `open()` then returns `-EACCES` instead of the
   generic `-ENODEV`, printing one line of explanation.
3. **loader path shim**: `v4l2_open()` now distinguishes three outcomes —
   *ours and opened*, *ours and failed with errno X*, *not ours* — and the
   caller turns the middle one into the guest's errno instead of falling through
   to a stray `open()` of the host node.

## The bug this uncovered (worth more than the feature)

`v4l2_shim::err()` returned a **positive** errno:

```rust
fn err(e: i64) -> i32 { e as i32 }      // before
fn err(e: i64) -> i32 { -(e as i32) }   // now
```

Every caller checks `fd < 0` to detect failure, so on any failed open the guest
received the *errno number* as a file descriptor:

```
/dev/video0 -> fd 13      (13 = EACCES)
/dev/video9 -> fd 19      (19 = ENODEV)
   close err 9            (EBADF — the "fd" was never open)
```

It only surfaced because failure paths had never been exercised: the device was
always present and the camera permission always granted. Downstream, this is
exactly how a denial would have looked like "camera works, then everything is
EBADF", which is much harder to diagnose than a clean EACCES.

## Limits of this test (stated, not hidden)

- **FACT**: this ROM refuses both ways of inducing a real denial from adb:
  `pm revoke … CAMERA` → `SecurityException: … nor current process has
  android.permission.REVOKE_RUNTIME_PERMISSIONS`; `appops set … CAMERA ignore`
  → `SecurityException: … does not have android.permission.MANAGE_APP_OPS_MODES`.
- Reinstalling without `-g` would drop the 1.5 GB rootfs inside `files/`, so it
  is not an option on this device.
- Therefore the marker exercises **our** degradation path; the platform's own
  behaviour on a genuinely ungranted permission (whether
  `ACameraManager_getCameraIdList` returns an empty list or an error status) is
  **UNKNOWN**. The app treats a zero-length list as denial, which is the
  documented Camera2 behaviour, but that branch has not been seen on-device.
