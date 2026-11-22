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
  -> build + install the probe APK          (tools/probe-app)
  -> am start ... --es exe libld.so --es root nrfull/root
        libld.so --library-path <root>/usr/lib/aarch64-linux-gnu <root>/usr/bin/bash /<script>
```

`libld.so` is the embedded loader from `tools/loader/build-embedded-loader.sh`
(gradle strips it, so compare behaviour, not hashes). The launcher sets
`NR_ROOTFS=<root>`, `NR_LD=<nativeLibraryDir>/libld.so` (app-private files are not
executable, the native library dir is) and `NR_SIGSYS=log`.

## Expected layout on the device

```
files/nrfull/root/            # real rootfs: glibc 2.39, bash, coreutils, grep, findutils
  etc/ usr/ ...               # plus the gcc 13 toolchain from provision-toolchain.sh
```

## Scripts

| file | what it is | run |
|---|---|---|
| `01-functional-matrix.sh` | 31 checks: mkdir -p, mv/cp/rm, chmod, symlinks (relative/absolute/`..`), stat/ls/find, glob, pipes, subshell, `pwd -P`, `..` clamping, `/evil` containment | `./run-guest.sh 01-functional-matrix.sh` |
| `03-containment.sh` | 11 escape attempts (symlinks out, `..` climbs, `/proc/self/root`), plus the mount points that pass through by design | `./run-guest.sh 03-containment.sh` |
| `02-bench.sh` + `bench.c` | per-call cost of the path layer, compiled and timed in the guest | `EXTRA=bench.c WAIT=90 ./run-guest.sh 02-bench.sh` |
| `accept-gcc-make.sh` | gcc -> cc1 -> as -> collect2 -> ld.bfd, then `make -j4` / no-op / clean | `WAIT=60 ./run-guest.sh accept-gcc-make.sh` |
| `provision-toolchain.sh` | host-side: resolve, unpack and push the Ubuntu gcc/make toolchain (skips libc6 on purpose). Thin wrapper over `nr-tools provision` (Rust); `NR_TOOLS=python` runs `provision-toolchain-legacy.sh` | `./provision-toolchain.sh` |

`run-guest.sh` environment knobs: `DEV=` device id, `ENV=K=V,K2=V2` extra environment
(a single `--es env` value, comma separated), `EXTRA="file.c"` extra files to push,
`WAIT=` seconds to wait for the run (default 20).

The scripts print `PASS`/`FAIL` per check and a final `#checks=`/`#escapes=` line, and
exit non-zero if anything failed; `run-guest.sh` prints the tail of the app's output,
including `#exit=`.

## Where the numbers live

Results are written up with device output in `docs/design/evidence/`:
`14` (matrix without LD_PRELOAD, exec cost), `16` (gcc + make), `17` (start at `/`,
matrix re-run), `18` (containment, per-call cost, the `execl` gap).
