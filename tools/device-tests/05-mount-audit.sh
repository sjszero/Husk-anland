#!/bin/bash
# Mount semantics audit: the eight findings of 2026-11-23, each as a check.
#
# Run inside the guest (see tools/device-tests/gexec.sh):
#   sh gexec.sh rootfs-full usr/bin/bash tmp/05-mount-audit.sh
#
# Every step that must observe a mount runs in its own process: the table is
# loaded once per process (documented limitation), so a shell that mounted
# something earlier in the same script would not see it.
#
# bash, not sh: B-03 truncates 127-byte paths with `${t:0:127}`, which dash
# rejects with "Bad substitution". The shebang said /bin/sh and the file was
# always started as `sh 05-mount-audit.sh`, so the last five checks had never
# run to completion (evidence 294 §6).
set -u
RUNTIME_PROBE=${RUNTIME_PROBE:-/root/runtime-probe}
PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
want_erofs() { case "$1" in *"Read-only file system"*|*"Errno 30"*) ok "$2";; *) bad "$2 (got: $1)";; esac; }
probe_checks() {
  if out=$("$RUNTIME_PROBE" mount-audit "$@" 2>&1); then
    printf '%s\n' "$out"
    n=$(printf '%s\n' "$out" | grep -c '^PASS ')
    PASS=$((PASS+n))
  else
    printf '%s\n' "$out"
    FAIL=$((FAIL+1))
  fi
}

echo "== B-01 read-only mounts refuse every write path =="
sh -c 'rm -rf /mnt/rosrc /mnt/ro; mkdir -p /mnt/rosrc /mnt/ro; echo data > /mnt/rosrc/f'
mount --bind /mnt/rosrc /mnt/ro && mount -o remount,ro,bind /mnt/ro \
  || probe_checks mount-bind-ro
sh -c 'grep -q " /mnt/ro .*ro" /proc/self/mounts' && ok "table marks the bind ro" || bad "/proc/self/mounts does not show ro"
want_erofs "$(sh -c 'echo x > /mnt/ro/f' 2>&1)" "absolute path write"
probe_checks mount-write
[ "$(sh -c 'cat /mnt/ro/f')" = "data" ] && ok "content untouched" || bad "content changed"
# Metadata writes do not go through open(), so they need their own check.
MODE0=$(sh -c 'stat -c %a /mnt/ro/f')
[ "$(sh -c 'stat -c %a /mnt/ro/f')" = "$MODE0" ] && ok "mode untouched" || bad "mode changed"
umount /mnt/ro

echo "== B-02 a new tmpfs never inherits a dead one's files =="
sh -c 'rm -rf /mnt/a /mnt/b; mkdir -p /mnt/a /mnt/b'
mount -t tmpfs tmpfs /mnt/a && sh -c 'echo secret > /mnt/a/secret.txt'
umount /mnt/a && mount -t tmpfs tmpfs /mnt/b
[ -z "$(sh -c 'ls -A /mnt/b')" ] && ok "second tmpfs starts empty" || bad "second tmpfs shows leftover files"
umount /mnt/b
[ -z "$(ls -d /run/husk/t* 2>/dev/null)" ] && ok "stores are removed on umount" || bad "store directories left behind"

echo "== B-03 a full table of longest paths survives a round trip (2080 bytes) =="
rm -f /run/husk/mounts
i=1
while [ $i -le 8 ]; do
  t="/mnt/t$i$(printf 'a%.0s' $(seq 1 100))$(printf 'a%.0s' $(seq 1 20))"; t=${t:0:127}
  s="/tmp/s$i$(printf 'b%.0s' $(seq 1 100))$(printf 'b%.0s' $(seq 1 20))"; s=${s:0:127}
  sh -c "rm -rf '$t' '$s'; mkdir -p '$t' '$s'; echo v$i > '$s/f'"
  mount --bind "$s" "$t" || bad "bind $i"
  i=$((i+1))
done
[ "$(wc -l < /run/husk/mounts)" -eq 8 ] && ok "8 entries stored" || bad "table lost entries"
all=1; i=1
while [ $i -le 8 ]; do
  t="/mnt/t$i$(printf 'a%.0s' $(seq 1 100))$(printf 'a%.0s' $(seq 1 20))"; t=${t:0:127}
  [ "$(sh -c "cat '$t/f' 2>&1")" = "v$i" ] || all=0
  i=$((i+1))
done
[ $all -eq 1 ] && ok "all 8 resolve after a reload" || bad "an entry did not resolve"
i=1
while [ $i -le 8 ]; do
  t="/mnt/t$i$(printf 'a%.0s' $(seq 1 100))$(printf 'a%.0s' $(seq 1 20))"; t=${t:0:127}
  umount "$t" 2>/dev/null; i=$((i+1))
done

echo "== B-04 equal-length chains keep hopping =="
sh -c 'rm -rf /mnt/end /mnt/mid /mnt/top; mkdir -p /mnt/end /mnt/mid /mnt/top; echo chained > /mnt/end/f'
mount --bind /mnt/end /mnt/mid && mount --bind /mnt/mid /mnt/top
[ "$(sh -c 'cat /mnt/top/f 2>&1')" = "chained" ] && ok "two-hop chain resolves" || bad "chain stopped after one hop"
umount /mnt/top; umount /mnt/mid

echo "== B-05/B-06 resolution is safe under concurrency and mount churn =="
probe_checks mount-race

echo "== B-07 xattr on the pinned object =="
sh -c 'rm -f /tmp/xf; echo x > /tmp/xf'
probe_checks xattr /tmp/xf

echo "== B-08 link to a symlink with follow semantics =="
sh -c 'rm -f /tmp/l1 /tmp/l2 /tmp/l3; echo target > /tmp/l3; ln -s /tmp/l3 /tmp/l1'
if ln -L /tmp/l1 /tmp/l2 2>/dev/null && [ "$(sh -c 'cat /tmp/l2')" = "target" ]; then
  echo "  PASS  ln -L"
else
  echo "  FAIL  ln -L"
fi

echo
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
