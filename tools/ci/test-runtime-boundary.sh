#!/bin/sh
# Negative/positive test for audit-runtime-boundary.sh. The gate must reject a
# bundle carrying an X server and accept the same bundle without host UI assets.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-runtime-boundary-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
root="$tmp/root"
mkdir -p "$root/guest-config"
for f in libld.so libc.so.6 husk-ldconfig husk-usermgr husk-su husk-symlinks \
         husk-logd audio-setup audio-relay audio-capture audio-probe \
         install-husk.sh husk-session.sh guest-config/husk-gpu-defaults.sh \
         guest-config/mksys-net.sh; do
    : > "$root/$f"
done
tar czf "$tmp/valid.tar.gz" -C "$root" .

if ! sh tools/ci/audit-runtime-boundary.sh "$tmp/valid.tar.gz" > "$tmp/valid.out" 2>&1; then
    cat "$tmp/valid.out" >&2
    echo "runtime boundary test: valid bundle was rejected" >&2
    exit 1
fi

cp -R "$root" "$tmp/bad"
: > "$tmp/bad/libXlorie.so"
tar czf "$tmp/bad.tar.gz" -C "$tmp/bad" .
if sh tools/ci/audit-runtime-boundary.sh "$tmp/bad.tar.gz" > "$tmp/bad.out" 2>&1; then
    cat "$tmp/bad.out" >&2
    echo "runtime boundary test: embedded X server was accepted" >&2
    exit 1
fi

echo "runtime boundary test: PASS (negative Xlorie rejected, valid bundle accepted)"
