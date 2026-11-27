#!/bin/sh
# Compile and run the distribution-neutral rootfs selector as a standalone Rust
# test. This catches the old failure where desktop code silently resolved an
# Ubuntu-only path while the active runtime tree was files/rootfs-full.
set -eu
cd "$(dirname "$0")/../.."

tmp=${TMPDIR:-/tmp}/husk-rootfs-select.$$.test
trap 'rm -f "$tmp"' EXIT HUP INT TERM

rustc --edition=2021 --test app-rs/src/guest_rootfs.rs -o "$tmp"
"$tmp"

if grep -R -n 'linux/distributions/ubuntu-24.04/rootfs' app-rs/src >/dev/null 2>&1; then
    echo "rootfs selection: FAIL (distribution-specific path in app-rs)" >&2
    exit 1
fi

# Regression H35: only the selector may know the current/legacy tree names.
# A bridge that joins "rootfs-full/..." directly starts the session but writes
# its socket or marker under the wrong tree when the active tree is legacy.
if grep -R -nE '(\.join\("rootfs-(full)?[^" ]*"|const [A-Z_]+: &str = "rootfs-)' app-rs/src     | grep -v 'app-rs/src/guest_rootfs.rs' >/dev/null 2>&1; then
    echo "rootfs selection: FAIL (bridge hardcodes an install-tree name)" >&2
    exit 1
fi

# Executable device-test defaults must point at the same mutable runtime tree as
# the app. The stock-package staging path remains available as an explicit
# override, but it must not be the silent default.
for file in Makefile tools/device-tests/*.sh; do
    if grep -nE '^[[:space:]]*(ROOTFS|ROOTFS_REL|GUEST_ROOT)=.*(linux/)?distributions/ubuntu-24.04/rootfs' "$file" >/dev/null 2>&1; then
        echo "rootfs selection: FAIL (distribution-specific executable default in $file)" >&2
        exit 1
    fi
done

# Regression H34: repository-relative paths must be translated for any
# files/<tree>, not only the historical rootfs-full/rootfs names. Test trees
# such as files/clean-audio-344/root are valid and cross adb in every gate.
for file in tools/device-tests/*.sh tools/probe/*.sh; do
    if grep -q '^device_path()[[:space:]]*{' "$file" \
        && ! grep -q 'files/\*)[[:space:]]*printf' "$file"; then
        echo "device path contract: FAIL (files/* not handled in $file)" >&2
        exit 1
    fi
done

echo "rootfs selection: PASS (rootfs-full -> rootfs, no distro-pinned app or device default)"
