#!/bin/sh
# Release gate for the vendored Mali kbase PanVK snapshot.
#
# A mismatched manifest or a truncated gzip does not fail at build time by
# itself: the release tar still packs, the installer reports success, and only
# the first GL/Vulkan program silently falls back to llvmpipe (or SIGSEGVs).
# This script checks the bytes and the guest-relative ICD contract before dist.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
V="$ROOT/vendor/panvk/fristonerr-1.2.0-mali-g77"
DRIVER_GZ="$V/libvulkan_panfrost_frist370_execva.so.gz"
ICD="$V/husk_panvk_guest.json"
WANT_GZ=587d77b5637a1603f84acda1685d4896959459086a648f36e68eb36adc3fc4bd
WANT_SO=599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c
TMP=${TMPDIR:-/tmp}/husk-panvk-audit.$$
trap 'rm -f "$TMP"' EXIT HUP INT TERM

fail() { echo "panvk bundle: FAIL: $*" >&2; exit 1; }
[ -f "$DRIVER_GZ" ] || fail "missing $DRIVER_GZ"
[ -f "$ICD" ] || fail "missing $ICD"
[ -f "$V/LICENSE-Mesa.txt" ] || fail "missing Mesa license"
[ -f "$V/PROVENANCE.md" ] || fail "missing provenance"

have=$(sha256sum "$DRIVER_GZ" | cut -d' ' -f1)
[ "$have" = "$WANT_GZ" ] || fail "compressed driver sha256 $have != $WANT_GZ"

gzip -dc "$DRIVER_GZ" > "$TMP" || fail "driver gzip is corrupt"
have=$(sha256sum "$TMP" | cut -d' ' -f1)
[ "$have" = "$WANT_SO" ] || fail "decompressed driver sha256 $have != $WANT_SO"

grep -q '"library_path"[[:space:]]*:[[:space:]]*"/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so"' "$ICD" \
    || fail "ICD library_path is not the guest-relative driver path"
grep -q '"api_version"[[:space:]]*:[[:space:]]*"1.3.0"' "$ICD" \
    || fail "ICD api_version is not 1.3.0"
grep -q 'FristOneRR' "$V/PROVENANCE.md" || fail "provenance has no upstream identity"
grep -q '0c9dce35094dbe89c412b79bf57c6067c7b97e59' "$V/PROVENANCE.md" \
    || fail "provenance has no pinned source commit"
grep -q 'Permission is hereby granted' "$V/LICENSE-Mesa.txt" \
    || fail "Mesa license text is missing"

echo "panvk bundle: PASS ($WANT_GZ)"
