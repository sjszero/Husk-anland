#!/bin/sh
# Guest-side installer: point the account tools at husk-usermgr.
#
# dpkg-divert, not overwrite: these files belong to `passwd`/`shadow`, so a plain
# replace is reverted by the next upgrade and makes `dpkg -V` complain forever.
# A local diversion renames the original to *.distrib and records it.
#
# The replacement is a two-line wrapper rather than a symlink. Reason (measured,
# evidence 111): the runtime re-enters the loader with the *resolved* program
# path, so a symlink makes the process see argv[0]==.../husk-usermgr and lose
# which tool it was invoked as. The wrapper passes the name explicitly.
set -e
BIN=/usr/sbin/husk-usermgr
[ -x "$BIN" ] || { echo "install-usermgr: $BIN missing"; exit 1; }
for t in useradd groupadd usermod userdel groupdel chpasswd passwd chfn chsh; do
    src=$(command -v "$t" 2>/dev/null || true)
    [ -n "$src" ] || continue
    if [ -f "$src" ] && [ ! -L "$src" ] && head -n 1 "$src" 2>/dev/null | grep -q '^#!/bin/sh$'; then
        :  # already our wrapper
    else
        dpkg-divert --local --rename --add "$src" >/dev/null 2>&1 || true
    fi
    printf '#!/bin/sh\nexec %s %s "$@"\n' "$BIN" "$t" > "$src.husk.new"
    chmod 755 "$src.husk.new"
    mv "$src.husk.new" "$src"
done
echo "usermgr: installed"
