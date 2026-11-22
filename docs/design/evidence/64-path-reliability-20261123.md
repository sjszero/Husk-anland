# Path resolver and loader reliability — evidence 64

Date: 2026-11-23
Device: RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
Repository: `/Users/l/Documents/Codex/2026-08-10/husk`
Test tree: `/data/data/com.linuxhub.husk/files/rootfs-full` (private copy)

This record verifies the third-party reliability audit against the current
source and device. It does not treat the audit's proposed fixes as correct by
default.

## Audit disposition

| Audit item | Current verdict |
|---|---|
| Final symlink TOCTOU | **Confirmed risk; fixed path**. Final open/stat/access use an `O_NOFOLLOW` object and manual guest-root symlink resolution. Concurrent replacement may still alter the chosen target, but it cannot escape the guest root. |
| Temporary fd leak on errors | **Confirmed defect; fixed and measured** with the 1000-call test. |
| 512-byte symlink buffer | **Confirmed defect; fixed** to `PATH_MAX` with truncation detection. |
| Overlength absolute path | **Confirmed defect; fixed**; panic no longer spins. |
| GNU Hash bounds | **Partially fixed**. Bounded counters were added; complete mapping-range validation is still missing. |
| GOT completeness gate | **Implemented, branch not injected**. Normal boot passes; forced-failure branch is UNKNOWN. |
| 4K/16K/64K build chain | **Overclaim confirmed in documentation**. Loader static congruence passes; App 64K fails and no 16K/64K runtime exists. |
| `check-elf` alignment check | **Audit is partly incorrect**. `max_align` is reported; `p_align < page` is a warning because PT_LOAD congruence is the decisive exec-time requirement. The real issue was elsewhere: the project claimed whole-product support from a loader-only static check. |
| SIGSYS frame validation | **Source validation fixed** (`si_code` and AArch64 arch required); full signal-frame portability remains UNKNOWN. |
| rootfd identity | **Confirmed weakness; fixed** with `fstat` dev/inode/directory checks and reopen on mismatch. Multi-thread close/reopen stress remains UNKNOWN. |

## Confirmed root causes and fixes

### 1. Temporary fds leaked on failed path resolution

Before the RAII change, 1000 `test -e` calls on non-existent paths produced:

```text
FD_BEFORE=5 FD_AFTER=1005
```

After `Walk` owns and closes every temporary fd on both success and error:

```text
FD_BEFORE=5 FD_AFTER=5
```

Source: `runtime/crates/runtime-core/src/lib.rs`.

### 2. Absolute paths longer than `PATH_MAX`

`abs_guest()` now rejects `s.len() >= PATH_MAX` before copying. A 5000-byte
absolute path returns `ENAMETOOLONG` quickly instead of entering the old panic
handler, which spun forever. The focused shell check reports `LONG_RC=1`; the
host-side `timeout` does not fire.

### 3. Wrong AArch64 `O_NOFOLLOW` / `O_DIRECTORY` values

This was the most important finding, and was not explicit in the audit. The
resolver used the asm-generic values rather than the AArch64 UAPI values:

```text
O_NOFOLLOW  wrong 0o400000  correct 0o100000
O_DIRECTORY wrong 0o200000  correct 0o40000
```

With the wrong values, a final absolute symlink was opened by the kernel and
resolved against the host root. Reproduction before the fix:

```text
readlink /tmp/husk-link -> /etc/os-release
cat /tmp/husk-link      -> No such file or directory
```

After the fix, an absolute symlink is resolved manually under the root fd:

```text
PRETTY_NAME="Ubuntu 24.04.4 LTS"
```

A focused C test also confirms the no-follow contract:

```text
NOFOLLOW_ERRNO=40       # ELOOP
FOLLOW_READ=31
```

## Root fd identity, not just fd number

The previous check only used `fcntl(F_GETFD)`, which cannot distinguish a
reused descriptor. The current check uses `fstat` and compares device, inode,
and directory mode. The device test:

1. raw-closes fd 1024;
2. duplicates stdin into fd 1024 with `F_DUPFD`;
3. calls `stat("/etc/os-release")`.

The shim detected the mismatch, reopened the root at a higher fd, and kept
working:

