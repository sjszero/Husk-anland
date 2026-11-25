#!/bin/sh
F=/data/user/0/com.linuxhub.myth.husk/files/linux
R=$F/distributions/ubuntu-24.04/rootfs
L=$R/tmp/imtest.log
: > "$L"
G() { sh $F/xe-plain.sh usr/bin/env DISPLAY=:0 HOME=/root XDG_RUNTIME_DIR=/tmp/xdg-husk \
      DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/xdg-husk/bus XMODIFIERS=@im=fcitx \
      GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx "$@" </dev/null; }

printf '' > "$R/root/t.txt"
echo "== start editor ==" >> "$L"
( cd "$R" && exec 0</dev/null 1>>"$L" 2>&1
  nohup env -u XDG_CACHE_HOME HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    NR_ROOTFS=$R NR_LD=$F/husk/libld.so NR_LIBRARY_PATH="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu" \
    DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/xdg-husk DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/xdg-husk/bus \
    XMODIFIERS=@im=fcitx GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx \
    LANG=zh_CN.UTF-8 LANGUAGE=zh_CN:zh \
    $F/husk/libld.so --library-path "$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu" \
    usr/bin/gnome-text-editor /root/t.txt >/dev/null 2>&1 & )

i=0; win=
while [ $i -lt 20 ]; do
  win=$(G usr/bin/xdotool search --name 't\.txt' 2>/dev/null | head -1)
  [ -n "$win" ] && break
  sleep 1; i=$((i+1))
done
echo "window=$win after ${i}s" >> "$L"
[ -n "$win" ] || { echo "no window" >> "$L"; cat "$L"; exit 1; }

G usr/bin/xdotool windowactivate --sync "$win" >/dev/null 2>&1
sleep 1
echo -n "state-before=" >> "$L"; G usr/bin/fcitx5-remote >> "$L" 2>&1; echo >> "$L"
G usr/bin/xdotool key --clearmodifiers ctrl+space >/dev/null 2>&1
sleep 1
echo -n "state-after-toggle=" >> "$L"; G usr/bin/fcitx5-remote >> "$L" 2>&1; echo >> "$L"
echo -n "im-name=" >> "$L"; G usr/bin/fcitx5-remote -n >> "$L" 2>&1; echo >> "$L"
G usr/bin/xdotool type --delay 120 'nihao' >/dev/null 2>&1
sleep 2
G usr/bin/xdotool key --clearmodifiers space >/dev/null 2>&1
sleep 1
G usr/bin/xdotool key --clearmodifiers ctrl+s >/dev/null 2>&1
sleep 3
echo "== file bytes ==" >> "$L"
od -An -tx1 "$R/root/t.txt" | head -3 >> "$L"
echo "== file text ==" >> "$L"
cat "$R/root/t.txt" >> "$L"; echo >> "$L"
cat "$L"
