# Native runtime device tests

These run **on the phone**, inside the app's own uid, against a real Ubuntu arm64
glibc rootfs. Nothing needs root, Shizuku, chroot or namespaces.

## How a test actually runs

There is no `LD_PRELOAD` any more. The path shim is a `PT_LOAD` **inside the guest
loader** (`docs/design/evidence/14`), so every process that is started
through the loader is already interposed:

```
run-guest.sh <script>
  -> push <script> into <root>/<script>
  -> push husk-exec.sh into <root>/tmp/
  -> run-as <pkg> sh <root>/tmp/husk-exec.sh <root> <loader> usr/bin/bash /<script>
        cd <root>
        NR_ROOTFS=<root> NR_LD=<loader> libld.so --library-path <root>/lib... usr/bin/bash /<script>
```

No APK is built and nothing is installed: the guest is processes in the App's own
uid, so a test only needs `run-as`. `libld.so` is the embedded loader from
`tools/loader/build-embedded-loader.sh` (default output `dist/embedded-loader/`,
which is a local build artifact, not a source file).

## Expected layout on the device

```
files/rootfs-full/                                  # the rootfs the App launches
  etc/ usr/ ...                                  # glibc 2.39, bash, coreutils
files/linux/husk/libld.so                        # the loader under test
```

