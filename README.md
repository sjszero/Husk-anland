<!-- The header block: title on one line, the logo on the next. GitHub only ends
     an HTML block at a blank line, so these must stay two separate blocks or the
     title falls back to left-aligned. -->
<h1 align="center">Husk</h1>

<p align="center">
  <img src="docs/assets/brand/husk-mark-hexagon-h-512.png" width="128" alt="Husk">
</p>

<p align="center">Run a full Linux distribution as an ordinary Android app process, on the Android kernel.</p>

<p align="center">
  <a href="https://github.com/xishanp/Husk/releases"><img alt="Release" src="https://img.shields.io/github/v/release/xishanp/Husk?style=flat-square&color=EA580C"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/xishanp/Husk?style=flat-square&color=2563EB"></a>
  <img alt="Platform: Android arm64" src="https://img.shields.io/badge/Platform-Android%20arm64-3DDC84?style=flat-square&logo=android&logoColor=white">
  <img alt="Rust 1.75+" src="https://img.shields.io/badge/Rust-1.75%2B-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="CONTRIBUTING.md"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-0F766E?style=flat-square"></a>
</p>

<p align="center">
  <b>English</b> · <a href="README.zh-CN.md">简体中文</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/BENCHMARKS.md">Benchmarks</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

Husk is a runtime for Android that executes an unmodified glibc userland —
Ubuntu 24.04 with `apt`, `gcc`, GNOME Shell — inside an ordinary `untrusted_app`
process. No root, no unlocked bootloader, no hardware virtualization, and no
`ptrace`: path virtualisation is done once, by rewriting the GOT of each object
at load time, so the program's own `svc` instruction reaches the Android kernel
unmodified.

## Table of contents

