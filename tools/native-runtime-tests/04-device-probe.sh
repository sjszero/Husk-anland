#!/bin/sh
# What the guest can reach on the device: PTY, DRM/GPU nodes, vendor GL libs.
# Read-only. Always exits 0 so the harness prints everything.
echo "== identity =="
id
awk '/^(Name|Uid|Gid|CapEff)/{print}' /proc/self/status

echo
echo "== nodes (guest view) =="
for p in /dev/ptmx /dev/pts /dev/dri /dev/kgsl-3d0 /dev/mali0 /dev/fastrpc-cdsp; do
    if [ -e "$p" ]; then printf '%-22s %s\n' "$p" "$(ls -ld "$p" 2>&1 | cut -d' ' -f1,3,4)"; else printf '%-22s %s\n' "$p" "MISSING"; fi
done

echo
echo "== open tests =="
probe() {  # probe <path> <fd>
    if eval "exec $2<>'$1'" 2>/dev/null; then
        echo "OPEN  ok      $1"
        readlink /proc/self/fd/$2 2>/dev/null | sed 's/^/            -> /'
        eval "exec $2<&-"
    else
        echo "OPEN  DENIED  $1"
    fi
}
probe /dev/ptmx 3
probe /dev/pts/ptmx 3
probe /dev/dri/renderD128 4
probe /dev/dri/card0 4
probe /dev/kgsl-3d0 5
probe /dev/mali0 5

echo
echo "== vendor graphics libs (readable? executable-mapped?) =="
for d in /vendor/lib64/egl /vendor/lib64/hw /system/lib64; do
    [ -d "$d" ] || continue
    echo "-- $d"
    ls "$d" 2>/dev/null | grep -Ei 'egl|gles|vulkan|gralloc|adreno|mali' | head -8 | while read -r f; do
        if [ -r "$d/$f" ]; then r=r; else r=-; fi
        echo "   $r  $d/$f"
    done
done

echo
echo "== ptmx smoke test: does a pty pair actually work? =="
if command -v script >/dev/null 2>&1; then
    echo "hello from a pty" | script -q -c 'cat' /dev/null 2>&1 | head -2
else
    echo "no 'script' binary in the guest"
fi
echo "#done"