Overrides: `PKG=`, `ROOTFS=`, `LD=` (all relative to the app's data dir).

## Scripts

| file | what it is | run |
|---|---|---|
| `01-functional-matrix.sh` | 31 checks: mkdir -p, mv/cp/rm, chmod, symlinks (relative/absolute/`..`), stat/ls/find, glob, pipes, subshell, `pwd -P`, `..` clamping, `/evil` containment | `./run-guest.sh 01-functional-matrix.sh` |
| `03-containment.sh` | 11 escape attempts (symlinks out, `..` climbs, `/proc/self/root`), plus the mount points that pass through by design | `./run-guest.sh 03-containment.sh` |
| `04-path-reliability.sh` | fd stability, overlength paths, final-symlink containment, optional Rust `faccessat` and relative-dirfd probes | run inside the guest with the current embedded loader |
| `02-bench.sh` + `guest-bench` | per-call cost of the path layer, timed in the guest | `EXTRA=guest-bench ./run-guest.sh 02-bench.sh` |
| `es3-workload.sh` + `gl-probe` | real ES3 workload (instancing/UBO/texture/MRT), pixel-checked in the guest | push `gl-probe`, then `sh /tmp/es3-workload.sh` |
| `accept-gcc-make.sh` | gcc -> cc1 -> as -> collect2 -> ld.bfd, then `make -j4` / no-op / clean | `./run-guest.sh accept-gcc-make.sh` |
| `05-mount-audit.sh` | mount/tmpfs/read-only audit: 8 findings from an external review; libc write/xattr/churn checks are Rust | `DEV=<serial> sh run-mount-audit.sh` |
| `16-umount-legacy.sh` | `umount <dir>` (the libmount no-flag path that calls the legacy `umount()`) and `umount <device>`: entry gone, tmpfs store directory gone, not-mounted still fails without touching the table (A-20, evidence 287) | `sh husk-exec.sh <rootfs> <loader> usr/bin/bash tmp/16-umount-legacy.sh` |
| `17-gate-a-hw.sh` | A-hw group: CAMERA/RECORD_AUDIO granted by a *user* (no `install -g`), no `USER_FIXED`, a real 3-frame Camera2 capture through the app's socket, and no crash records. A-core is untouched (evidence 288) | `DEV=<serial> ./17-gate-a-hw.sh` |
| `18-gpu-loader-defaults.sh` | loader-injected GPU defaults in a non-login process, plus the two explicit llvmpipe controls | `DEV=<serial> ./18-gpu-loader-defaults.sh` |
| `28-snapshot-pipewire.sh` | guest husk end-to-end GNOME Snapshot test through PipeWire/WirePlumber: media readiness, V4L2 enumeration, 20s no-139 survival, redundant-caps skip, numeric target, real DQBUF frames, screenshot and logs | `DEV=<serial> ./28-snapshot-pipewire.sh <serial>` |
| `19-readback-suite.sh` | full 19-case Rust readback suite through the loader: width boundaries, canary tails, whole-block reads and screenshot row shape | `DEV=<serial> make readback-audit` |
| `21-gpu-soak-audit.sh` | four concurrent Rust zink/PanVK soaks with renderer, exit, GL error, fd and RSS checks; no desktop claim | `DEV=<serial> make gpu-soak` |
| `ipc-probe` | POSIX mqueue, SysV shm, SysV sem/msg (12 / 13 / 24 项) | `sh gexec.sh rootfs-full usr/bin/env /tmp/ipc-probe {mq,shm,sem-msg}` |
| `09-network-surface.sh` | netlink surface: `ip`/`hostname -I`/`/proc/net/*`/`getent` answer, writes are refused with EPERM/EOPNOTSUPP | `sh gexec.sh rootfs-full usr/bin/bash tmp/09.sh` |
| `10-device-nodes.sh` | device node surface by name, plus the three platform walls (`/dev` listing, `/dev/full`, `/dev/tty`) asserted as known limits | `sh gexec.sh rootfs-full usr/bin/bash tmp/10.sh` |
| `11-process-view.sh` | guest processes carry their real names (`ps`/`top`/`pgrep`), and another app's `/proc/<pid>` stays unreadable | `sh gexec.sh rootfs-full usr/bin/bash tmp/11.sh` |
| `12-supervisor.sh` | `husk-init`: services run, crash restarts with backoff, `once`/`on-failure` policies, SIGTERM stops in 0.5s, zero leftovers and zero zombies. Not run through `gexec.sh` (the supervisor is a static binary, ld.so cannot run it) | `adb shell 'run-as com.linuxhub.myth.husk sh /data/local/tmp/12.sh'` |
| `13-egress.sh` | outbound network: DNS, `send()` on a real socket, HTTP, HTTPS/TLS, and raw ICMP asserted as a known limit. The probe is Rust and needs no guest Python/curl/ping. Needs real connectivity | `DEV=<serial> sh run-egress-probe.sh` |
| `rt-server-parity.sh` | 90-item server-surface matrix: NSS/accounts, locks, xattr, mmap, UNIX/SCM_RIGHTS, SysV/POSIX IPC, sockets, proc/sys views, D-Bus and package state. All libc-level checks are Rust; guest Python/awk count is zero | `DEV=<serial> make server-parity` |
| `provision-toolchain.sh` | host-side: resolve, unpack and push the Ubuntu gcc/make toolchain (skips libc6 on purpose). Thin wrapper over `nr-tools provision` (Rust), the only implementation | `./provision-toolchain.sh` |

`run-guest.sh` environment knobs: `DEV=` device id, `ENV=K=V,K2=V2` extra environment
(comma separated, exported before the guest starts), `EXTRA="bench guest-bench"` extra
files to push into the guest's `/tmp`. The run's stdout/stderr is the guest's own, and
the exit status is the script's.

The `faccessat` branch of `04-path-reliability.sh` needs the guest-side Rust probe.
Build it against the staged Ubuntu glibc sysroot, then place the resulting binary
at `/path-probe` inside the guest:

```sh
GLIBC_SYSROOT=/tmp/nr-toolchain/root ./build-path-probe.sh
```

The source is `path-probe.rs`. It must run through the loader's GOT interpose;
a direct syscall would bypass the code under test.

The optional relative-dirfd branch needs a second guest-side Rust probe. It uses
the runtime `libc.so.6` from the same guest, links dynamically so the loader's
GOT interpose sees every call, and checks `openat`/`fstatat`/`faccessat`/
`mkdirat` containment plus an absolute symlink:

```sh
adb exec-out run-as com.linuxhub.myth.husk \
  cat files/linux/distributions/ubuntu-24.04/rootfs/usr/lib/aarch64-linux-gnu/libc.so.6 > /tmp/husk-glibc.so.6
LIBC_SO=/tmp/husk-glibc.so.6 ./build-dirfd-probe.sh
```

Place the result at `/dirfd-probe` inside the guest. `fdops-kernel-probe.rs` is
the separate static raw-syscall probe used to establish which `O_PATH` metadata
operations this 4.19 kernel accepts; build it with
`./build-fdops-kernel-probe.sh`.

The scripts print `PASS`/`FAIL` per check and a final `#checks=`/`#escapes=` line, and
exit non-zero if anything failed; `run-guest.sh` prints the tail of the app's output,
including `#exit=`.

## Where the numbers live

Results are written up with device output in `docs/design/evidence/`:
`14` (matrix without LD_PRELOAD, exec cost), `16` (gcc + make), `17` (start at `/`,
matrix re-run), `18` (containment, per-call cost, the `execl` gap).
