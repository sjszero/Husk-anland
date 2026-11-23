#!/bin/sh
# Service supervisor: start, restart-on-crash, backoff, clean stop, no zombies.
#
# husk-init is a freestanding static binary, so it is NOT started through
# gexec.sh (ld.so cannot run a static binary). Run this directly as the app:
#   adb push runtime/target/aarch64-unknown-linux-gnu/release/husk-init /data/local/tmp/husk-init
#   adb push tools/device-tests/12-supervisor.sh /data/local/tmp/12.sh
#   adb shell 'run-as <pkg> sh /data/local/tmp/12.sh'
#
# Overridable so the same script covers any tree/package without editing it:
#   FILES  app-private files dir   (default /data/data/com.linuxhub.husk/files)
#   ROOT   rootfs, relative to FILES (default rootfs-full)
#   LD     loader, relative to FILES (default libld39.so)
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

FILES=${FILES:-/data/data/com.linuxhub.husk/files}
RS=${ROOT:-rootfs-full}
LDS=${LD:-libld39.so}
R=$FILES/$RS
cd "$R" || { echo "no rootfs"; exit 1; }
mkdir -p etc/husk var/log/husk tmp
cp /data/local/tmp/husk-init usr/bin/husk-init || { echo "push husk-init first"; exit 1; }
chmod 755 usr/bin/husk-init

# services, written with echo: no heredocs inside a run-as script
echo '#!/bin/sh' > tmp/ticker.sh
echo 'while :; do echo "tick $(date +%s)" >> /tmp/tick.log; sleep 1; done' >> tmp/ticker.sh
echo '#!/bin/sh' > tmp/quick.sh
echo 'echo ran-once; exit 0' >> tmp/quick.sh
echo '#!/bin/sh' > tmp/boom.sh
echo 'echo boom; exit 3' >> tmp/boom.sh
chmod 755 tmp/ticker.sh tmp/quick.sh tmp/boom.sh
echo 'ticker   always      /usr/bin/bash /tmp/ticker.sh' >  etc/husk/services.conf
echo 'once     once        /usr/bin/bash /tmp/quick.sh'  >> etc/husk/services.conf
echo 'failure  on-failure  /usr/bin/bash /tmp/boom.sh'   >> etc/husk/services.conf

rm -f tmp/tick.log var/log/husk/*.log
export NR_ROOTFS=$R
export NR_LD=$FILES/$LDS
./usr/bin/husk-init > /dev/null 2>&1 &
INIT=$!

sleep 4
n=$(wc -l < tmp/tick.log 2>/dev/null || echo 0)
[ "$n" -ge 2 ] && ok "a service really runs (tick.log=$n)" || bad "a service really runs (tick.log=$n)"

grep -q 'ran-once' var/log/husk/once.log 2>/dev/null \
  && ok "once service ran and was not respawned" || bad "once service ran and was not respawned"
c=$(grep -c 'ran-once' var/log/husk/once.log 2>/dev/null || echo 0)
[ "$c" -eq 1 ] && ok "once service ran exactly once (count=$c)" || bad "once service ran exactly once (count=$c)"

grep -q 'boom' var/log/husk/failure.log 2>/dev/null \
  && ok "on-failure service logged its failure" || bad "on-failure service logged its failure"

# kill the ticker and check the supervisor brings it back
victim=""
for d in /proc/[0-9]*; do
  p=${d#/proc/}
  pp=$(cat "$d/stat" 2>/dev/null | awk '{print $4}')
  [ "$pp" = "$INIT" ] && [ "$(cat "$d/comm" 2>/dev/null)" = "bash" ] && victim=$p
done
if [ -n "$victim" ]; then
  before=$(wc -l < tmp/tick.log)
  kill -9 "$victim" 2>/dev/null
  sleep 4
  after=$(wc -l < tmp/tick.log)
  [ "$after" -gt "$before" ] && ok "a killed service is restarted (tick.log $before -> $after)" \
                             || bad "a killed service is restarted (tick.log $before -> $after)"
else
  bad "could not find the service child of the supervisor"
fi

# stop: everything must be gone, nothing left as a zombie
kill -TERM $INIT
gone=0
i=0
while [ $i -lt 20 ]; do
  sleep 0.5
  i=$((i+1))
  st=$(ps -A -o PID,STAT,NAME | awk -v p=$INIT '$1==p {print $2}')
  if [ -z "$st" ]; then gone=1; break; fi
done
[ $gone -eq 1 ] && ok "SIGTERM stops the supervisor (${i}x0.5s)" || bad "SIGTERM stops the supervisor"
left=$(ps -A -o NAME | grep -c '^bash$')
[ "$left" -eq 0 ] && ok "no service process survives the stop" || bad "$left service process(es) survived the stop"
z=$(ps -A -o STAT,NAME | awk '$1 ~ /^Z/ && $2 ~ /husk-init|bash/ {print}' | wc -l)
[ "$z" -eq 0 ] && ok "no zombies left behind" || bad "$z zombies left behind"

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
