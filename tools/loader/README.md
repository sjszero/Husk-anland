# Guest loader with the path shim inside it

Turns a stock glibc `ld.so` into a "native runtime" loader that

* survives Android's app seccomp policy, and
* carries the Rust path shim as part of itself, so no `LD_PRELOAD` is needed.

No root, no chroot, no namespaces, no ptrace, no seccomp supervisor, no VM.

## Why the shim has to be inside the loader

Inside `untrusted_app`, `set_robust_list(99)` / `rseq(293)` are `SECCOMP_RET_TRAP`
and glibc calls them from `__pthread_initialize_minimal_internal` *inside* the
loader, before any preload constructor could install anything. Two consequences:

1. those `svc` sites are neutralised at build time (`nr-nop-syscall.py`);
2. the first point at which user space can safely take over is `_dl_init`, which
   runs after every object is relocated and before any constructor. That is where
   the embedded shim installs itself.

## Pipeline

```
path-shim (Rust, no_std, zero DT_NEEDED)
  └─ cargo build -C link-arg=-z -C link-arg=max-page-size=4096   → compact spans
     (the segment embed adds is 64K-aligned: SEG_ALIGN in nr-tools)
init-hook.S                                                    → 288-byte hook blob
nr-tools scan-syscalls patch ld.so --version 2.35 ..           → one `svc` -> `nop`
nr-tools embed shim.so ld.so hook.bin ld-embedded.so           → the product
```

The ELF surgery is `runtime/crates/nr-tools` (Rust, no dependencies) and it is the
only implementation. The Python scripts it replaces were kept as an oracle until the
port reproduced them byte for byte; evidence/22 records that comparison (identical
stdout, identical sha256) and the two porting bugs it caught. The oracle was then
removed from the tree - recover it with `git show v0.1.0:tools/loader/nr-embed.py`.

The same crate also carries `nr-tools provision`, which replaces the sh+python
toolchain provisioning script (`../device-tests/provision-toolchain.sh`
is now a thin wrapper over it; evidence/23).

## The `svc` sites are found, not remembered

`nr-tools scan-syscalls` does not carry offsets. It reads the image's
`.gnu.version_d` (and refuses another glibc), finds every `svc #0`, walks
*backwards* for the `mov x8,#NR` that selects the syscall (glibc sets the
argument registers after x8, so they are several instructions apart), keeps only
the optional accelerators, and requires the instruction after the `svc` to
*write* x0 without reading it - proof that the caller ignores the result, which is
the only reason a `nop` is meaningful. The site count must match `--expect`.

All four of those are assertions, not advice: wrong version, wrong count, a site
that vanished, or a successor that reads x0 all print `REFUSING` and exit
non-zero without writing anything (`docs/design/evidence/15`).

`rseq` sites are located but never patched - their callers do read the result -
so the runtime keeps glibc from trying rseq with its own documented switch,
`GLIBC_TUNABLES=glibc.pthread.rseq=0`.

`clone3` is different: glibc 2.39 inspects its result and falls back to the legacy
`clone(2)` path only when it receives `ENOSYS`. The scanner therefore replaces
the `clone3` `svc` with `movn x0,#37` after asserting that the next instruction
checks x0. This keeps the guest process out of Android's `SECCOMP_RET_TRAP`
while preserving glibc's normal fork/thread bookkeeping.

`./build-embedded-loader.sh` runs all of it and drops the result into the App's
`jniLibs` (a build output, not tracked; `make apk` packages it). With `LIBC=<pristine libc.so.6>` it also emits `libc-nop.so.6`,
which is the file that must be installed into the guest rootfs (the device copy
is byte-identical to it). Output is deterministic (`nr-tools embed` twice → identical bytes).

`nr-tools embed` asserts, and refuses to emit an image if any of it fails: the loader
entry address, that `0x1aa74` really is a `bl` to `_dl_init`, the phdr shape,
offset/vaddr congruence, that the space it claims is empty, and that the hook is
inside branch range. Patching a different glibc build therefore fails loudly
instead of producing a subtly broken loader.

## What `nr-tools embed` builds

* one extra `R|E` `PT_LOAD` at vaddr `0x42000` / file `0x32000`, taken from the
  `PT_NOTE` program header (the loader has no spare slot and no free segment
  space). The build-id note is debug-only; nothing at run time reads it.
* the shim's four 4K segments copied as one block, so every internal offset is
  preserved. Its `R_AARCH64_RELATIVE` targets are written as *link-time* values
  and the hook adds the load bias at startup — the bias cannot be known earlier.
* three tables after the shim: imports (name → our GOT slot), interpose
  (name → our function), and the self-relocation slots.
* the hook at `0x52000`: mprotect its own writable window, call `nr_boot`, then
  branch to the real `_dl_init`. `_start`'s `bl _dl_init` is repointed at it. `e_entry` is deliberately left
  alone: glibc decides "program or interpreter" by comparing auxv `AT_ENTRY` with
  `&_start`, and repointing `e_entry` makes it abort in `rtld_setup_main_map`.

## What runs at startup (`nr_boot`, in `crates/path-shim/src/boot.rs`)

1. add the load bias to our own `R_AARCH64_RELATIVE` slots;
2. resolve our 12 imports by walking the `link_map` chain with a built-in
   GNU-hash lookup (calling IFUNC resolvers, exactly as the loader does);
3. for every object in the chain, walk `.rela.dyn`/`.rela.plt` and rewrite the
   GOT slot of each of our 45 names. Read-only RELRO pages are mprotected
   writable for the write. This *is* LD_PRELOAD's symbol interposition, done
   once, by hand, before constructors.

`boot.rs` references no external symbol except the imports resolved in step 2.

Diagnostics: `NR_BOOT_DEBUG=1` makes the loader print, per process,
`nr-boot: imports=0xc objects=0x5 slots=0x10` to fd 2. Building the hook with
`-DNR_FAULT=1` installs a fault reporter that dumps `si_addr` and the saved
register frame — this ROM writes no tombstone for these child processes, so a
fault is otherwise invisible.

## Status

* WORKING (measured, `docs/design/evidence/27` and `32`): RMX3357 / Android 13 /
  `untrusted_app`, Ubuntu 24.04 glibc 2.39 — guest `bash` + coreutils matrix green,
  no `LD_PRELOAD`, 600 s soak with zero zombies.
* The `svc` sites are located by pattern with a glibc version self-check
  (`nr-tools scan-syscalls`); updating the count expectations is a deliberate human step.
* A 16K-page kernel is untested. The stub blob is placed at a 4K-aligned
  `NR_STUB_VADDR`; on a 16K kernel that placement (and every `mprotect` length in
  the shim) must be built from `AT_PAGESZ` instead of a literal 4096. See
  `docs/design/page-size.md`.
