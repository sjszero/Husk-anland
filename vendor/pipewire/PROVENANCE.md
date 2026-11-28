# PipeWire GStreamer camera crash fix

This directory carries the native `gstpipewiresrc` fix used by Husk 0.0.37.

## Problem

When a reconfigure event carries caps that still accept the fixed caps already
running, PipeWire 1.0.5 compared only the first peer-caps structure. A fixed
caps is never equal to a range, so `gst_pipewire_src_negotiate()` disconnected
and reconnected an unchanged stream. The disconnect removed a dequeue buffer
while `videoflip` was still copying the downstream frame, causing Snapshot to
exit with SIGSEGV (139).

## Fix

The patch checks whether `current_caps` is a subset of the complete peer caps.
If so, the running stream is left connected and renegotiation is skipped.

Upstream reference:
`97699d707df374773fe5d6e32ea7362ef8dcf51b`
(`gstpipewiresrc: do not restart the stream when the running caps still fit`).

## Inputs

- Distribution: Ubuntu 24.04 arm64
- Package: `gstreamer1.0-pipewire 1.0.5-1ubuntu3.3`
- Source: PipeWire 1.0.5
- Original `src/gst/gstpipewiresrc.c` SHA256:
  `c3edb98be06d584a945ae68ba0356a29ccd0da7fe318c7360d780a538a74caaa`
- Patched `src/gst/gstpipewiresrc.c` SHA256:
  `9e289d0d95fe34bc80c048fece71a9fdd653654e79308849324e10bd94e22fbb`
- Original distro plugin SHA256:
  `15ac8b33ce86b7099a8eda2b0a539698e680911e01572c162978aef84761eae3`
- Patched plugin SHA256:
  `5baacf01e9a98f74a0d3fef88f8da16e9d6e54245b82e871a2b6844665ba7f01`
- Patched plugin size: `200920` bytes

The binary was built in the arm64 Ubuntu guest against Ubuntu 24.04
`libpipewire-0.3-dev`, `libgstreamer1.0-dev`, and
`libgstreamer-plugins-base1.0-dev`, then stripped. It must not be used as a
drop-in replacement for a different PipeWire SONAME major without re-running
the plugin audit and device checks.

## Verification

Evidence:
`docs/design/evidence/367-husk-camera-pipewire-20261128.md`

The 139 crash disappeared, `stream is running and new caps are still
acceptable; skipping renegotiation` was observed, and the stream continued at
about 30 fps. The subsequent black preview was independently traced to
near-black source frames; this patch does not claim to fix lighting or camera
exposure.
