#!/bin/sh
# Guest-side installer: point `su` and `runuser` at husk-su.
#
# util-linux' su is dynamically linked, so it does pass through the shim - but
# it cannot change identity: its change of identity runs initgroups() inside
# libc, whose setgroups() never reaches the interposer (the real syscall is
# refused, uid != 0), and its PAM stack answers "su: System error". husk-su
# needs no privilege at all: identity inside this runtime is NR_FAKE_UID /
# NR_FAKE_GID, which husk-su sets before exec'ing the shell (evidence 139).
#
# dpkg-divert, not overwrite: these files belong to `util-linux`, so a plain
# replace is reverted by the next upgrade and makes `dpkg -V` complain forever.
set -e
BIN=/usr/sbin/husk-su
[ -x "$BIN" ] || { echo "install-su: $BIN missing"; exit 1; }
for t in /usr/bin/su /usr/sbin/runuser; do
    [ -e "$t" ] || continue
    if [ -f "$t" ] && head -n 1 "$t" 2>/dev/null | grep -q 'husk-su'; then
        continue  # already our wrapper
    fi
    dpkg-divert --local --rename --add "$t" >/dev/null 2>&1 || true
    printf '#!/bin/sh\nexec %s "$@"\n' "$BIN" > "$t.husk.new"
    chmod 755 "$t.husk.new"
    mv "$t.husk.new" "$t"
done
echo "su: installed"
