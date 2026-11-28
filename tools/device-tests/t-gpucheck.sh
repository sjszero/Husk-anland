#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
pid=
for p in /proc/[0-9]*; do [ "$(cat $p/comm 2>/dev/null)" = gnome-shell ] && pid=${p#/proc/}; done
echo "shell pid=$pid"
tr '\0' '\n' < /proc/$pid/environ | grep -E "GALLIUM|MESA|VK_|LIBGL|HUSK_GPU|GSK" | sort
