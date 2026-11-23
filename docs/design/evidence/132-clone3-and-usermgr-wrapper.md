# Evidence 132: clone3 ENOSYS fallback and account-tool wrappers

Date: 2026-11-24
Device: RMX3357, Android 13, kernel 4.19.191, arm64, 4 KiB pages
Domain: untrusted_app / `com.linuxhub.myth.husk`

## FACT: glibc 2.39 calls clone3 on Ubuntu 24.04

Before patching, the guest libc issued syscall 435:

```text
ebb20  mov x8, #0x1b3
ebb24  svc #0
ebb28  cmp x0, #0
```

Android seccomp traps `clone3` in this domain. A child could die with
`SIGSYS`, and `dpkg-deb` failed while decompressing a control archive:

```text
HUSK_PROBE ctrl-size=0
HUSK_PROBE ctrl-sha=e3b0c442...
dpkg-deb: <decompress> subprocess was killed by signal (Bad system call)
```

`glibc.pthread.use_clone3=0` is not a real glibc 2.39 tunable and was not used.
The libc patch changes the syscall instruction to return `-ENOSYS` while
preserving the following `cmp x0, #0`, so glibc takes its native legacy
`clone(2)` fallback:

```text
mov x0, #-38       ; -ENOSYS
cmp x0, #0
```

The scanner in `runtime/crates/nr-tools` only permits this rewrite when the
next instruction is exactly `cmp x0, #0`, and the loader build asserts exactly
one patched `clone3` site.

After patching:

```text
ctrl-tarfile_rc=0 size=10240
control_rc=0
control 477
md5sums 464
postinst 449
prerm 132
triggers 42
```

The same result was reproduced after removing temporary path-shim tracing, so
the fix does not depend on the diagnostic build.

Artifacts:

```text
patched libc sha256:
32d4d3f9501a2c717cf2246a2df1a898f1361b49e0576614461d70fc94774b56
clean embedded loader sha256:
d727fd3c6e3aacaa41ea37c3578f50c39bfdfeadfc4a9c059d9281df6b4a8aab
```

## FACT: account tools must be wrappers, not symlinks

The first full 2.0 package run reached `dbus-system-bus-common`, then failed:

```text
usage: husk-usermgr <useradd|groupadd|...>
fatal: `/sbin/groupadd -g 101 messagebus' returned error code 2.
dpkg: error processing package dbus-system-bus-common (--install):
 installed dbus-system-bus-common package post-installation script
 subprocess returned error exit status 82
```

Root cause: the runtime re-execs the resolved program path. A symlink such as
`/usr/sbin/groupadd -> husk-usermgr` therefore makes `argv[0]` become
`husk-usermgr`, losing the invoked account-tool name.

The working form is a two-line wrapper:

```sh
#!/bin/sh
exec /usr/sbin/husk-usermgr groupadd "$@"
```

After installing the wrappers, this was verified in the guest:

```text
groupadd_rc=0
messagebus:x:101:
```

This mirrors `tools/guest-config/install-usermgr.sh`; the LinuxHub 2.0 test
application was changed to install wrappers and was re-run from the same
rootfs. The package queue advanced past `hicolor-icon-theme` and
`dbus-system-bus-common`.

## INFERENCE

The two fixes address different mandatory paths:

- `clone3` -> ENOSYS prevents seccomp from killing every glibc-created child.
- wrappers preserve the account tool's invocation name when `execve` resolves
  a symlink.

Neither fix expands the syscall interception surface.
