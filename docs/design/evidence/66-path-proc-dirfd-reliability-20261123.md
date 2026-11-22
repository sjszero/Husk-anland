# Path semantics, procfs dirfd containment, and final reliability gate — evidence 66

Date: 2026-11-23
Device: RMX3357 / MT6893 / Dimensity 1200 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
Repository: `/Users/l/Documents/Codex/2026-08-10/husk`
Private test tree: `/data/data/com.linuxhub.husk/files/rootfs-full`
Final loader sha256: `6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c`

## Scope

This closes the two findings supplied in the latest audit and one additional
containment hole found while validating them:

- `BUG-001`: `linkat(AT_SYMLINK_FOLLOW)` could let the host kernel follow an
  absolute final symlink outside the guest root.
- `BUG-002`: `renameat2()` fell back to `renameat()` on `ENOSYS` even when flags
  such as `RENAME_NOREPLACE` could not be preserved.
- A real directory fd under passthrough `/proc` could escape through `..` or
  procfs magic links (`self/root`, `self/cwd`). The same paths were also exposed
  as absolute `/proc/...` paths.

## Implemented fixes

### 1. `linkat(AT_SYMLINK_FOLLOW)`

The old path is now resolved through the guest root before the kernel sees the
follow flag. The kernel never receives an unresolved absolute symlink target.

Device result:

```text
linkat-follow rc=-1 errno=13
PASS linkat-follow-resolved-guest
```

`EPERM` is Android refusing hard-link creation in this directory, not a host
lookup failure. Before the fix the guest-only target resolved as `ENOENT`.

### 2. `renameat2()` flag preservation

The legacy `renameat()` fallback is now used only when `flags == 0`. Every
non-zero flag is returned to the caller as the kernel reported it.

Device result:

```text
renameat2-noreplace rc=-1 errno=17
PASS renameat2-noreplace-preserved
```

### 3. Unbounded path string scan

`c_strlen()` scans at most `PATH_MAX` bytes. A missing NUL now becomes
`ENAMETOOLONG` rather than an unbounded memory walk.

### 4. More than 32 path components

The resolver previously held one fd per directory and rejected a valid 64-level
path with `Too many levels of symbolic links`. It now keeps only the current
directory fd and uses `..` from that fd; the lexical guest path still clamps at
the guest root.

Device result:

```text
PASS deep-path
```

### 5. Relative dirfd containment

All non-absolute `*at()` path calls now classify the dirfd:

- fd under the guest root: reconstruct a guest-absolute path and use the guest
  resolver;
- fd under the deliberately passed-through `/proc`, `/dev`, or `/sys` tree:
  permit only lexically safe relative paths;
- any other directory fd: deny with `EACCES`.

This covers `openat`, `fstatat`, `faccessat`, `readlinkat`, metadata operations,
mkdir/unlink, rename/link, and the shared two-path resolver. `fchdir` is also
restricted to a guest or passthrough directory.

### 6. Procfs magic-link containment

Passthrough paths now reject:

- any `..` component, because it can climb out of `/proc`, `/dev`, or `/sys`;
- under `/proc`, the `root` and `cwd` components, which jump back to the host
  namespace (`/proc/<pid>/root`, `/proc/<pid>/cwd`).

Normal queries such as `/proc/self/status` and `/dev/null` remain available.

## Final device results

The final loader was installed in the APK and executed from the current dynamic
native library directory. The old `files/libld39.so` was not used.

```text
01-functional-matrix.sh: #checks=31 fail=0
03-containment.sh:       #escapes=0
04-path-reliability.sh:  #checks=15 fail=0
faccessat probe:         #checks=8 fail=0
dirfd probe:             #checks=31 fail=0
host escape check:       PROC_ESCAPE_ABSENT
HOST_RC=0
```

The new dirfd checks include guest-relative `..`, `/proc` magic links, absolute
`/proc/self/root` and `/proc/../...`, safe `/proc/self/status`, plus
`openat`/`fstatat`/`faccessat`/`mkdirat`/`fchmodat`/`readlinkat` coverage.
They also recheck `AT_SYMLINK_FOLLOW`, `renameat2(RENAME_NOREPLACE)`, deep paths,
metadata pinning, and fd stability.

Static ELF alignment of the final loader:

```text
tools/loader/ld-embedded.so  p_align=0x10000 loads=3
  4096:ok 16384:ok 65536:ok
```

All three copies have the same hash:

```text
6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c  app/app/src/main/jniLibs/arm64-v8a/libld.so
6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c  tools/loader/ld-embedded.so
6c0796482ea3dbff508abd1a9f47267086819a9e1f38e73c642f89369cf78b9c  tools/probe-app/app/src/main/jniLibs/arm64-v8a/libld.so
```

## Remaining UNKNOWN / not promoted to PASS

- Deliberately malformed ELF branches and forced GOT-patch failure remain
  UNKNOWN.
- Concurrent rootfd close/reopen stress remains UNKNOWN.
- SIGSYS frame portability beyond this device remains UNKNOWN.
- 16 KiB and 64 KiB kernel execution has not been run on hardware. Static
  congruence is not a runtime claim.
- `libhuskapp.so` still fails the 64 KiB static congruence check; Android's
  supported 16 KiB target passes.
