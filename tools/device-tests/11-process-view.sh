#!/bin/sh
# Process view: the guest must see its own processes with their real names, and
# must not be able to read another app's /proc entries.
# Run inside the guest:  sh gexec.sh rootfs-full usr/bin/bash tmp/11.sh
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

c=$(cat /proc/self/comm)
[ "$c" = "cat" ] && ok "/proc/self/comm names the program (cat)" || bad "/proc/self/comm names the program (got '$c')"

ps -e >/dev/null 2>&1 && ok "ps -e runs" || bad "ps -e runs"

# the process running this script must be visible under its own name
ps -eo comm 2>/dev/null | grep -qx bash && ok "ps lists this shell as bash" || bad "ps lists this shell as bash"
ps -eo comm 2>/dev/null | grep -q "^\(libld\|libld39\)\.so$" && bad "ps still shows the loader name" || ok "ps never shows the loader name"

pgrep -x bash >/dev/null 2>&1 && ok "pgrep -x bash matches" || bad "pgrep -x bash matches"

# top must agree with ps about how many tasks there are
n_ps=$(ps -e --no-headers 2>/dev/null | wc -l)
n_top=$(timeout 20 top -b -n1 2>/dev/null | sed -n 's/^Tasks: *\([0-9]*\) total.*/\1/p')
if [ -n "$n_top" ] && [ "$n_top" -ge 1 ] && [ "$n_top" -le $((n_ps + 3)) ] && [ "$n_top" -ge $((n_ps - 3)) ]; then
  ok "top and ps agree on the task count ($n_top vs $n_ps)"
else
  bad "top and ps disagree on the task count ($n_top vs $n_ps)"
fi

# containment: a pid that is not ours must not be readable
for p in 1 2; do
  if [ -r /proc/$p/comm ] && [ "$(cat /proc/$p/comm 2>/dev/null)" != "$c" ]; then
    bad "/proc/$p/comm is readable from the guest"
  else
    ok "/proc/$p/comm is not readable from the guest"
  fi
done

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
