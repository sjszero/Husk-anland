#!/bin/sh
# A release rootfs can be exposed through a stable symlink. The session must
# resolve that symlink before looking for the libc backup and install state,
# otherwise it can mix the new rootfs with an old bundle's libc and overwrite
# the installed tree.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d /tmp/husk-session-rootfs-bundle.XXXXXX)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

bundle=$tmp/bundle
legacy=$tmp/linux/distributions/ubuntu-24.04
rootfs=$bundle/root
mkdir -p "$rootfs/etc" "$rootfs/run/dbus" "$rootfs/tmp" \
    "$rootfs/usr/bin" "$rootfs/usr/lib/aarch64-linux-gnu" \
    "$rootfs/usr/share/vulkan/icd.d" \
    "$legacy"
printf 'root:x:0:0:root:/root:/bin/sh\n' > "$rootfs/etc/passwd"
: > "$rootfs/usr/share/vulkan/icd.d/husk_panvk_guest.json"

printf 'new-patched-libc\n' > "$rootfs/usr/lib/aarch64-linux-gnu/libc.so.6"
printf 'new-patched-libc\n' > "$bundle/libc.so.6"
printf 'legacy-patched-libc\n' > "$legacy/libc.so.6"
new_sha=$(hash_file "$rootfs/usr/lib/aarch64-linux-gnu/libc.so.6")
old_sha=$(hash_file "$legacy/libc.so.6")
printf 'libc_sha256=%s\n' "$new_sha" > "$bundle/.husk-install-state"
printf 'libc_sha256=%s\n' "$old_sha" > "$legacy/.husk-install-state"
ln -s ../../../bundle/root "$legacy/rootfs"

loader=$tmp/fake-loader
cat > "$loader" <<'EOF'
#!/bin/sh
printf 'loaded\n' > "$CAPTURE"
EOF
chmod +x "$loader"

if ! CAPTURE=$tmp/loader.out NR_LD=$loader sh tools/guest/husk-session.sh \
    "$legacy/rootfs" -- /usr/bin/true 2> "$tmp/stderr"; then
    echo "session rootfs bundle test: session exited non-zero" >&2
    sed 's/^/  /' "$tmp/stderr" >&2
    exit 1
fi

if [ "$(hash_file "$rootfs/usr/lib/aarch64-linux-gnu/libc.so.6")" != "$new_sha" ]; then
    echo "session rootfs bundle test: new rootfs libc was overwritten from the legacy bundle" >&2
    exit 1
fi
if grep -q 'restored' "$tmp/stderr"; then
    echo "session rootfs bundle test: session used the legacy install state" >&2
    sed 's/^/  /' "$tmp/stderr" >&2
    exit 1
fi
if [ ! -f "$tmp/loader.out" ]; then
    echo "session rootfs bundle test: fake loader was not reached" >&2
    exit 1
fi

echo "session rootfs bundle: PASS"
