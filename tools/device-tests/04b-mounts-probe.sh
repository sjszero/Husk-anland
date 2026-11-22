#!/bin/sh
echo "== top level of the guest view =="
for d in / /vendor /system /apex /sys /proc /dev; do
    if [ -d "$d" ]; then echo "dir   $d"; else echo "MISS  $d"; fi
done
echo
echo "== does the guest see Android's vendor trees? =="
ls /vendor 2>&1 | head -6
echo "--"
ls /system 2>&1 | head -6
echo
echo "== kgsl sysfs (Adreno driver) =="
for f in /sys/class/kgsl/kgsl-3d0/gpu_model /sys/class/kgsl/kgsl-3d0/gpu_busy_percentage /sys/class/kgsl/kgsl-3d0/devfreq/cur_freq; do
    if [ -r "$f" ]; then echo "read  $f = $(cat "$f" 2>&1 | head -1)"; else echo "MISS  $f"; fi
done
ls /sys/class/kgsl/kgsl-3d0 2>&1 | head -8
echo
echo "== vendor libs visible from guest? =="
for p in /vendor/lib64/egl/libGLESv2_adreno.so /vendor/lib64/libEGL_adreno.so /vendor/lib64/hw/vulkan.adreno.so; do
    [ -e "$p" ] && echo "exists $p" || echo "absent $p"
done
echo "#done"
