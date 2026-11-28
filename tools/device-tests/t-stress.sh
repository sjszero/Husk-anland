#!/bin/bash
# scratch: does gnome-shell survive a pile of windows? print pid + proc count before/after
export PATH=/usr/local/bin:/usr/bin:/bin
export DISPLAY=:0
shpid() { for p in /proc/[0-9]*; do c=$(cat $p/comm 2>/dev/null); case "$c" in gnome-shell) echo "${p#/proc/}"; return;; esac; done; }
nproc_() { ls /proc 2>/dev/null | grep -c '^[0-9]\+$'; }
echo "before pid=$(shpid) procs=$(nproc_)"
for i in $(seq 1 12); do
    (gnome-text-editor --new-window /etc/hostname >/dev/null 2>&1 &)
    sleep 0.5
done
sleep 15
echo "after  pid=$(shpid) procs=$(nproc_)"
for i in 3; do :; done
echo stress-1-done
