#!/bin/bash
# 31 functional checks, run *inside* the guest:
#   PATH=/usr/bin:/bin, cwd=/, one process per exec, shim lives in the loader.
# Driven from the host by run-guest.sh (see README). No LD_PRELOAD anywhere.
ok(){ echo "PASS $1"; n=$((${n:-0}+1)); }
bad(){ echo "FAIL $1 -- $2"; f=$((${f:-0}+1)); }
chk(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }
# 别 `rm -rf /tmp`：桌面会话的 X socket（`/tmp/.X11-unix/X0`）就住在这里，删掉它
# 之后 X server 手里那个监听 fd 变成 unlink 过的孤儿，已有连接还能画、新客户端一律
# `unable to open display :0`（2026-11-27 实测，defect H47）。只清本脚本自己造的名字。
if pgrep -f 'usr/bin/gnome-shell' >/dev/null 2>&1; then
    echo "FAIL guarded -- a desktop session is running; this script deletes /tmp names a live session needs" >&2
    echo "#checks=0 fail=1"; exit 1
fi
rm -rf /tmp/n1 /tmp/d1 /tmp/d2 /tmp/fx /a /esc 2>/dev/null; mkdir -p /tmp
mkdir -p /tmp/n1/n2/n3 && chk mkdirp "$(cd /tmp/n1/n2/n3 && pwd -P)" /tmp/n1/n2/n3
chk mkdirp-place "$(test -d /tmp/n1/n2/n3 && test ! -d /tmp/n3 && echo clean)" clean
echo deep > /tmp/n1/n2/n3/f && chk deepwrite "$(cat /tmp/n1/n2/n3/f)" deep
mkdir -p /tmp/d1 /tmp/d2
echo hello > /tmp/d1/a && chk redirect "$(cat /tmp/d1/a)" hello
cp /etc/os-release /tmp/d1/b && chk cp "$(grep -c '^NAME=' /tmp/d1/b)" 1
mv /tmp/d1/b /tmp/d1/c && chk mv "$(grep '^NAME=' /tmp/d1/c)" NAME\=\"Ubuntu\"
chmod 640 /tmp/d1/c && chk chmod "$(stat -c %a /tmp/d1/c)" 640
ln -s a /tmp/d1/l && chk readlink "$(readlink /tmp/d1/l)" a
chk symlink-follow "$(cat /tmp/d1/l)" hello
ln -s /etc/os-release /tmp/d1/abs && chk symlink-abs "$(grep -c ID=ubuntu /tmp/d1/abs)" 1
ln -s ../d1/c /tmp/d2/up && chk symlink-dotdot "$(grep '^NAME=' /tmp/d2/up)" NAME\=\"Ubuntu\"
chk find-files "$(find /tmp/d1 -type f | wc -l | tr -d " ")" 2
chk find-links "$(find /tmp/d1 -type l | wc -l | tr -d " ")" 2
chk readdir "$(ls -a /tmp/d1 | grep -vc "^\.\.\?$")" 4
chk ls-long "$(ls -l /tmp/d1/c | cut -c1-10)" -rw-r-----
chk pwd "$(cd /usr/bin && pwd -P)" /usr/bin
chk pwd-root "$(cd / && pwd -P)" /
chk dotdot "$(cd /etc && cd .. && pwd -P)" /
chk dotdot-read "$(grep '^NAME=' /../../etc/os-release)" NAME\=\"Ubuntu\"
chk escape-link "$(cat /evil 2>/dev/null; echo rc=$?)" rc=1
chk escape-write "$(mkdir -p /../esc 2>/dev/null; test -d /esc && echo y)" y
chk rm-rf "$(rm -rf /tmp/d1 /tmp/d2 && test ! -e /tmp/d1 && echo y)" y
chk sort "$(printf "3\n1\n2\n" | sort | head -1)" 1
chk sha256 "$(printf hello | sha256sum | cut -c1-8)" 2cf24dba
chk glob "$(cd /etc && ls os-release* | head -1)" os-release
chk stdin "$(echo pipe | cat)" pipe
chk subshell "$( (cd /etc; grep '^NAME=' os-release) )" NAME\=\"Ubuntu\"
chk exec-abs "$(/usr/bin/grep '^NAME=' /etc/os-release)" NAME\=\"Ubuntu\"
chk exec-rel "$(cd /etc && /usr/bin/grep '^NAME=' os-release)" NAME\=\"Ubuntu\"
chk bigio "$(yes abcdefghij | head -2000 > /tmp/big; wc -c < /tmp/big)" 22000
rm -rf /tmp/big
chk write-read "$(echo abc > /tmp/w && cat /tmp/w && rm /tmp/w)" abc
echo "#checks=$n fail=${f:-0}"
[ "${f:-0}" -eq 0 ] || exit 1
