#!/bin/bash
# 24 的静态段：把每个 husk ICD 清单的 library_path 打出来，交给宿主侧判定。
export PATH=/usr/bin:/bin:$PATH
for f in /usr/share/vulkan/icd.d/husk_panvk*.json; do
  [ -f "$f" ] || continue
  p=$(sed -n 's/.*"library_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$f" | head -n 1)
  echo "ICD $(basename "$f") $p"
done
