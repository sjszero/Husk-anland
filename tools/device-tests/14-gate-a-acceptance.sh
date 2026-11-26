#!/bin/sh
# Gate A acceptance: bash / locale / user management / dpkg / apt / no host-path
# leak / no real seccomp trap - all measured inside the guest, on whatever rootfs
# the caller points at.
#
#   DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
#   ROOTFS=files/linux/husk-clean/rootfs LD=files/linux/husk/libld.so \
#     ./14-gate-a-acceptance.sh
#
# The guest body below is pushed as a file (a heredoc *inside* run-as cannot
# create temporary files in this data domain) and then copied into the rootfs.
set -eu
DEV=${DEV:?usage: DEV=<serial> PKG=<package> ROOTFS=<rel rootfs> LD=<rel loader> $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/linux/husk-clean/rootfs}
LD=${LD:-files/linux/husk/libld.so}

HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/accept.sh" <<'GUEST'
#!/bin/bash
# Runs inside the guest as root. Every check prints one PASS/FAIL line.
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }
eq()  { [ "$2" = "$3" ] && ok "$1 ($3)" || bad "$1 (want '$2' got '$3')"; }

export NR_SIGSYS_LOG=/tmp/gate-a-sigsys.log
: > /tmp/gate-a-sigsys.log 2>/dev/null || true
export LC_ALL=C.UTF-8

echo "== identity =="
eq "guest uid is root" "0" "$(id -u)"
eq "guest root is the rootfs" "/" "$(cd / && pwd)"