```text
RAW_CLOSE=0
RAW_DUPFD=1024
STAT=0 errno=0 size=400
root high rc=1025
```

## SIGSYS source validation

The handler now requires both `si_code == SYS_SECCOMP` and
`si_arch == AUDIT_ARCH_AARCH64` before it rewrites a saved frame. A manually
injected, ordinary `SIGSYS` is logged and terminated with the original fatal
semantics instead of being emulated:

```text
sigsys|unexpected source code=-6|arch=0
GUEST_RC=159
```

The normal seccomp fallback path remains covered by evidence 20/32; no claim is
made that every Android vendor policy uses the same trap set.

## GOT completeness gate

`patch_object()` now returns both patched and failed slot counts. `nr_boot()`
returns a failure code if the link-map chain is truncated or any required GOT
slot cannot be made writable; the assembly `_dl_init` hook then calls
`exit_group` instead of entering a partially interposed guest. Normal boot is
verified by the 31-item matrix. A deliberate failure-injection test has not yet
been performed, so that specific branch is **UNKNOWN**, not PASS.

## Regression results

Functional matrix:

```text
#checks=31 fail=0
```

Containment:

```text
#escapes=0
```

Focused reliability script:

```text
PASS fd-stable
PASS long-path-rejected
PASS symlink-follow
PASS escape-abs
PASS escape-dots
PASS faccessat-nofollow
#checks=6 fail=0
```

## faccessat no-follow semantics

The third-party audit correctly identified that the legacy AArch64
`faccessat(48)` syscall ignores its fourth argument, so the old shim still
followed a dangling final symlink even when `AT_SYMLINK_NOFOLLOW` was set.
This kernel (4.19) has no `faccessat2(439)`, so the fix does not depend on that
syscall.

`runtime-core` now resolves the final component to an `O_PATH|O_NOFOLLOW` fd.
A symlink fd is assessed as the link itself; a non-symlink is checked through
`/proc/self/fd/<n>`, which pins the inode and avoids a final-component
replacement race. The path shim uses the same helper for relative dirfds and
handles `AT_EMPTY_PATH`.

The Rust guest probe `tools/device-tests/path-probe.rs`, run through the
loader's GOT interpose, reports:

```text
PASS dangling-follow got=false want=false
PASS dangling-nofollow got=true want=true
PASS dangling-nofollow-r got=true want=true
PASS regular-nofollow-r got=true want=true
PASS escape-follow got=false want=false
PASS escape-nofollow got=true want=true
PASS rel-follow got=false want=false
PASS rel-nofollow got=true want=true
#checks=8 fail=0
```

`faccessat2` was deliberately not used as the primary implementation: on this
ROM its optional syscall path is policy-sensitive and is not available on the
4.19 kernel. The implementation is verified on the current device; other
kernels still need the same probe.

## Findings not promoted to PASS

- Full mapping-range validation for `link_map` and GNU Hash parsing is still
  not implemented; the bounded counters prevent unbounded walks but do not
  prove every dynamic pointer is inside a mapped object.
- A deliberate GOT-failure injection has not been run. The code exits before
  guest entry on a reported failure, but that branch is still **UNKNOWN**.
- Page-size independence remains a static/congruence claim for the loader, not
  a 16 KiB or 64 KiB runtime claim. `libhuskapp.so` still fails the 64 KiB
  congruence check.

## Page size: distinction between static congruence and a runtime claim

`nr-tools check-elf` on the current loader and app libraries:

```text
libhuskapp.so  p_align=0x4000  loads=3  4096:ok 16384:ok 65536:FAIL
libld.so       p_align=0x10000 loads=3  4096:ok 16384:ok 65536:ok
ld-embedded.so p_align=0x10000 loads=3  4096:ok 16384:ok 65536:ok
```

**FACT:** the loader's added `PT_LOAD` is congruent for 4 KiB, 16 KiB, and
64 KiB in this build. **FACT:** `libhuskapp.so` is not 64 KiB-congruent.
**UNKNOWN:** this 4 KiB device cannot prove a 16 KiB or 64 KiB kernel boot; no
runtime PASS is claimed. The build script still defaults to 4 KiB alignment,
so a separate multi-page build target remains required for a universal claim.
