# Roadmap

This is what the project promises publicly. `docs/GOALS.md` is the internal
scope definition; if the two disagree, GOALS wins and this file is the bug.

## Shipped

- Loader + path shim: a glibc userland running as a normal app process, no
  `ptrace`, no namespaces (v1.0.0).
- Guest basics at parity with a container: bash, locale, user management, `dpkg`,
  `apt` (install and configure), `getent`, `sudo` via the guest's own helpers.
- GNOME 46 desktop on X11 inside the app, 30 minutes idle with flat RSS, thread
  and fd counts.
- Three-runner benchmark harness and published numbers (`docs/BENCHMARKS.md`).

## Next

Ordered by what blocks a user, not by what is fun.

1. **GPU on the desktop path.** The guest can open `/dev/mali0` and PanVK works;
   the 2.0 desktop still renders with `swrast`/`llvmpipe`. Until the zero-copy
   path (ScreenCast → PipeWire → AHardwareBuffer → SurfaceControl) carries
   frames, every desktop number is "software rendering".
2. **Static binaries.** A statically linked guest program cannot be interposed,
   so it sees Android paths. Documentation covers the mechanism; a fix needs
   either a linker-level answer or an explicit allow-list.
3. **Distribution coverage.** The loader is built against one glibc; a second
   distro needs a second loader build and an asset revision, not a new code path.
4. **Codec and camera zero-copy.** H.264 via the platform decoder is verified;
   HEVC/VP9/AV1 and camera buffers still copy.

## Not planned

`root`, bootloader unlocking, hardware virtualization, `ptrace`, Termux as a
delivery form, cross-device tuning without hardware, and any claim that the
software-rendered desktop path is "hardware accelerated".
