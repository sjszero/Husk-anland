# Benchmarks

One device, one rootfs, three runners. The runner is the only variable; every
number below is a median of 5 measured repetitions on the same phone.

## What was measured

| 项 | 值 |
|---|---|
| Device | RMX3357, MT6893 (Dimensity 1200), Mali-G77, Android 13 (SDK 33), kernel 4.19.191, 4 KiB pages, arm64 |
| Sandbox | `untrusted_app`, no root, no Shizuku, no unlocked bootloader, no virtualization |
| Guest | Ubuntu 24.04, glibc 2.39, 882 dpkg packages, one rootfs directory |
| Husk | loader `rev 23`, sha256 `d5dd7e63…`, path shim embedded, zero ptrace |
| proot | the `proot` binary and exact flag set LinuxHub 2.0 shipped (`-0 -r --link2symlink -b /dev -b /proc -b /sys`) |
| proot-distro | the same binary with proot-distro's invocation profile |
| Time | 2026-11-24 16:40–17:10 CST |
| Samples | 5 per item per runner (`apt-get update` cold: 2, because it deletes the index) |
| Battery | 100 % throughout, battery temperature 27.7–28.5 °C |
| Power | USB powered (adb needs USB); the device was never on battery during the run |

Two details that decide whether the table means anything:

**The three runners read the same directory, not three copies.** The rootfs is
one tree under the app's private directory; Husk, proot and proot-distro are
started with their root pointed at it. There is nothing to checksum-compare
because nothing was duplicated — a file added between runs would be visible to
all three.

**The runners are interleaved, not run one after another.** Running each runner
to completion takes ~25 minutes, and this phone throttles: the same Husk block
measured 1.5–1.8× slower at 16:32 than at 16:05 (`stat`, `find`, `tar`, `make`).
So the five repetitions rotate through the three runners inside one round
(`tools/bench/run-interleaved.sh`), with a 60-second cooldown between blocks.
Thermal and page-cache drift therefore hits all three runners the same way.

Reproduce with:

```
tools/bench/install.sh                       # push the device-side half
tools/bench/run-interleaved.sh 5             # 5 reps, 3 runners, round-robin
tools/bench/compare.py docs/design/evidence/bench-*-il.log
```

Raw logs: `docs/design/evidence/bench-{husk,proot,proot-distro}-20261124-1640-il.log`.

## The table

| Dimension / item | Husk | proot | proot-distro | Husk vs proot |
|---|---:|---:|---:|---:|
| **1 · Startup** | | | | |
| Cold start to `/bin/true` (includes adb) | 155 ms | 151 ms | 162 ms | 1.0× |
| `apt-get update`, cold (index deleted) | 7,337 ms | 7,901 ms | 7,906 ms | 1.1× |
| `apt-get update`, warm | 7,347 ms | 7,874 ms | 7,941 ms | 1.1× |
| **2 · Process & syscall density** | | | | |
| fork+exec × 100 | 363 ms | 550 ms | 564 ms | 1.5× |
| `/bin/true` × 50 | 207 ms | 309 ms | 315 ms | 1.5× |
| Shell loop, 1 000 iterations | 16 ms | 30 ms | 28 ms | 1.9× |
| `getpid` × 20 000 | 62 ms | 104 ms | 99 ms | 1.7× |
| `stat` × 20 000 | 650 ms | 3,734 ms | 3,796 ms | 5.7× |
| `open`+`close` × 20 000 | 1,252 ms | 8,481 ms | 8,697 ms | 6.8× |
| `make -j4`, 200-object link | 5,194 ms | 24,182 ms | 24,864 ms | 4.7× |
| **3 · File I/O** | | | | |
| `find /usr` | 1,484 ms | 5,464 ms | 5,623 ms | 3.7× |
| Source tree, `tar xf` | 533 ms | 2,051 ms | 2,124 ms | 3.8× |
| `grep -r` over headers | 214 ms | 713 ms | 885 ms | 3.3× |
| Sequential write, 256 MB | 1,294 ms | 1,321 ms | 1,295 ms | 1.0× |
| Sequential read, 256 MB (page cache) | 133 ms | 159 ms | 156 ms | 1.2× |
| 10 000 small files, create+delete | 239 ms | 687 ms | 666 ms | 2.9× |
| Local `git clone` + `status` | 889 ms | 3,357 ms | 3,380 ms | 3.8× |
| **4 · Packages & dev workflow** | | | | |
| `dpkg -l` | 45 ms | 89 ms | 93 ms | 2.0× |
| `apt-get -s install`, resolver | 89 ms | 258 ms | 249 ms | 2.9× |
| `gcc` hello world | 165 ms | 358 ms | 385 ms | 2.2× |
| `python3` import json | 88 ms | 169 ms | 156 ms | 1.9× |
| `python3` import numpy | 410 ms | 726 ms | 722 ms | 1.8× |
| **8 · Compatibility** | | | | |
| `bash` | yes | yes | yes | — |
| `gcc` | yes | yes | yes | — |
| `git` | yes | yes | yes | — |
| `make` | yes | yes | yes | — |
| `nano` | yes | yes | yes | — |
| `node` | yes | yes | yes | — |
| `python3` | yes | yes | yes | — |
| `vim` | yes | yes | yes | — |

