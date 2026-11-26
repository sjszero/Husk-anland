#!/bin/sh
# Exercise the GPU variables that must survive husk-session.sh's final env -i.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d /tmp/husk-session-gpu.XXXXXX)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

rootfs=$tmp/rootfs
mkdir -p "$rootfs/etc" "$rootfs/run/dbus" "$rootfs/tmp" "$rootfs/usr/bin" \
    "$rootfs/usr/share/vulkan/icd.d"
printf 'root:x:0:0:root:/root:/bin/sh\n' > "$rootfs/etc/passwd"
: > "$rootfs/usr/share/vulkan/icd.d/husk_panvk_guest.json"
: > "$rootfs/usr/bin/env"

# No bus config and no dbus-daemon in this fixture. The session must skip the
# optional bus start instead of waiting six seconds or depending on `nc -lU`.

loader=$tmp/fake-loader
cat > "$loader" <<'EOF'
#!/bin/sh
for arg in "$@"; do
    printf 'ARG:%s\n' "$arg" >> "$CAPTURE"
done
EOF
chmod +x "$loader"

run_case() {
    name=$1
    expected=$2
    forbidden=$3
    shift 3
    capture=$tmp/$name.args
    : > "$capture"
    CAPTURE=$capture NR_LD=$loader env "$@" sh tools/guest/husk-session.sh \
        "$rootfs" -- /usr/bin/true
    if ! grep -Fqx "$expected" "$capture"; then
        echo "session GPU test: $name missing '$expected'" >&2
        sed 's/^/  /' "$capture" >&2
        exit 1
    fi
    if [ -n "$forbidden" ] && grep -Fq "$forbidden" "$capture"; then
        echo "session GPU test: $name unexpectedly contains '$forbidden'" >&2
        sed 's/^/  /' "$capture" >&2
        exit 1
    fi
    echo "session GPU test: $name PASS"
}

run_case husk-gpu \
    ARG:HUSK_GPU=llvmpipe ARG:VK_ICD_FILENAMES \
    HUSK_GPU=llvmpipe
run_case default-off \
    ARG:NR_GPU_DEFAULT_OFF=1 ARG:VK_ICD_FILENAMES \
    NR_GPU_DEFAULT_OFF=1
run_case force-software \
    ARG:NR_FORCE_SOFTWARE_GPU=1 ARG:VK_ICD_FILENAMES \
    NR_FORCE_SOFTWARE_GPU=1
run_case default-off-zero \
    ARG:VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json '' \
    NR_GPU_DEFAULT_OFF=0

echo "session GPU escape: PASS"
