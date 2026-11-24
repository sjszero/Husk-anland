#!/bin/sh
# Gate A acceptance: bash / locale / user management / dpkg / apt / no host-path
# leak / no real seccomp trap - all measured inside the guest, on whatever rootfs
# the caller points at.
#
#   DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
#   ROOTFS=files/linux/husk-clean/rootfs LD=files/linux/husk/libld.so \
#     ./14-gate-a-acceptance.sh
#
# The guest body below is pushed as a file (a heredoc *inside* run-as cannot
# create temporary files in this data domain) and then copied into the rootfs.
set -eu
DEV=${DEV:?usage: DEV=<serial> PKG=<package> ROOTFS=<rel rootfs> LD=<rel loader> $0}
PKG=${PKG:-com.linuxhub.myth.husk}
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
eq "installed packages" "${EXPECT_PKGS:-811}" "$(dpkg-query -W -f='${Status}\n' | grep -c 'install ok installed')"
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
eq "ps sees only the guest process tree" "small" \
  "$( [ "$(ps -e -o pid= 2>/dev/null | wc -l | tr -d ' ')" -le 16 ] && echo small || echo many )"
eq "no host system processes visible" "0" \
  "$(ps -e -o args 2>/dev/null | grep -v '/data/user' | grep -cE 'system_server|zygote|/system/bin|/apex' || true)"
# FACT worth recording: the loader keeps its own argv, so a guest reading
# /proc/self/cmdline or `ps -o args` sees the app dir. Same uid, no privilege
# crosses; recorded instead of papered over.
nargv=$(ps -e -o args 2>/dev/null | grep -c '/data/user' || true)
echo "  NOTE  guest-visible argv shows the app dir in $nargv process(es) (the loader's own argv)"
eq "guest /data/user does not exist" "absent" "$([ -e /data/user ] && echo present || echo absent)"
eq "guest /system does not exist" "absent" "$([ -e /system ] && echo present || echo absent)"
eq "readlink /proc/self/exe is host loader" "host" \
  "$(case "$(readlink /proc/self/exe)" in /data/user/*) echo host;; *) echo guest;; esac)"

echo "== seccomp traps (real, excluding the init marker) =="
traps=$(grep -c '^sigsys|nr=' /tmp/gate-a-sigsys.log 2>/dev/null || true)
eq "real SIGSYS traps" "0" "${traps:-0}"
[ -s /tmp/gate-a-sigsys.log ] && { echo "  log tail:"; tail -3 /tmp/gate-a-sigsys.log | sed 's/^/    /'; }

echo
echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
GUEST

adb -s "$DEV" push "$TMP/accept.sh" /data/local/tmp/husk-accept.sh >/dev/null
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/husk-accept.sh $ROOTFS/tmp/husk-accept.sh"
adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/tmp/husk-accept.sh"
# Three things the loader needs and none of them are defaults:
#   * cwd inside the guest root - the program argument is guest-relative
#     ("usr/bin/bash") and is resolved against it;
#   * --library-path, which *replaces* the loader's search path, so every guest
#     library directory has to be listed (libtinfo.so.6 is the first thing bash
#     asks for);
#   * NR_ROOTFS and NR_LD both set. Without NR_ROOTFS the parent process does
#     no rewriting at all (measured: a relative program still starts, every
#     absolute path - /tmp/husk-accept.sh included - then misses); without NR_LD
#     a re-entering child falls back to the distro loader.
LIBS="lib/aarch64-linux-gnu:usr/lib/aarch64-linux-gnu:usr/lib/aarch64-linux-gnu/mutter-14:usr/lib/gnome-shell:usr/lib/aarch64-linux-gnu/gvfs"
# HOME and the XDG base directories are dropped on purpose: the app process'
# own values point at /data/user/0/<package>, and dconf happily creates
# $HOME/.cache/dconf - which the shim then creates *inside* the rootfs as
# rootfs/data/user/0/..., i.e. host-shaped dirt in the guest (measured).
adb -s "$DEV" shell "run-as $PKG sh -c 'R=\$PWD/$ROOTFS; L=\$PWD/$LD; unset XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME; cd \"\$R\" && HOME=/root EXPECT_PKGS=${EXPECT_PKGS:-811} PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin NR_ROOTFS=\$R NR_LD=\$L exec \$L --library-path \"\$R/$LIBS\" usr/bin/bash /tmp/husk-accept.sh'" 2>&1
