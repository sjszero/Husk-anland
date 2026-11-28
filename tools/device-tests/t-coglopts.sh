#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
pid=
for p in /proc/[0-9]*; do [ "$(cat $p/comm 2>/dev/null)" = gnome-shell ] && pid=${p#/proc/}; done
echo "shell=$pid"
tr '\0' '\n' < /proc/$pid/environ | grep -E "COGL_DRIVER|GJS_DISABLE_JIT|MESA_NO_ERROR|CLUTTER_BACKEND" | sort
echo "== cogl debug tokens in libmutter:"
for f in /usr/lib/gnome-shell/libmutter-14.so* /usr/lib/aarch64-linux-gnu/libmutter-14.so*; do
  [ -f "$f" ] || continue
  echo "-- $f"
  grep -a -o "disable-[a-z-]*" "$f" 2>/dev/null | sort -u | head -20
done
