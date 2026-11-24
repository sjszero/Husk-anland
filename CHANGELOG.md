# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).
Every claim below carries the evidence number it came from; the raw output lives in
`docs/design/evidence/`. The pre-1.0 engineering diary (36 entries, chronological)
is kept in `docs/design/DEVLOG.md`.

## [Unreleased]

## [0.01] - 2026-11-24

First release. Reference environment: arm64 Android 13, kernel 4.19.191, 4 KiB pages,
running as `untrusted_app`, no root (the exact device is in `docs/STATUS.md`; it is a
measurement condition, not a requirement).

### Added

- **Native runtime.** An aarch64 loader with an embedded path shim runs an
  unmodified glibc userland in-process: no ptrace, no namespaces, no
  virtualization, no root. Path virtualisation is done once by rewriting the
  GOT of each loaded object, so the program's own `svc` goes straight to the
  kernel (evidence 130–146).
- **Ubuntu 24.04 + GNOME Shell 46, X11 session**, with top bar, dock, wallpaper
  and notifications on screen (evidence 147).
- **Touch input** through `TouchInputHandler` → in-process X server → XI2 →
  mutter (evidence 148).
- **`apt` works**: `apt-get install` from 815 to 882 packages in 27 s, including
  `build-essential`, `git`, `vim`, `nodejs`, `python3-numpy`, `nautilus`,
  `gnome-terminal` (evidence 154).
- **Working toolchain**: `gcc`, `make -j4` on a 200-object project, `git clone`,
  `python3` + `numpy` (evidence 155).
- **Session services**: polkit authorization (`pkcheck` rc=0), at-spi
  accessibility bus, gvfs without session-bus fallback, xrandr RandR 1.2 with no
  scaling warning (evidence 151, 152).
- **Benchmark harness** (`tools/bench/`) that runs the same rootfs under three
  runners — Husk, proot, proot-distro — with the runner as the only variable,
  and the results in `docs/BENCHMARKS.md`.
- **Standalone flavour**: `tools/guest/install-husk.sh` installs a rootfs and
  generates a `./husk` launcher; no JVM, no Python, no APK required
  (`docs/RUNNING.md`).

### Changed

- proot removed from the Android integration: the same rootfs boots through the
  loader only, with no functional loss (evidence 144).

### Fixed

- `statvfs` / `statvfs64` were missing from the shim, so `apt` measured free
  space on a host path (evidence 154).
- The static `ldconfig` could not see the guest; replaced by `husk-ldconfig`
  (evidence 154).
- `execve` kept argv in a 64-slot stack array, silently dropping every argument
  after the 63rd; `make -j4` on a 200-object project failed on a link error that
  pointed at `__TMC_END__` instead of at argv (evidence 155).
- 552 absolute symlinks inside the rootfs resolved against the Android root;
  rewrote them as relative links, which made `numpy`, `libblas` and `liblapack`
  load by soname (evidence 155).
- `nr-sigsys.log` noise: one line per session instead of 4 499 (evidence 152).

### Known limitations

- The desktop is software-rendered (`swrast`/`llvmpipe`). The guest GPU path
  exists but is not wired to the desktop yet; see `docs/LIMITATIONS.md` §1.
- Sensors, GPS, USB, Bluetooth, NFC, fingerprint, hardware video decode, WSI:
  not started.
- No browser in the shipped rootfs (the offline package index does not contain
  one). Termux is not a supported environment.

[Unreleased]: https://github.com/xishanp/Husk/compare/v0.01...HEAD
[0.01]: https://github.com/xishanp/Husk/releases/tag/v0.01
