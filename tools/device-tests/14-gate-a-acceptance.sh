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
P=0; F=0; S=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }
skip(){ S=$((S+1)); echo "  SKIP  $1"; }
eq()  { [ "$2" = "$3" ] && ok "$1 ($3)" || bad "$1 (want '$2' got '$3')"; }
re()  { printf '%s\n' "$3" | grep -Eq "$2" && ok "$1 ($3)" || bad "$1 (want /$2/ got '$3')"; }

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
# 这两条只读 /etc/shadow 和 /etc/group，用 cut/case 而不是 awk/getent：
# 最小树里既可能没有 mawk 也可能没有 libc-bin（getent 在里面），2026-11-26 第一次
# 在干净树上跑，两条都因为 `awk: not found` 拿到空串而报 FAIL —— 是断言依赖了
# 树里的包，不是 usermgr 写错了。
sh_line=$(grep -m1 '^nrtest:' /etc/shadow 2>/dev/null || true)
case "$sh_line" in
  *'$6$'*) eq "shadow hash" "sha512" "sha512" ;;
  *)       bad "shadow hash (want sha512 got '${sh_line%%:*}' no-\$6\$)" ;;
esac
eq "group entry" "nrtest" "$(grep -m1 '^[^:]*:[^:]*:1500:' /etc/group | cut -d: -f1)"
groupdel nrtest 2>/dev/null
userdel -r nrtest 2>/dev/null
eq "cleanup" "gone" "$(id nrtest >/dev/null 2>&1 && echo present || echo gone)"

echo "== dpkg =="
eq "dpkg -C clean before" "0" "$(dpkg -C >/tmp/dpkgC.1 2>&1; wc -l < /tmp/dpkgC.1 | tr -d ' ')"
# Package count is a property of the rootfs, not of the runtime, so the floor
# must match the kind of tree under test:
#   bare ubuntu-base tarball  -> ~93 packages   -> EXPECT_PKGS_MIN=90  (default)
#   full desktop distro tree  -> ~700-900       -> EXPECT_PKGS_MIN=700 must be set
# The default is the base floor. A desktop tree run without the override still
# passes (more packages), so the override only guards against a truncated
# desktop tree; state the number explicitly instead of hiding it in a magic 700.
pkg_n=$(dpkg-query -W -f='${Status}\n' | grep -c 'install ok installed')
PKGS_MIN=${EXPECT_PKGS_MIN:-90}
[ "$pkg_n" -ge "$PKGS_MIN" ] \
  && ok "installed packages ($pkg_n >= $PKGS_MIN)" \
  || bad "installed packages (want >= $PKGS_MIN got $pkg_n)"
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
# acl 是树里的包，不是运行时的能力：装了才断言，没装就 SKIP 并写清原因，
# 不要把一个"树里没这个包"变成 FAIL（那会逼着人去放宽断言）。
if dpkg-query -W -f='${Status}' acl 2>/dev/null | grep -q 'install ok installed'; then
  eq "getfacl still runs" "ok" "$(getfacl --version >/dev/null 2>&1 && echo ok || echo fail)"
else
  skip "getfacl (this tree has no acl package)"
fi

echo "== apt =="
# 版本号写成正则：干净树装的是 noble-updates 的 2.8.4，写死 2.8.3 只在 11-25 那棵
# 树上成立，换一棵树就变成假失败（2026-11-26 实测）。
aptv=$(apt --version 2>/dev/null); aptv=${aptv#apt }; aptv=${aptv%% *}
re "apt version" '^2\.8\.[0-9]+$' "$aptv"
# apt-cache policy / apt-get -s 都要读包索引；离线树没有 /var/lib/apt/lists 时它们
# 按设计失败，那是树的状态而不是运行时缺陷 —— 明确 SKIP 并打出原因。
# "有索引"要按真的索引文件判：/var/lib/apt/lists 里通常只有一个 partial/ 目录，
# 用 `ls -A` 判空会被它骗过去（2026-11-26 实测：ls -A 非空，但没有任何 *_Packages）。
if ls /var/lib/apt/lists/*_Packages* >/dev/null 2>&1 || ls /var/lib/apt/lists/*InRelease* >/dev/null 2>&1; then
  pol=$(apt-cache policy acl 2>/dev/null | sed -n 's/^  Installed: //p')
  eq "policy knows acl" "yes" "$([ -n "$pol" ] && [ "$pol" != "(none)" ] && echo yes || echo no)"
  echo "  apt-cache policy acl -> Installed: ${pol:-<none>}"
  # apt-get -s 的退出码要分开读：离线树里"要下的 deb 不在本地"是树的状态
  # （rc=100 + cannot be downloaded），求解器或运行时坏掉是另一回事。
  apt-get -s install --reinstall acl >/tmp/apt-dry.log 2>&1
  rc_s=$?
  if [ "$rc_s" -eq 0 ]; then
    ok "apt-get -s rc (0)"
  elif grep -qi 'cannot be downloaded\|Unable to fetch\|Failed to fetch' /tmp/apt-dry.log; then
    skip "apt-get -s (rc=$rc_s: no downloadable .deb on this offline tree)"
  else
    bad "apt-get -s rc (want '0' got '$rc_s')"
  fi
else
  skip "apt-cache policy / apt-get -s (no package index in /var/lib/apt/lists)"
fi

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
# first written against: hard-coding another id made *our own* processes count as
# a leak (measured 2026-11-26: the count was 1 on a tree with no other app running
# at all). HUSK_PKG comes from the host invocation. No other package id is
# written here on purpose - tools/ci/audit-runtime-package.sh rejects a retired id
# anywhere in a live script, and "anything that is not the package under test is a
# leak" is the invariant this check is supposed to state.
own=${HUSK_PKG:-com.linuxhub.husk}
leaked=$(ps -e -o args= 2>/dev/null | grep -oE 'com\.[a-z0-9_.]+' \
           | grep -v -x -e "$own" | sort -u || true)
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
echo "PASS=$P FAIL=$F SKIP=$S"
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
[ -f "$R/etc/ld.so.husk-path" ] && E=$(sed -n 's|^|'"$R"'|p' "$R/etc/ld.so.husk-path" | tr '\n' ':')
cd "$R" || exit 1
unset XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
# NR_LIBRARY_PATH is the launcher-to-shim contract for *nested* execs (the app
# exports it). Without it a child falls back to the shim's built-in list plus
# /etc/ld.so.husk-path, which is close but not what the app does - and the gate
# has to measure the app's launch, not a cousin of it.
LP="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu:$E"
if [ -f "$R/etc/ld.so.husk-path" ]; then
    while IFS= read -r d; do
        case "$d" in ''|'#'*) continue ;; esac
        expected=$R$d
        case ":$LP:" in
            *":$expected:"*) ;;
            *) echo "husk-exec: malformed library path for $d (expected $expected)" >&2; exit 78 ;;
        esac
    done < "$R/etc/ld.so.husk-path"
fi
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
# EXPECT_PKGS_MIN 必须真的传进去：上一个版本只在 guest 里 `EXPECT_PKGS_MIN:-90`
# 默认，宿主的 EXPECT_PKGS_MIN=... 被丢在半路，最小树于是永远报 FAIL（2026-11-26）。
adb -s "$DEV" shell "run-as $PKG env EXPECT_PKGS=${EXPECT_PKGS:-811} EXPECT_PKGS_MIN=${EXPECT_PKGS_MIN:-90} HUSK_PKG=$PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD usr/bin/bash /tmp/husk-accept.sh" 2>&1