echo "== bash =="
eq "bash version" "5.2.21(1)-release" "$(bash -c 'echo $BASH_VERSION')"
eq "pipeline" "3" "$(printf 'a\nb\nc\n' | wc -l | tr -d ' ')"
out=$(bash -c 'cat <<EOF
heredoc:$((6*7))
EOF')
eq "heredoc + arithmetic" "heredoc:42" "$out"
eq "subshell isolation" "1 0" "$(bash -c 'x=1; ( x=2 ); echo $x $((2-2))')"
eq "process substitution" "ok" "$(bash -c 'cat <(echo ok)')"
eq "brace expansion" "4" "$(bash -c 'echo {1..4} | wc -w | tr -d " "')"

echo "== locale =="
eq "zh_CN charmap" "UTF-8" "$(LC_ALL=zh_CN.UTF-8 locale charmap)"
eq "zh_CN weekday" "星期一" "$(LC_ALL=zh_CN.UTF-8 date -d 2026-11-23 +%A)"
eq "C.UTF-8 collation works" "ok" "$(LC_ALL=C.UTF-8 bash -c 'echo ok')"

echo "== user management =="
groupdel nrtest 2>/dev/null; userdel -r nrtest 2>/dev/null
eq "groupadd rc" "0" "$(groupadd -g 1500 nrtest; echo $?)"
eq "useradd rc" "0" "$(useradd -u 1500 -g 1500 -m -s /bin/bash nrtest 2>&1; echo $?)"
eq "chpasswd rc" "0" "$(echo 'nrtest:husk123' | chpasswd; echo $?)"
eq "id resolves" "1500" "$(id -u nrtest)"
eq "home created" "yes" "$([ -d /home/nrtest ] && echo yes || echo no)"
eq "shadow hash" "sha512" "$(awk -F: '$1=="nrtest"{ if ($2 ~ /^\$6\$/) print "sha512"; else print "other" }' /etc/shadow)"
eq "group entry" "nrtest" "$(getent group 1500 | cut -d: -f1)"
groupdel nrtest 2>/dev/null
userdel -r nrtest 2>/dev/null
eq "cleanup" "gone" "$(id nrtest >/dev/null 2>&1 && echo present || echo gone)"

echo "== dpkg =="
eq "dpkg -C clean before" "0" "$(dpkg -C >/tmp/dpkgC.1 2>&1; wc -l < /tmp/dpkgC.1 | tr -d ' ')"
# Package count is a property of the rootfs, not of the runtime: a tree with
# mesa-vulkan-drivers installed carries more packages than the bare one this
# gate was written against. Judge dpkg health (below) plus a floor, and print
# the exact number instead of demanding one magic value.
pkg_n=$(dpkg-query -W -f='${Status}\n' | grep -c 'install ok installed')
[ "$pkg_n" -ge "${EXPECT_PKGS_MIN:-700}" ] \
  && ok "installed packages ($pkg_n >= ${EXPECT_PKGS_MIN:-700})" \
  || bad "installed packages (want >= ${EXPECT_PKGS_MIN:-700} got $pkg_n)"
eq "unpacked packages" "0" "$(dpkg-query -W -f='${Status}\n' | grep -c 'install ok unpacked')"
# A .deb to re-install. The App tree removes its offline cache once the
# initialisation ends, so discovery has to look in more than one place; when
# there is none, say so instead of turning a missing fixture into a failure.
deb=$(ls /var/cache/doro-offline/packages/*.deb 2>/dev/null | head -1)
[ -n "$deb" ] || deb=$(ls /var/cache/apt/archives/*.deb 2>/dev/null | head -1)
if [ -n "$deb" ]; then
  eq "dpkg-deb -I ($(basename "$deb"))" "ok" "$(dpkg-deb -I "$deb" >/dev/null 2>&1 && echo ok || echo fail)"
  eq "force-overwrite reinstall rc" "0" "$(dpkg --force-overwrite -i "$deb" >/tmp/reinstall.log 2>&1; echo $?)"
else
  echo "  NOTE  no .deb on this tree (offline cache already removed) - reinstall checks skipped"
fi
eq "dpkg -C clean after" "0" "$(dpkg -C >/tmp/dpkgC.2 2>&1; wc -l < /tmp/dpkgC.2 | tr -d ' ')"
eq "getfacl still runs" "ok" "$(getfacl --version >/dev/null 2>&1 && echo ok || echo fail)"

echo "== apt =="
eq "apt version" "2.8.3" "$(apt --version 2>/dev/null | awk '{print $2}')"
pol=$(apt-cache policy acl 2>/dev/null | sed -n 's/^  Installed: //p')
eq "policy knows acl" "yes" "$([ -n "$pol" ] && [ "$pol" != "(none)" ] && echo yes || echo no)"
echo "  apt-cache policy acl -> Installed: ${pol:-<none>}"
apt-get -s install --reinstall acl >/tmp/apt-dry.log 2>&1; eq "apt-get -s rc" "0" "$?"
grep -qi 'cannot be downloaded\|Unable to fetch' /tmp/apt-dry.log \
  && echo "  NOTE  apt-get -s needs a network index (offline cache has the deb): $(grep -m1 -i 'cannot be downloaded\|Unable to fetch' /tmp/apt-dry.log)"

echo "== host-path leak =="
outside=$(env | grep '/data/user' | cut -d= -f1 | sort | tr '\n' ' ' | sed 's/ $//')
echo "  vars whose value points at the app dir: ${outside:-<none>}"
eq "host paths only in internal NR_ vars" "0" \
  "$(echo "$outside" | tr ' ' '\n' | grep -v '^NR_' | grep -c . || true)"
# The invariant is containment, not a process count: a desktop session
# legitimately runs 60+ guest processes while an idle shell runs 5, so counting
# proved nothing and was changed on 2026-11-24. What must never happen is the
# guest seeing *another* Android app: our own package (and its :x11 process) is
# expected - it is the container - anything else is a leak.
# The exclusion has to name the package under test, not the one this script was
# first written against. Hard-coding com.linuxhub.myth.husk made *our own*
# processes count as a leak on com.linuxhub.husk (measured 2026-11-26: the count
# was 1 on a tree with no other app running at all). HUSK_PKG comes from the host
# invocation; the old name stays excluded so an old install still reads clean.
own=${HUSK_PKG:-com.linuxhub.husk}
leaked=$(ps -e -o args= 2>/dev/null | grep -oE 'com\.[a-z0-9_.]+' \
           | grep -v -x -e "$own" -e com.linuxhub.myth.husk | sort -u || true)
[ -n "$leaked" ] && { echo "  offenders:"; printf '%s\n' "$leaked" | head -5 | sed 's/^/    /'; }
eq "ps exposes no Android app other than this one" "0" "$(printf '%s' "$leaked" | grep -c . || true)"
# 把命中的行打出来：只报一个计数的话，出现 1 的时候没法判断是宿主泄漏还是
# 我们自己的进程被误计（2026-11-25 第一次跑 0.0.11 门禁时就是这种情况）。
# 注意：这条检查自己的 grep 命令行里就带着上面那几个 pattern，`ps` 会把它列出来，
# 于是"命中 1 条"永远是它自己（2026-11-25 实测）。先把带 grep 的行剔掉再计数。
hosts=$(ps -e -o args 2>/dev/null | grep -v '/data/user' \
          | grep -v 'grep -E' \
          | grep -E 'system_server|zygote|/system/bin|/apex' || true)
[ -n "$hosts" ] && { echo "  offenders:"; echo "$hosts" | head -5 | sed 's/^/    /'; }
eq "no host system processes visible" "0" "$(printf '%s' "$hosts" | grep -c . || true)"
# FACT worth recording: the loader keeps its own argv, so a guest reading
# /proc/self/cmdline or `ps -o args` sees the app dir. Same uid, no privilege
# crosses; recorded instead of papered over.
nargv=$(ps -e -o args 2>/dev/null | grep -c '/data/user' || true)
echo "  NOTE  guest-visible argv shows the app dir in $nargv process(es) (the loader's own argv)"
eq "guest /data/user does not exist" "absent" "$([ -e /data/user ] && echo present || echo absent)"
eq "guest /system does not exist" "absent" "$([ -e /system ] && echo present || echo absent)"
# `/proc/self/exe` 必须报**当前 guest 程序**的 guest 路径（evidence 217）。
# 三条一起断言，缺一条就可能"看着对"：路径里不许出现 app 目录、在 guest 里
# 必须真的存在且可执行、而且必须是**这个**程序（这里跑的是 readlink 自己）。
# 0.0.11 之前它返回宿主 loader 的绝对路径，`sys.executable`、按自身目录找资源的
# 程序全跟着错。
exe=$(readlink /proc/self/exe)
eq "/proc/self/exe has no app dir" "clean" \
  "$(case "$exe" in /data/*) echo leaked;; *) echo clean;; esac)"
eq "/proc/self/exe resolves inside guest" "ok" "$([ -x "$exe" ] && echo ok || echo missing)"
eq "/proc/self/exe names this program" "readlink" "$(basename "$exe")"

echo "== seccomp traps (real, excluding the init marker) =="
traps=$(grep -c '^sigsys|nr=' /tmp/gate-a-sigsys.log 2>/dev/null || true)
eq "real SIGSYS traps" "0" "${traps:-0}"
[ -s /tmp/gate-a-sigsys.log ] && { echo "  log tail:"; tail -3 /tmp/gate-a-sigsys.log | sed 's/^/    /'; }

echo
echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
GUEST

cat > "$TMP/husk-exec.sh" <<'GUESTEXEC'
#!/bin/sh
# Guest launcher for the gate scripts. Four things the loader needs and none of
# them are defaults:
#   * cwd inside the guest root - the program argument is guest-relative
#     ("usr/bin/bash") and is resolved against it;
#   * --library-path, which *replaces* the loader's search path, so the guest's
#     standard directories and every directory its own binaries declare in
#     DT_RUNPATH go here (glibc resolves RUNPATH inside the loader, where nothing
#     can interpose - evidence 139);
#   * NR_ROOTFS and NR_LD both set. Without NR_ROOTFS the parent process does no
#     rewriting at all; without NR_LD a re-entering child falls back to the
#     distro loader;
#   * the app process' HOME/XDG values dropped, or dconf creates
#     $HOME/.cache/dconf *inside* the rootfs as rootfs/data/user/0/... (measured).
# usage: husk-exec.sh <rootfs> <loader> <program> [args...]
R=$1; L=$2; shift 2
# Both come in relative to the app's data directory (what run-as starts in): make
# them absolute before the cd below, or the loader path breaks with it.
case $R in /*) ;; *) R=$PWD/$R ;; esac
case $L in /*) ;; *) L=$PWD/$L ;; esac
E=
[ -f "$R/etc/ld.so.husk-path" ] && E=$(sed -n 's|^/|'"$R"'|p' "$R/etc/ld.so.husk-path" | tr '\n' ':')
cd "$R" || exit 1
unset XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
# NR_LIBRARY_PATH is the launcher-to-shim contract for *nested* execs (the app
# exports it). Without it a child falls back to the shim's built-in list plus
# /etc/ld.so.husk-path, which is close but not what the app does - and the gate
# has to measure the app's launch, not a cousin of it.
LP="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu:$E"
HOME=/root EXPECT_PKGS=${EXPECT_PKGS:-811} \
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  NR_ROOTFS=$R NR_LD=$L NR_LIBRARY_PATH="$LP" \
  exec "$L" --library-path "$LP" "$@"
GUESTEXEC

adb -s "$DEV" push "$TMP/accept.sh" /data/local/tmp/husk-accept.sh >/dev/null
adb -s "$DEV" push "$TMP/husk-exec.sh" /data/local/tmp/husk-exec.sh >/dev/null
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/husk-accept.sh $ROOTFS/tmp/husk-accept.sh"
adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/tmp/husk-accept.sh"
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/husk-exec.sh $ROOTFS/tmp/husk-exec.sh"
adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/tmp/husk-exec.sh"
adb -s "$DEV" shell "run-as $PKG env EXPECT_PKGS=${EXPECT_PKGS:-811} HUSK_PKG=$PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD usr/bin/bash /tmp/husk-accept.sh" 2>&1
