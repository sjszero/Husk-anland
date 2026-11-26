#!/bin/bash
# A-20: unmounting has to work from the tools users actually run.
#
# Before the fix, `umount <dir>` answered
#   umount: /mnt/t: must be superuser to unmount.   (rc=32)
# because libmount calls the *legacy* one-argument umount() when no flags are
# given, and glibc expands that to an inline umount2(2) syscall - it never calls
# the `umount2` symbol, so nothing interposed it and the kernel answered EPERM.
# `umount -l` (flags set, real umount2 call) worked, which is what made the
# distinction visible. The mount table therefore only ever grew: 240 store
# directories under /run/husk and a full 64-entry table on the test device.
#
# Run inside the guest:  sh husk-exec.sh <rootfs> <loader> usr/bin/bash /tmp/16-umount-legacy.sh
set -u
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }
eq()  { [ "$2" = "$3" ] && ok "$1 ($3)" || bad "$1 (want '$2' got '$3')"; }
T=/run/husk/mounts
lines() { wc -l < "$T" 2>/dev/null | tr -d ' ' || echo 0; }
[ -f "$T" ] || : > "$T"

mkdir -p /mnt/u1 /mnt/u2

echo "== umount by mount point (the libmount no-flag path) =="
mount -t tmpfs tmpfs /mnt/u1 >/dev/null 2>&1; eq "mount rc" 0 $?
src=$(awk -F'\t' '$3=="/mnt/u1"{print $4}' "$T")
eq "table has the entry" yes "$([ -n "$src" ] && echo yes || echo no)"
eq "store dir exists" yes "$([ -n "$src" ] && [ -d "$src" ] && echo yes || echo no)"
out=$(umount /mnt/u1 2>&1); rc=$?
eq "umount rc" 0 $rc
[ $rc -eq 0 ] || echo "        $out"
eq "entry removed" 0 "$(lines)"
eq "store dir removed" no "$([ -n "$src" ] && [ -d "$src" ] && echo yes || echo no)"

echo "== umount by source path (legacy one-argument semantics) =="
mount -t tmpfs tmpfs /mnt/u2 >/dev/null 2>&1; eq "mount rc" 0 $?
src=$(awk -F'\t' '$3=="/mnt/u2"{print $4}' "$T")
eq "umount <source> rc" 0 "$(umount "$src" >/dev/null 2>&1; echo $?)"
eq "entry removed" 0 "$(lines)"

echo "== a path that is not mounted still fails, and changes nothing =="
mount -t tmpfs tmpfs /mnt/u2 >/dev/null 2>&1
before=$(lines)
out=$(umount /mnt/u1 2>&1); rc=$?
eq "not-mounted rc is non-zero" yes "$([ $rc -ne 0 ] && echo yes || echo no)"
eq "table unchanged" "$before" "$(lines)"
echo "        $out"
umount /mnt/u2 >/dev/null 2>&1

echo "== nothing left behind =="
eq "table empty" 0 "$(lines)"
rmdir /mnt/u1 /mnt/u2 2>/dev/null

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
