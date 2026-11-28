#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
R=${NR_ROOTFS:?NR_ROOTFS must be set by run-guest.sh}
cat > /tmp/icd-warmup-on.json <<J
{"file_format_version":"1.0.1","ICD":{"api_version":"1.3.0","library_arch":"64","library_path":"$R/usr/lib/aarch64-linux-gnu/.libvulkan_panfrost_husk.so.bak-orig"}}
J
cat > /tmp/icd-warmup-off.json <<J
{"file_format_version":"1.0.1","ICD":{"api_version":"1.3.0","library_arch":"64","library_path":"$R/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so"}}
J
echo "== A/B reason: same session, only the ICD library_path differs =="
for i in 1 2; do
  s=$(date +%s%N); VK_ICD_FILENAMES=/tmp/icd-warmup-on.json VK_DRIVER_FILES=/tmp/icd-warmup-on.json env -u DISPLAY /usr/bin/vulkan-probe >/dev/null 2>&1; e=$(date +%s%N)
  echo "WARMUP ON  (upstream 2s) vulkan-probe run$i: $(( (e-s)/1000000 )) ms"
done
for i in 1 2; do
  s=$(date +%s%N); VK_ICD_FILENAMES=/tmp/icd-warmup-off.json VK_DRIVER_FILES=/tmp/icd-warmup-off.json env -u DISPLAY /usr/bin/vulkan-probe >/dev/null 2>&1; e=$(date +%s%N)
  echo "WARMUP OFF (patched) vulkan-probe run$i: $(( (e-s)/1000000 )) ms"
done
for i in 1 2; do
  s=$(date +%s%N); VK_ICD_FILENAMES=/tmp/icd-warmup-on.json VK_DRIVER_FILES=/tmp/icd-warmup-on.json DISPLAY=:0 /usr/bin/glxinfo -B >/dev/null 2>&1; e=$(date +%s%N)
  echo "WARMUP ON  glxinfo -B run$i: $(( (e-s)/1000000 )) ms"
done
for i in 1 2; do
  s=$(date +%s%N); VK_ICD_FILENAMES=/tmp/icd-warmup-off.json VK_DRIVER_FILES=/tmp/icd-warmup-off.json DISPLAY=:0 /usr/bin/glxinfo -B >/dev/null 2>&1; e=$(date +%s%N)
  echo "WARMUP OFF glxinfo -B run$i: $(( (e-s)/1000000 )) ms"
done
