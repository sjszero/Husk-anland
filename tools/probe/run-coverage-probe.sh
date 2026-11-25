#!/bin/sh
# run-coverage-probe.sh - 在设备上验「这些 libc 符号到底有没有被路径翻译」。
#
# 为什么要有它：path-shim 按符号名改写 GOT/PLT，覆盖范围等于符号表，
# 漏一个不会报错，只会**静默走错路**。这个探针把每个候选符号都跑一遍，
# 用「文件落在 guest 里还是宿主里」判定，结果和期望不一致就 exit 1。
#
# usage: sh tools/probe/run-coverage-probe.sh <device-serial> [rootfs] [loader]
#   默认值取自 LinuxHub 2.0 安装树（可用环境变量 ROOTFS/LD 覆盖）。
set -eu
DEV=${1:?device serial}
PKG=${PKG:-com.linuxhub.myth.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/distributions/ubuntu-24.04/rootfs}
LD=${LD:-$APPDIR/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
SESS=${SESS:-/data/local/tmp/husk-session.sh}

adb push "$HERE/coverage-probe.c" /data/local/tmp/coverage-probe.c >/dev/null
adb shell "run-as $PKG sh -c 'cp /data/local/tmp/coverage-probe.c $ROOTFS/root/coverage-probe.c'"
adb shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/gcc -O1 -o /root/p /root/coverage-probe.c" >/dev/null 2>&1
OUT=$(adb shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/p" 2>&1)

echo "$OUT"

fail=0
want() { # want <grep 模式> <说明>
    if echo "$OUT" | grep -q "$1"; then
        printf 'ok   %s\n' "$2"
    else
        printf 'FAIL %s   (期望匹配: %s)\n' "$2" "$1"
        fail=1
    fi
}

want '^CTL_UNLINK rc=0 errno=0 exists_after=0'      'unlink 翻译（对照组）'
want '^REMOVE rc=0 errno=0 exists_after=0'         'remove(3) 翻译（glibc 内部调用点）'
want '^INOTIFY fd=[0-9]* wd=[0-9]* errno=0'        'inotify_add_watch 翻译（GIO 文件监视）'
want '^INOTIFY_EVENT n=[1-9]'                      'inotify 事件真的送到 guest'
want '^FREOPEN ok=1'                               'freopen 翻译'
want '^PATHCONF v=255'                              'pathconf 翻译'
want '^TRUNCATE64 rc=0 errno=0 size=3'             'truncate64 别名'
want '^UTIME rc=0 errno=0'                          'utime(3) 翻译'
want '^RAW_OPENAT fd=-1 errno=2'                   'raw syscall 仍然绕过（已知边界，不是 bug）'
want '^SCANDIRAT n=[1-9][0-9]* errno=0'             'scandirat 翻译（自己实现的扫描体）'

if [ "$fail" -ne 0 ]; then
    echo '--- coverage-probe: FAIL' >&2
    exit 1
fi
echo '--- coverage-probe: PASS'
