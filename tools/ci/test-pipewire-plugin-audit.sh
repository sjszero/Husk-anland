#!/bin/sh
# Positive/negative test for audit-pipewire-plugin.sh, plus patch round-trip.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-pipewire-plugin-audit.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

sh tools/ci/audit-pipewire-plugin.sh vendor/pipewire/libgstpipewire.so >/dev/null
cp vendor/pipewire/libgstpipewire.so "$tmp/tampered.so"
printf '\nlocal-tamper\n' >> "$tmp/tampered.so"
if sh tools/ci/audit-pipewire-plugin.sh "$tmp/tampered.so" >/dev/null 2>&1; then
    echo "pipewire plugin audit test: tampered plugin was accepted" >&2
    exit 1
fi

mkdir -p "$tmp/src/src/gst"
cp vendor/pipewire/src/gst/gstpipewiresrc.c "$tmp/src/src/gst/gstpipewiresrc.c"
(
    cd "$tmp/src"
    patch -R -p1 < "$OLDPWD/vendor/pipewire/1.0.5-gst-caps-subset.patch" >/dev/null
)
[ "$(sha256sum "$tmp/src/src/gst/gstpipewiresrc.c" | cut -d' ' -f1)" = \
  c3edb98be06d584a945ae68ba0356a29ccd0da7fe318c7360d780a538a74caaa ] || {
    echo "pipewire plugin audit test: reverse patch did not reproduce upstream 1.0.5" >&2
    exit 1
}
(
    cd "$tmp/src"
    patch -p1 < "$OLDPWD/vendor/pipewire/1.0.5-gst-caps-subset.patch" >/dev/null
)
[ "$(sha256sum "$tmp/src/src/gst/gstpipewiresrc.c" | cut -d' ' -f1)" = \
  9e289d0d95fe34bc80c048fece71a9fdd653654e79308849324e10bd94e22fbb ] || {
    echo "pipewire plugin audit test: forward patch did not reproduce vendored source" >&2
    exit 1
}

echo "pipewire plugin audit test: PASS (plugin hash checked, tamper rejected, patch round-trip exact)"