`Husk vs proot` is proot ÷ Husk: 5.7× means Husk finished 5.7 times faster.

Dimension 5 (desktop and graphics), 6 (interaction latency) and 7 (30-minute
idle) are not in this table, and the reason is not "we did not get to it":

- **5 / 6** — the proot runner in this repository has no display server
  integration: proot was removed from the Android integration (evidence 144), so
  the proot path can start a shell but not a session, and the rootfs contains no
  X server of its own (`xvfb` is not in the offline package index). The desktop
  numbers that do exist are Husk-only and measured in-app: GNOME Shell 46 X11
  first frame with top bar, dock and notifications (evidence 147), and touch
  input through `TouchInputHandler` → X server → XI2 → mutter (evidence 148).
  A runner-to-runner graphics comparison needs a second display path, which this
  release does not have. Both paths are software-rendered when they run.
- **7** — measured separately as idle soaks, not as part of the timed rounds:
  Husk 30 minutes at 232 661 → 232 138 KiB PSS, 47 threads, 118 fds, 0 guest
  zombies (evidence 153); proot and proot-distro soaks are in
  `docs/design/evidence/soak-*.log`. Power draw on battery is UNKNOWN: adb
  requires USB, so the device was charging for every number on this page.

## Why

The gap is not a constant. It tracks syscall and path density, which is exactly
what the two designs differ on.

- **Husk does not intercept syscalls.** Path virtualisation happens once, when
  the loader rewrites the GOT of each object; after that the program's own `svc`
  instruction goes straight to the Android kernel. Pure syscall loops therefore
  pay almost nothing: `getpid` × 20 000 is 62 ms vs 104 ms, `fork+exec` × 100 is
  363 ms vs 550 ms — 1.5–1.7×, the cost of the process work itself, not of a
  translation layer.
- **proot intercepts every syscall with ptrace.** Each `stat`, `open`, `close`,
  `readlink` stops the tracee, gets decoded, path-translated and resumed. That is
  why the path-heavy rows move the most: `stat` × 20 000 5.7×, `open`+`close`
  × 20 000 6.8×, `find /usr` 3.7×, `grep -r` 3.3×, `tar xf` 3.8×.
- **Bulk I/O is identical, and the table says so.** A 256 MB sequential write is
  1 294 ms vs 1 321 ms (1.0×) and a cached read is 133 ms vs 159 ms. Once a
  buffer is mapped and the data goes to the kernel in big blocks there is nothing
  for a runner to translate — the win disappears. Likewise `apt-get update`
  (7 347 ms vs 7 874 ms, 1.1×) is dominated by the mirror and by extraction, not
  by the runner.
- **Builds and interpreters sit in between**, because they are a mix of process,
  path and CPU work: `make -j4` 4.7×, `gcc` 2.2×, `import numpy` 1.8×,
  `git clone` 3.8×.

## What it costs

- **Maturity.** proot is a general tool with a decade of distribution quirks
  handled. Husk runs exactly the trees it has been verified against: arm64,
  glibc 2.39, Ubuntu 24.04. The loader's patching is version-asserted, so a
  different glibc fails loudly instead of working by accident.
- **Distribution coverage.** One distribution, one ABI, one device family
  (arm64 Android with 4 KiB pages verified; 16/64 KiB on QEMU only).
- **Rootfs preparation.** A stock Ubuntu tree needs three steps before it runs
  under Husk: libc patching, absolute symlinks rewritten to relative, and
  `husk-ldconfig` instead of the static `ldconfig`
  (`docs/RUNNING.md`). proot needs none of them.
- **Graphics.** The desktop is software-rendered; proot was never the reason the
  GPU did not work, and Husk does not fix it in this release
  (`docs/LIMITATIONS.md` §1).
- **Device access.** No sensors, GPS, USB, Bluetooth, NFC or fingerprint in
  either runner.
