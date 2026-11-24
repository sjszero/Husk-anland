# Contributing

Husk runs a glibc userland inside an ordinary Android app process: no root, no
bootloader unlock, no hardware virtualization, and no `ptrace`. Every patch has
to keep that shape, because the reason the project exists is the performance that
shape buys.

## Glossary

These five words are used with a fixed meaning everywhere in the repo; if a
document disagrees with this list, the document is wrong.

| Term | Meaning |
|---|---|
| **loader** | `libld.so` — the guest's glibc `ld.so` with the shim embedded in an extra `PT_LOAD`. It maps the guest program and re-enters itself for every `execve`. |
| **path shim** | The Rust (`no_std`) module inside the loader that interposes libc symbols which take a path (`open`, `stat`, `execve`, `statvfs`, ...) and resolves them against the guest root. |
| **guest** | The process tree started by the loader. It sees the rootfs as `/`, uid 0, and no Android paths. |
| **rootfs** | The distro tree (`files/linux/distributions/ubuntu-24.04/rootfs` on device). Shared by every runner in the benchmarks, so the runner stays the only variable. |
| **runner** | What starts the guest: `husk` (loader + shim), `proot`, `proot-distro`. `tools/bench/` treats them as interchangeable adapters. |

## Ground rules

1. **No third-party code in the tree.** Dependencies are resolved by Cargo at
   build time; artifacts that must ship (`ld-embedded.so`, `libc-nop.so.6`, the
   guest helpers) are built from sources in this repo and committed only where
   `docs/ENGINEERING.md` says so.
2. **No root, no `ptrace`, no namespaces.** A patch that needs any of them is a
   different project; open an issue first.
3. **Claims need a measurement.** A PR that says "faster" needs a line in
   `docs/design/evidence/` with the command and the output, including the cases
   where Husk lost.
4. **Absolute paths in guest code, relative paths on the command line.** The
   loader is invoked as `libld.so --library-path ... usr/bin/bash` — the program
   path is relative on purpose (see `docs/STATUS.md`).
5. **Never read or write `com.linuxhub.myth`** except through the documented
   adb/`run-as` path for the app under test.

## Build

```sh
# type-check the parts that are pure Rust, for the target they run on
make check

# formatting gate (what CI runs)
make lint

# rebuild the loader — needs NDK (clang + llvm-objcopy) on PATH via $NDK
make loader
```

`cargo test` builds aarch64 inline assembly: it only runs on an arm64 Linux
host. On macOS or x86_64 the tests fail to compile, which is a platform
limitation, not a failing patch. Use `make check` there.

## Style

- Conventional Commits: `feat|fix|docs|perf|refactor|test|build|ci|chore`. The
  body says what was measured, not how hard it was.
- Rust: `cargo fmt` and clippy are expected to stay quiet about *errors*;
  warnings in `path-shim` are old and tracked separately.
- Shell: POSIX `sh`, `set -eu`, no bashisms in `tools/`. Verify with `sh -n`.
- Documents: `#`/`##`/`###` only, ≤100 columns, code fences carry a language.

## Tests that matter

| Layer | How it is checked |
|---|---|
| shim / ELF surgery | `cargo test --target aarch64-unknown-linux-gnu -p runtime-core -p nr-tools` on an arm64 Linux host |
| guest basics | `tools/device-tests/14-gate-a-acceptance.sh` (36 items: bash, locale, user management, dpkg, apt, host-path leaks, seccomp traps) |
| desktop | start GNOME, then 30 minutes idle: RSS, threads, fds and zombie counts must not grow (`docs/design/evidence/153-*`) |
| performance | `tools/bench/run-all.sh <runner>` — same rootfs, same shell, one runner changed |

## Reporting a bug

Use the issue template. Include the device properties it asks for, the exact
command, and the exact output. If the runtime printed a `SIGSYS` trap, say which
number — `docs/LIMITATIONS.md` explains which ones are expected.
