# Evidence 61 — path root fd preservation and loader regression

Device: RMX3357 / Android 13 / kernel 4.19.191 / arm64  
Time: 2026-11-23 05:56–05:59 CST  
Conclusion: **PASS**. The runtime root descriptor survives guest `close(1024)`, and the corrected
31-check functional matrix passes 31/31 with the current embedded loader.

## 1. Bug and fix

Fact: PulseAudio closes inherited descriptors before its final `chdir("/")`. When the path shim kept
the guest root at a low descriptor, a guest bulk close could destroy it; every subsequent absolute
path then failed with `ENOENT`/`EBADF`.

Fix in `runtime/crates/path-shim/src/lib.rs`:

- duplicate the root `O_PATH` descriptor to fd 1024 or above with `F_DUPFD_CLOEXEC`;
- verify it with `fcntl(F_GETFD)` and reopen when necessary;
- interpose `close()` so the root fd is preserved;
- interpose `close_range()` and punch a one-fd hole around the root fd.

## 2. Focused proof

The guest explicitly closed fd 1024 and continued resolving absolute guest paths:

```text
rootfd env=/data/data/com.linuxhub.husk/files/rootfs-full fd=1024
close preserved rootfd
NAME="Ubuntu"
chdir realnow=/data/data/com.linuxhub.husk/files/rootfs-full/etc
/etc
```

## 3. Functional matrix

The existing matrix initially reported 7 failures because it assumed the Ubuntu 22.04 line ordering
of `/etc/os-release`; Ubuntu 24.04 starts with `PRETTY_NAME`, not `NAME`. The test now asserts the
`NAME=` field instead. This was a test-fixture bug, not a loader regression.

Final result with the packaged app's current loader:

```text
#checks=31 fail=0
```

All 31 checks passed, including mkdir/cp/mv/rm, chmod, relative and absolute symlinks, `..` clamping,
absolute and relative exec, `pwd -P`, stdio, subshell and large I/O.

## 4. Reproduce

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c 'cd /data/user/0/com.linuxhub.husk/files && R=/data/user/0/com.linuxhub.husk/files/rootfs-full; NR_DEBUG=1 NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full \$R/usr/bin/bash /01-functional-matrix.sh 2>&1'"
```

Do not use the stale `files/libld39.so` default when validating the current app.

## 5. Artifacts

```text
app APK sha256         : 9e10a41d3ad32ebdf2c20eed9db17ba35eb7c42f4c245d3742c343c6bdf904c2
source libld sha256    : 92000b676169f39509d8cb53b9e20b6cf4cc0935477f0d1d5a5bf4601380a6fc
APK libld sha256       : 92000b676169f39509d8cb53b9e20b6cf4cc0935477f0d1d5a5bf4601380a6fc
matrix log sha256      : 712b57a812ba12ddc701dbf02afdfaf92a2f80fd362409a4bdcc66b44f2b0ad6
close-test log sha256  : 8b8953533d8e39d16f458f278377e1c949b4c64ce98c0e35c15f397a706ccd8d
```

- `docs/design/evidence/61-path-rootfd-regression-matrix.log`
- `docs/design/evidence/61-path-rootfd-close-preserved.log`

## 6. Boundary

The matrix uses the full Ubuntu 24.04 rootfs on this device. It does not prove every bulk-close path
inside every guest daemon; the PulseAudio startup remains a separate integration test. It does prove
the specific high-fd and `close()` failure reproduced during audio work no longer breaks later path
resolution.

## 7. Packaging integrity

Fact: AGP previously stripped `libld.so` even though it is a deliberately patched guest ELF. The app
Gradle configuration now keeps `**/libld.so` byte-for-byte. After rebuilding and installing the APK,
the source file and the `lib/arm64-v8a/libld.so` extracted from the APK both hash to
`92000b676169f39509d8cb53b9e20b6cf4cc0935477f0d1d5a5bf4601380a6fc`.
