# Evidence 70 — Guest-visible AF_UNIX socket path translation

Date: 2026-11-23
Device: RMX3357 / MediaTek MT6893 / Dimensity 1200 / Mali-G77 MC9 /
Android 13 / kernel 4.19.191 / 4 KiB pages / arm64
Repository: `/Users/l/Documents/Codex/2026-08-10/husk`

## Conclusion

**FACT: path-shim now translates pathname `AF_UNIX` addresses passed to `connect(2)`.**
A glibc guest rooted at the private `files/rootfs-full` tree connected to the
guest-visible path `/run/husk/video.sock` with the normal `NR_ROOTFS` environment.
The app-side bridge accepted the connection on the physical socket
`files/rootfs-full/run/husk/video.sock`. Rootfs isolation was not disabled.

This closes the video bridge's deferred socket-path item. It does not close the
remaining video work: AHB import/display, HEVC/VP9/AV1, VA-API/GStreamer/Chromium,
A/V sync, or the ten-minute playback gate.

## Implementation

- `runtime/crates/path-shim/src/lib.rs` interposes `connect()`.
- Only pathname `AF_UNIX` addresses are translated.
- Abstract sockets, unnamed sockets, and non-`AF_UNIX` addresses pass through
  unchanged to the raw kernel syscall.
- The guest path is resolved inside the existing root fd. The shim connects via
  `/proc/self/fd/<resolved-parent>/<final-name>`, then closes the temporary
  parent fd. A failed guest path cannot fall back to the host path.
- The app bridge now uses `rootfs-full/run/husk/video.sock`; the enable marker is
  `rootfs-full/run/husk/enabled`.

## Real-device verification

Raw logs: `70-guest-unix-socket-path.log`,
`70-guest-unix-socket-path-app.log`, `70-loader-regression.log`.

Guest connection, using the loader actually installed in the app:

```text
loader=/data/app/~~F0aoJufTkP4mEZs0EF_QeQ==/com.linuxhub.husk-Mtjp4KjChB5pRgHPeFkcLg==/lib/arm64/libld.so
rootfs=/data/data/com.linuxhub.husk/files/rootfs-full
guest_socket=/run/husk/video.sock
CONNECT_OK path=/run/husk/video.sock guest_cwd=/ root=/data/data/com.linuxhub.husk/files/rootfs-full
HOST_RC=0
```

App-side socket lifecycle:

```text
video|bridge|API|libmediandk.so
video|bridge|LISTEN|/data/user/0/com.linuxhub.husk/files/rootfs-full/run/husk/video.sock
video|bridge|client|connected|fd=3
video|bridge|client|read|peer closed
video|bridge|client|disconnected
```

The test stimulus was a short Python socket client already present in the guest
rootfs. It is test-only; no Python is shipped in Husk.

## Regression gates

The rebuilt loader was rerun inside the same guest root:

| Gate | Result |
|---|---|
| `01-functional-matrix.sh` | `#checks=31 fail=0`, `HOST_RC=0` |
| `03-containment.sh` | `#escapes=0`, `HOST_RC=0` |
| `04-path-reliability.sh` | `#checks=15 fail=0`, `faccessat 8/8`, `dirfd 31/31`, `HOST_RC=0` |

## Artifact hashes

```text
0f3eca76af7cac8258258ea1e6d8932f070034dbd8e913cc19d29797bd13c229  libld.so
b09997c6e3d0cb67c7dbe3ae66ee43929a697b0267f2c8685edc35bf508eee7f  libhuskapp.so
8501304e58a9ba43834f056ae9a27863d7f9da2715b4a6c0f13d344b2fd9f7d8  app-debug.apk
```

## Boundaries

- **FACT**: this evidence covers a guest client connecting to an app listener.
- **BY DESIGN**: `bind()` / `listen()` path translation is not added; the guest
  is currently the client side of these runtime bridges.
- **NOT TESTED**: socket paths longer than the kernel `sun_path` limit, other
  devices, and 16/64 KiB kernels.
- **NOT DONE**: the full AVC media-plane rerun over the guest-visible socket still
  needs the 1080p test asset installed in this rootfs. Evidence 69 already closes
  the media data plane with the physical private socket path.

