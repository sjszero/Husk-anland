# Path metadata, relative dirfd, and loader bounds — evidence 65

Date: 2026-11-23
Device: RMX3357 / MT6893 / Android 13 / kernel 4.19.191+ / 4 KiB / arm64
Repository: `$REPO`
Test rootfs: `/data/user/0/com.linuxhub.husk/files/rootfs-full` (private copy)

This is the second pass over the third-party reliability audit. It records only
what was observed on the device. Inferrences are labelled explicitly.

## Final disposition

| Audit item | Current verdict |
|---|---|
| Final symlink TOCTOU | **Fixed for the tested metadata calls**. The final object is pinned with `O_PATH`; mutation uses `/proc/self/fd/N` or `AT_EMPTY_PATH`. A concurrent replacement can change which legitimate guest inode is selected, but the selected inode is the one mutated. |
| Relative dirfd containment | **Confirmed escape, fixed and tested**. `openat`/`fstatat`/`faccessat`/`mkdirat` with a real dirfd and `..` previously reached host paths. Calls from a dirfd inside the rootfs are now reconstructed as guest absolute paths and resolved through the root fd. |
| Temporary fd leak | **Fixed**, 1000 missing-path checks leave the fd count unchanged. |
| 512-byte symlink buffer | **Fixed** to `PATH_MAX` with truncation detection. |
| Overlength absolute path | **Fixed**; 5000-byte path returns promptly with failure. |
| GNU Hash / dynamic bounds | **Implemented**. Dynamic entries, GNU hash tables, symbol entries, relocation tables, string tables and GOT slots are range-checked against the object's `PT_LOAD` ranges. A deliberately malformed ELF has not yet been injected, so that adversarial case is **UNKNOWN**. |
| GOT completeness gate | **Implemented**; normal boot passes. Deliberate failure injection remains **UNKNOWN**. |
| ET_EXEC loader support | **Confirmed blocker, fixed and tested**. The main executable's `l_addr == 0` and low link-time `l_ld` are valid ET_EXEC semantics; Python 3.12 now runs. |
| Early-boot external calls | **Confirmed defect, fixed**. Returning `ObjMap` by value made Rust emit `memcpy@plt` before the shim's imports were resolved. The map is built in a static scratch object instead. |
| 4K/16K/64K page size | Loader static congruence is verified; actual 16K/64K execution remains **UNKNOWN**. App `libhuskapp.so` is still not 64K-congruent. |
| SIGSYS frame validation | Source validation is fixed; full signal-frame portability remains **UNKNOWN**. |
| Root fd identity | Identity is checked by device/inode/directory mode. Concurrent close/reopen stress in another thread remains **UNKNOWN**. |

## 1. Metadata acts on the pinned final inode

The old follow-final pathop passed a parent fd plus the final name to the
kernel. A final-component replacement between resolution and the mutation could
change the target. That surface is now closed for the tested calls:

- `runtime-core::resolve_final()` resolves the path under the root and returns
  an `O_PATH` fd for the final object, following symlinks only when requested.
- `chmod` uses `fchmodat(AT_FDCWD, "/proc/self/fd/N", ...)`.
- `chown` uses `fchownat(fd, "", ..., AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW)`.
- `utimensat` uses `/proc/self/fd/N`; no-follow retains the symlink inode.
- `statx` uses `/proc/self/fd/N` because this 4.19 kernel rejects
  `statx(AT_EMPTY_PATH)`.

### Raw kernel results on 4.19.191+

`tools/device-tests/fdops-kernel-probe.rs` is a static raw-syscall probe, so it
measures the kernel without the shim. The observed results were:

```text
fchmod-file-direct:              ret=-9  errno=9   EBADF
fchmodat-proc-file:              ret=0
fchown-file-empty:               ret=0
fchown-link-empty-nofollow:      ret=0
utimensat-file-empty:            ret=-22 errno=22 EINVAL
utimensat-link-empty-nofollow:   ret=-22 errno=22 EINVAL
utimensat-proc-file:             ret=0
statx-file-empty:                ret=-2  errno=2  ENOENT
statx-proc-file:                 ret=0
statx-proc-link-nofollow:        ret=0
fchmodat-proc-link:              ret=-95 errno=95 EOPNOTSUPP
utimensat-proc-link:             ret=0
utimensat-proc-link-nofollow:    ret=-13 errno=13 EACCES
```

**FACT:** on this kernel, `/proc/self/fd/N` works for file chmod, file/symlink
chown and file/stat timestamps. `AT_EMPTY_PATH` is not a general replacement on
4.19. The symlink-chmod `EOPNOTSUPP` result is a platform boundary, not silently
treated as success.

### Shim-level metadata test

The focused script now checks chmod, follow/no-follow timestamps and stat
semantics through the interposed libc entry points:

