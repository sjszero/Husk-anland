#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
echo "== who has disable-software-clip:"
for f in /usr/lib/gnome-shell/*.so* /usr/lib/aarch64-linux-gnu/libmutter*.so* /usr/lib/aarch64-linux-gnu/libcogl*.so* /usr/lib/aarch64-linux-gnu/libgallium*.so; do
  [ -f "$f" ] || continue
  if grep -qa "disable-software-clip" "$f" 2>/dev/null; then echo "  $f"; fi
done
echo "== cogl debug tokens:"
for f in /usr/lib/gnome-shell/libmutter-14.so.0.0.0 /usr/lib/aarch64-linux-gnu/libcogl.so* /usr/lib/aarch64-linux-gnu/libgallium-27.0.0-devel.so; do
  [ -f "$f" ] || continue
  echo "-- $f"
  grep -a -o "disable-[a-z-]*\|[a-z-]*mipmap[a-z-]*\|[a-z-]*mip[a-z-]*" "$f" 2>/dev/null | sort -u | head -25
done
