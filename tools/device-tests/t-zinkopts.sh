#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
for f in /usr/lib/aarch64-linux-gnu/libgallium-*.so /usr/lib/aarch64-linux-gnu/dri/*.so /usr/lib/aarch64-linux-gnu/libvulkan_lvp.so; do
  [ -f "$f" ] || continue
  n=$(grep -a -o "ZINK_[A-Z0-9_]*" "$f" 2>/dev/null | sort -u | tr '\n' ' ')
  [ -n "$n" ] && echo "$f: $n"
done