```text
PASS metadata-chmod
PASS metadata-utime-nofollow
PASS metadata-stat-nofollow
PASS metadata-stat-follow
PASS metadata-utime-follow
```

## 2. Relative dirfd was a real containment escape

`openat(dirfd, "relative", ...)` does not make the dirfd a namespace boundary.
A relative `..` chain can walk from a directory inside the guest root to the
host filesystem. The earlier resolver passed such paths straight to the kernel.

Before the fix, a guest Rust probe opened a real dirfd under `/tmp` and called
`fstatat` with seven `..` components:

```text
FSTATAT_ESCAPE_RC=0
```

That proved the host file was reachable. After the fix:

```text
OPENAT_REL_FILE=4
FSTATAT_REL_FILE=0
OPENAT_REL_ESCAPE=-1
FSTATAT_REL_ESCAPE=-1
FACCESSAT_REL_ESCAPE=-1
MKDIRAT_REL_ESCAPE=-1
OPENAT_REL_ABS_LINK_CREATE=0
READLINKAT_REL_LINK=15
READLINK_VALUE=/etc/os-release
OPENAT_REL_ABS_LINK=5
FCHMODAT_REL_FILE=0
FSTATAT_REL_FILE_MODE=0
FILE_MODE=385
UTIMENSAT_REL_FILE=0
```

The host check after the run was:

```text
HOST_ESCAPE_ABSENT
```

`tools/device-tests/dirfd-probe.rs` is the reproducible Rust probe. It is linked
against the guest's runtime `libc.so.6` so its calls go through the loader's GOT
interpose. The full focused script reports:

```text
#checks=12 fail=0
PASS relative-dirfd-contained
```

The first `#checks=8 fail=0` line belongs to the existing `faccessat` probe.

## 3. Loader bounds and ET_EXEC

### Bounded dynamic parsing

`tools/loader/path-shim/src/boot.rs` now builds the effective `PT_LOAD` ranges
for each link-map object before reading its dynamic section. The following are
rejected or skipped when they fall outside those ranges:

- `DT_STRTAB`, `DT_SYMTAB`, `DT_GNU_HASH`, `DT_RELA`, `DT_JMPREL` and their sizes;
- GNU hash buckets, bloom words and chain entries;
- symbol table entries and string offsets;
- relocation entries and GOT slots.

For ET_EXEC main programs, `l_addr == 0` and low link-time virtual addresses are
valid. The map is built from the kernel's `AT_PHDR`/`AT_PHNUM`, and the low
addresses are checked against that map instead of being rejected by magnitude.

Before this correction, Python (ET_EXEC) failed at boot with:

```text
nr-boot: GOT patch incomplete: 0x1
RC=71
```

After the correction:

```text
3.12.3
2
RC=0
```

### Self-bootstrap constraint

The first bounds implementation returned `ObjMap` by value. The compiler emitted
`memcpy@plt` inside the early hook, before the shim's own imports were resolved,
and bash crashed with SIGSEGV. The map is now filled in place in a static
scratch object. A symbol check showed no generated `memcpy@plt` or `memset@plt`
in the early path. This is a hard boot invariant, not a style preference.

## 4. Final regression on the release loader

Final loader sha256:

```text
50d3f56f8fda86ae19138ebfebda34c68711a37c8658ffa744034873974bf96d  tools/loader/ld-embedded.so
50d3f56f8fda86ae19138ebfebda34c68711a37c8658ffa744034873974bf96d  app/app/src/main/jniLibs/arm64-v8a/libld.so
```

Results on the final loader:

```text
01-functional-matrix.sh: #checks=31 fail=0
03-containment.sh:       #escapes=0
04-path-reliability.sh:  #checks=12 fail=0
Python 3.12.3:           RC=0
dirfd probe:              #checks=18 fail=0
host escape check:        HOST_ESCAPE_ABSENT
```

Static page-size check:

```text
libhuskapp.so  p_align=0x4000  loads=3  4096:ok 16384:ok 65536:FAIL
libld.so       p_align=0x10000 loads=3  4096:ok 16384:ok 65536:ok
ld-embedded.so p_align=0x10000 loads=3  4096:ok 16384:ok 65536:ok
```

## 5. Remaining UNKNOWN / not promoted to PASS

- No deliberately malformed ELF was injected to exercise every bounds-failure
  branch. The bounds are present and normal binaries pass, but adversarial input
  coverage is not complete.
- The GOT-failure exit branch has not been deliberately triggered.
- Root fd close/reopen was tested single-threaded; concurrent close/open stress
  is still **UNKNOWN**.
- SIGSYS frame portability beyond this device remains **UNKNOWN**.
- 16 KiB and 64 KiB execution has not been run on hardware. Static congruence is
  not a runtime claim.
- Metadata on symlinks has kernel-specific limits: `fchmodat` on the symlink
  itself returns `EOPNOTSUPP` on this 4.19 device.
