#!/bin/sh
# Audit the checked-in Xlorie binary as a reproducible release input, not just
# as "some .so that happens to load".
set -eu
cd "$(dirname "$0")/../.."

BIN=third_party/xlorie/arm64-v8a/libXlorie.so
README=third_party/xlorie/README.md
WANT=76b4abc6827a0494c43ba55dcbc68cd8ad69d1ec0a761a8b9af6c5350b0abc34

fail() {
    echo "FAIL $*" >&2
    exit 1
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail "no sha256sum or shasum"
    fi
}

[ -f "$BIN" ] || fail "missing $BIN"
[ -f "$README" ] || fail "missing $README"

have=$(sha256_file "$BIN")
[ "$have" = "$WANT" ] || fail "libXlorie.so sha256 $have != $WANT"
grep -F "$WANT" "$README" >/dev/null || fail "README does not pin libXlorie.so sha256"

if command -v readelf >/dev/null 2>&1; then
    readelf_cmd=readelf
elif command -v llvm-readelf >/dev/null 2>&1; then
    readelf_cmd=llvm-readelf
elif [ -n "${NDK:-}" ] && [ -x "$NDK/llvm-readelf" ]; then
    readelf_cmd=$NDK/llvm-readelf
else
    fail "readelf/llvm-readelf not found (install binutils, add NDK bin to PATH, or set NDK)"
fi

header=$($readelf_cmd -h "$BIN")
printf '%s\n' "$header" | grep -q 'Class:.*ELF64' || fail "libXlorie.so is not ELF64"
printf '%s\n' "$header" | grep -q 'Machine:.*AArch64' || fail "libXlorie.so is not AArch64"
printf '%s\n' "$header" | grep -q 'Type:.*DYN' || fail "libXlorie.so is not a shared object"

sections=$($readelf_cmd -S --wide "$BIN")
if printf '%s\n' "$sections" | grep -Eq '\.debug_|\.note\.gnu\.build-id'; then
    fail "libXlorie.so still contains debug sections or a GNU build-id"
fi

symbols=$($readelf_cmd --dyn-syms --wide "$BIN" | awk '$8 ~ /^husk_xlorie_/ {print $8}' | sort -u)
set -- \
    husk_xlorie_connect \
    husk_xlorie_connect_renderer \
    husk_xlorie_send_key \
    husk_xlorie_send_mouse \
    husk_xlorie_send_touch \
    husk_xlorie_send_window_size \
    husk_xlorie_set_jni_env \
    husk_xlorie_set_window \
    husk_xlorie_start
for symbol in "$@"; do
    printf '%s\n' "$symbols" | grep -Fx "$symbol" >/dev/null || fail "missing ABI symbol $symbol"
done

echo "OK   libXlorie.so sha256=$WANT"
echo "OK   ELF64 AArch64 shared object, stripped, 9 husk ABI symbols"
