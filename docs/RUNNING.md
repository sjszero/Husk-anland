# Running Husk

Two ways to run a guest. Both need the same three things: our loader, a prepared
rootfs, and a directory the kernel lets us execute from.

| Flavour | What you get | Where the rootfs lives |
|---|---|---|
| **App integration** (LinuxHub 2.0) | shell, desktop (GNOME Shell 46 X11), touch input, audio, video | the app's private dir, reached through `run-as` |
| **Standalone** (`install-husk.sh`) | shell, `apt`, full toolchain | any directory you can execute from |

The standalone flavour does not use an APK, a JVM, Python or Termux. The app
integration is what adds a display server, so the graphical session is app-only;
see [Desktop](#desktop).

## Requirements

- arm64 Android, API 26+ (verified on Android 13).
- ~2.5 GB free where the rootfs goes; ~1.5 GB for an Ubuntu 24.04 tree.
- `adb` for the standalone flavour on a phone; a plain shell is enough on an
  Android terminal.
- An Ubuntu 24.04 rootfs tarball (or the `ubuntu-24.04-husk.tar` from the release
  assets, which is already prepared).

## Standalone

```sh
adb push husk-arm64.tar.gz /data/local/tmp/husk/
adb shell
cd /data/local/tmp/husk && tar xzf husk-arm64.tar.gz
sh install-husk.sh --rootfs ubuntu-24.04-husk.tar --dest ./root --loader ./libld.so
./husk
```

`install-husk.sh` extracts the tarball twice on purpose: Android refuses `link(2)`
inside app data, so the first pass leaves the hard links out and the second pass
goes through the shim, whose `link()` falls back to a copy. It then writes a
`husk` launcher next to the rootfs and verifies that the guest answers from its
own tree (`/etc/os-release`, `perl5.38.2` and `uncompress` are real files).

What `./husk` runs, under the hood:

```
NR_ROOTFS=<rootfs>  NR_LD=<loader>  NR_LIBRARY_PATH=<rootfs>/lib/aarch64-linux-gnu:...
  <loader> --library-path "$NR_LIBRARY_PATH" usr/bin/bash -l
```

Both environment variables are required. With `NR_ROOTFS` missing the shim has no
root to translate into and every path resolves against Android; with `NR_LD`
missing a nested exec cannot re-enter the loader.

## Preparing a stock rootfs

Three steps, in this order, on the extracted tree. A release asset is already
prepared; a stock Ubuntu tarball is not.

```sh
# 1. libc: slot entries for the calls libc makes internally, and clone3 -> ENOSYS
#    (Android's seccomp kills the process before glibc can take its fallback).
sh tools/guest-config/install-libc-patch.sh <rootfs> <pristine-libc-or-deb>

# 2. absolute symlinks inside the rootfs resolve against the Android root; rewrite
#    them relative. 552 of them exist in Ubuntu 24.04.
python3 tools/guest-config/relative-symlinks.py --apply <rootfs>

# 3. Ubuntu's /sbin/ldconfig.real is static, so the shim cannot translate it;
#    install husk-ldconfig instead.
sh tools/guest-config/install-ldconfig.sh <rootfs>
```

Skipping step 1 gives `Bad system call` at the first `pthread` use; skipping step
2 gives `ENOENT` on libraries that are obviously present; skipping step 3 gives
`Can't stat /lib` from `dpkg --configure`.

`apt upgrade` on `libc6` undoes step 1 — re-run it afterwards
(`docs/LIMITATIONS.md` §5).

## Desktop

The graphical session needs a display server. In this release the only one
wired up is the app's: the Android integration starts its own X server and then
GNOME Shell 46 inside the guest, with touch input reaching mutter through XI2.

```sh
# desktop, through the app integration
adb shell am start -n <app-id>/<activity>
# then the display button in the app; first frame in ~60–75 s on the target device
```

There is no standalone desktop: the rootfs ships no X server (`xvfb` is not in
the offline package index) and the app's X server is not reachable from an
unrelated process. `docs/LIMITATIONS.md` §1 also covers why the desktop is
software-rendered.

## Verifying a machine

```sh
./husk -c 'cat /etc/os-release | head -2; uname -m; nproc'
./husk -c 'dpkg -l | wc -l'                 # ~882 packages on the benchmark tree
./husk -c 'gcc --version | head -1'         # present on the benchmark tree only
./husk -c 'python3 -c "import numpy; print(numpy.arange(200000).sum())"'   # 499500
```

## Troubleshooting

| Symptom | Cause |
|---|---|
| `cannot execute the loader from here` | the directory is mounted `noexec`; use a dir where exec is allowed |
| `no writable TMPDIR` | nothing in `/tmp` or `/data/local` is writable for this uid; point `TMPDIR` at the rootfs |
| `Bad system call` | libc was not patched (step 1) |
| `ENOENT` on a library that exists | absolute symlinks in the rootfs (step 2) |
| `Can't stat /lib` from `dpkg` | static `ldconfig` (step 3) |
| output of `make` disappears over `adb shell` | `adb` gives the guest a socket as stdout; it is not a shim behaviour (`docs/LIMITATIONS.md` §7) |