- [Why Husk](#why-husk) · [Features](#features) · [Quick start](#quick-start)
- [How it works](#how-it-works) · [Performance](#performance) · [Compatibility](#compatibility)
- [Limitations](#limitations) · [Build from source](#build-from-source)
- [Repository layout](#repository-layout) · [Documentation](#documentation)
- [FAQ](#faq) · [Contributing](#contributing) · [License](#license)

## Why Husk

Same phone, same rootfs, same shell, three runners — only the runner changes.
Medians of 5 rounds, measured 2026-11-24 (`docs/BENCHMARKS.md`):

| work | Husk | proot | ratio |
|---|---:|---:|---:|
| `open` + `close` × 20 000 | 1 252 ms | 8 481 ms | **6.8×** |
| `make -j4`, 200-object link | 5 194 ms | 24 182 ms | **4.7×** |
| sequential write, 256 MB | 1 294 ms | 1 321 ms | 1.0× |

The first two rows are path- and syscall-heavy, so they pay proot's per-syscall
`ptrace` round trip. The third is a bulk write that never touches a translated
path, so it ties — and the table says so rather than hiding it.

## Features

- **Zero `ptrace`, zero syscall interception.** The kernel sees the guest's own
  syscalls (evidence 130–146).
- **Unmodified glibc userland.** Ubuntu 24.04, glibc 2.39, no patched binaries in
  the guest tree apart from `libc` slot entries for calls it makes internally.
- **A package manager that works.** `apt-get install` grew the rootfs from 815 to
  882 packages in 27 s, including `build-essential`, `nodejs` and `python3-numpy`
  (evidence 154).
- **A real toolchain.** `gcc`, `make -j4` on a 200-object project, `git clone`,
  `python3` + `numpy` (evidence 155).
- **A desktop.** GNOME Shell 46 in an X11 session, with top bar, dock, wallpaper
  and notifications on screen (evidence 147), and touch input reaching mutter
  through XI2 (evidence 148).
- **Session services.** polkit authorization, the at-spi accessibility bus, gvfs,
  and RandR 1.2 with no scaling warning (evidence 151, 152).
- **Nothing privileged.** Runs in the ordinary app sandbox on a stock phone.
- **A comparison you can rerun.** `tools/bench/` runs the same rootfs under Husk,
  proot and proot-distro with one command; the raw logs are in the repository.

## Quick start

Requirements: an arm64 Android device (Android 8+), `adb`, ~2.5 GB free space in
the target directory, and a rootfs tarball.

```sh
# 1. install the launcher and unpack the rootfs (no APK, no root)
adb push husk-arm64.tar.gz /data/local/tmp/husk/
adb shell
cd /data/local/tmp/husk && tar xzf husk-arm64.tar.gz
sh install-husk.sh --rootfs ubuntu-24.04-husk.tar --dest ./root --loader ./libld.so

# 2. a shell
./husk

# 3. the package manager, from inside the guest
apt-get update && apt-get install -y vim
```

The graphical session is started by the Android app integration (LinuxHub 2.0):
see [`docs/RUNNING.md`](docs/RUNNING.md) for the desktop step and for the three
rootfs preparation steps a stock Ubuntu tree needs.

## How it works

<p align="center">
  <img src="docs/assets/architecture.svg" alt="Husk architecture" width="760">
</p>

A statically linked loader with an embedded path shim is the only thing Android
executes. When it maps a guest object it rewrites that object's GOT entries so
every libc path call lands in the shim, which prefixes the guest root once and
then calls the real syscall. `exec` is resolved by the loader itself, including
`PT_INTERP` and shebangs, so a nested exec re-enters the same loader instead of
falling back to the host. Because nothing stops the process afterwards, the
program's syscalls are native from the first instruction it runs.

Details: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Performance

Eight dimensions, three runners, one device — the summary table lives in
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md), together with the mechanics behind
each number and what the approach costs. Headline: the win scales with syscall
and path density (1.5× on process work, 3–7× on path-heavy work) and disappears
on bulk I/O.

## Compatibility

Verified on Ubuntu 24.04 / glibc 2.39 / arm64: `bash`, coreutils, `git`,
`python3` (+`numpy`), `gcc`, `make`, `vim`, `nano`, `node`, `apt`, `dpkg`,
GNOME Shell 46, `nautilus`, `gnome-terminal`, `gnome-control-center`, dbus,
polkit, at-spi, gvfs. No browser is part of the shipped rootfs. The full matrix,
including what each runner cannot do, is in
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).

## Limitations

- ⚠️ The desktop is **software-rendered** (`swrast` / `llvmpipe`). The guest GPU
  path exists and is not yet wired to GNOME.
- ⚠️ One distribution, one ABI, one device family; the loader's patching is
  version-asserted, so other glibc builds fail loudly instead of half-working.
- ⚠️ A stock rootfs needs preparation: libc patching, absolute symlinks rewritten
  to relative, and `husk-ldconfig` in place of the static `ldconfig`.
- Sensors, GPS, USB, Bluetooth, NFC, fingerprint, hardware video decode and WSI
  are not implemented. Termux is not a supported environment.

Full list, with the evidence for each: [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md).

## Build from source

```sh
# the loader (needs the NDK; asserts the glibc version it patches)
NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin \
  sh tools/loader/build-embedded-loader.sh

# the Rust crates (no_std shim + host tools)
cargo build --release --manifest-path runtime/Cargo.toml

# checks that run in CI
make check   # fmt + cargo check for aarch64 + clippy + shell syntax + docs links
```

## Repository layout

```
runtime/crates/    loader-side path shim, exec, PTY, signals (Rust, no_std)
runtime/crates/nr-tools/     ELF surgery: scan, patch, embed, libc interpose
tools/loader/      entry + hook assembly, the build that produces the loader
tools/bench/       the three-runner harness used for docs/BENCHMARKS.md
tools/guest/       rootfs installer and session script (no APK needed)
tools/guest-config/  libc patch, ldconfig, symlink rewrite
tools/device-tests/  device acceptance suites, including gate A
app-rs/, app/      the Android shell (Rust Core + Kotlin integration)
docs/design/evidence/  raw device output behind every number in the docs
```

## Documentation

| Document | What it answers |
|---|---|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | how the loader, shim and exec graph fit together |
| [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) | the comparison, the mechanism, the cost |
| [`docs/RUNNING.md`](docs/RUNNING.md) | installing and running, with the rootfs preparation steps |
| [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) | what runs, per runner |
| [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) | what does not work, and why |
| [`docs/STATUS.md`](docs/STATUS.md) | current state on the target device |
| [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md) | the pre-1.0 engineering diary |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | how to build, test, and the glossary |

## FAQ

**Does it need root or an unlocked bootloader?** No. It runs as `untrusted_app`
on a stock device; everything it can reach is what the app sandbox allows.

**Is this a container or a VM?** Neither. Same kernel, no namespaces, no
cgroups, no instructions emulated. The guest is a normal process tree.

**Why is `apt-get update` only 1.1× faster?** It is bound by the mirror and by
dpkg's own work, not by path lookups. The benchmark reports that instead of
hiding it.

**Can I run Debian, Fedora, Alpine?** Only glibc-based Ubuntu 24.04 has been
verified. Alpine is musl and a different problem entirely; nothing else was run.

**Why is the desktop software-rendered?** Because the GPU path is not wired to
GNOME yet — see `docs/LIMITATIONS.md` §1. It is not a side effect of the runtime.

**Why does my rootfs need patching?** Two Android-specific holes: `libc` needs
slot entries for the calls it makes internally, and absolute symlinks inside the
rootfs would resolve against the Android root. `docs/RUNNING.md` has the steps,
and `tools/guest-config/` has the tools.

**Does it run in Termux?** No, and it is not supported. The guest tree expects
binaries and libraries a Termux `$PREFIX` does not provide.

**Which Android and kernel were verified?** Android 13, kernel 4.19.191, 4 KiB
pages, arm64, on one device (RMX3357 / MT6893). 16 KiB and 64 KiB kernels were
exercised on QEMU only.

## Contributing

Issues and pull requests are welcome; read [`CONTRIBUTING.md`](CONTRIBUTING.md)
first, and note that every performance claim needs an evidence file under
`docs/design/evidence/`. This project follows the
[Contributor Covenant 2.1](CODE_OF_CONDUCT.md). Security reports: see
[`SECURITY.md`](SECURITY.md).

## License

Apache License 2.0 — see [LICENSE](LICENSE). Third-party components, glibc
artefacts and trademark notes are in [NOTICE](NOTICE).

## Acknowledgements

The comparison in `docs/BENCHMARKS.md` was produced by running
[proot](https://proot-me.github.io/) and proot-distro on the same rootfs; both
were the previous runtime of LinuxHub 2.0, and the numbers exist because they
work. The Ubuntu 24.04 tree started from the LinuxHub 2.0 offline assets, and the
loader patches target glibc as shipped by Ubuntu.
