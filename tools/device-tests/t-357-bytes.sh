#!/bin/bash
# 357 补测：19 项精确读回在两颗驱动下是否都成立（判据是等于真值，不是两次自我一致）。
export PATH=/usr/bin:/bin:$PATH
R=${NR_ROOTFS:?}
cat > /tmp/icd-on.json <<J
{"file_format_version":"1.0.1","ICD":{"api_version":"1.3.0","library_arch":"64","library_path":"$R/tmp/panvk_upstream.so"}}
J
cat > /tmp/icd-off.json <<J
{"file_format_version":"1.0.1","ICD":{"api_version":"1.3.0","library_arch":"64","library_path":"$R/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so"}}
J
one() { # <icd> <label>
  VK_ICD_FILENAMES=$1 VK_DRIVER_FILES=$1 env -u DISPLAY /tmp/glprobe2 readback-suite > "/tmp/rb-$2.log" 2>&1
  echo "[$2] rc=$?  PASS lines=$(grep -c '^PASS' /tmp/rb-$2.log)  FAIL lines=$(grep -c '^FAIL' /tmp/rb-$2.log)  $(grep 'PASS=' /tmp/rb-$2.log | tail -1)"
}
one /tmp/icd-on.json  on
one /tmp/icd-off.json off
echo "== stdout diff (on vs off) =="
if cmp -s /tmp/rb-on.log /tmp/rb-off.log; then echo "STDOUT IDENTICAL"; else diff /tmp/rb-on.log /tmp/rb-off.log | head -10; fi
