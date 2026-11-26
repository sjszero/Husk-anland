#!/system/bin/sh
# soak.sh <runner> [seconds] — idle soak, run ON THE DEVICE as the app uid.
#
# Samples the runner's own process every interval: RSS, thread count, open fds,
# and how many zombies it left behind. Dimension 7 in the benchmark asks for
# growth over 30 idle minutes, so this prints a series and the growth is read
# off the first and last lines — nothing is summarised here.
#
#   adb shell "run-as com.linuxhub.husk sh files/linux/bench/soak.sh proot 1800"
set -u
RUNNER=${1:?usage: soak.sh <runner> [seconds]}
SECS=${SECS:-1800}
INT=${INT:-60}
B=/data/user/0/com.linuxhub.husk/files/linux
ME=$$

# Runner names are not file names: proot-distro's script is prootdistro-run.sh.
case "$RUNNER" in
    proot-distro) RS=prootdistro-run.sh ;;
    *.sh)         RS=$RUNNER ;;                 # a full script name is allowed
    *)            RS=$RUNNER-run.sh ;;
esac
[ -f "$B/bench/$RS" ] || { echo "soak: no runner script $B/bench/$RS" >&2; exit 66; }
sh "$B/bench/$RS" 'sleep 3000' &
child=$!

echo "# soak runner=$RUNNER pid=$child secs=$SECS interval=$INT"
echo "t_s,rss_kb,threads,fd,zombies"
i=0
while [ "$i" -le "$SECS" ]; do
    rss=NA; thr=NA; fd=NA
    if [ -r "/proc/$child/status" ]; then
        rss=$(awk '/^VmRSS:/{print $2}' "/proc/$child/status" 2>/dev/null)
        thr=$(awk '/^Threads:/{print $2}' "/proc/$child/status" 2>/dev/null)
        fd=$(ls "/proc/$child/fd" 2>/dev/null | wc -l | tr -d ' ')
    fi
    z=$(ps -A -o ppid,stat 2>/dev/null \
        | awk -v a="$child" -v b="$ME" '($1==a || $1==b) && $2 ~ /^Z/ {n++} END{print n+0}')
    echo "$i,${rss:-NA},${thr:-NA},${fd:-NA},$z"
    sleep "$INT"
    i=$((i + INT))
done
kill "$child" 2>/dev/null
echo "# done"
