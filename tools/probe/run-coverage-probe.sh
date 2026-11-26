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

# The Makefile invocation passes workspace-relative device paths. Normalize them
# before they cross into the guest session; keeping them relative makes failures
# depend on run-as's current directory and used to surface as an opaque rc=127.
device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$HERE/coverage-probe.c" /data/local/tmp/coverage-probe.c >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/coverage-probe.c $ROOTFS/root/coverage-probe.c'"
if ! BUILD=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/gcc -O1 -o /root/p /root/coverage-probe.c" 2>&1); then
    echo '--- coverage-probe: compile failed' >&2
    echo "$BUILD" >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/p" 2>&1); then
    echo '--- coverage-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi

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
want '^HARDLINK_STDIO link_rc=0 link_errno=0 content=AAABBB' '硬链接复制后备跟随 stdio fclose'
want '^CREATE_MODE open_fd=[0-9]* errno=[0-9]* mode=440 write=1 expected=440' 'O_CREAT 保留 caller 请求的模式'
want '^ACCESS_ROOT read=0/[0-9]* write=0/[0-9]* exec0=-1/13 exec5=0/[0-9]*' '假 root 的 access 读写与执行位语义'
want '^ACCESS_USER parent=-1/13 owned=0/[0-9]*' '非 root 的 access 检查父目录与虚构 owner'
want '^OWNER_RESTORE open_fd=[0-9]* errno=[0-9]* mode_open=440 write=1 mode_final=440' '假 root 写放宽后恢复原文件模式'
want '^NETLINK_DUP before=[0-9]* opened=[0-9]* families=16,16,16,16 dup=16,16,16 final=16,16,16 after=[0-9]*' \
     'netlink dup/dup2/fcntl 在逐级关闭后仍是 AF_NETLINK'
want '^CLOSE_RANGE_NETLINK before=[0-9]* opened=[0-9]* rc=0 errno=0 after=[0-9]*' \
     'close_range 用户态兜底关闭 netlink client 与 peer'
want '^CLOSE_RANGE_PIPE pipe=0 rc=0 errno=0 fd0=-1 fd1=-1' 'close_range 关闭普通 fd'
want '^CLOSE_RANGE_CLOEXEC pipe=0 rc=0 errno=0 fd0=1 fd1=1' 'close_range CLOEXEC 不关闭 fd'
want '^CLOSE_RANGE_UNSHARE rc=-1 errno=95'             'close_range UNHARE 明确拒绝而非伪成功'
want '^CLOSE_RANGE_ALL child=[0-9]* exit=0 exists=1'  'close_range(3,~0) 不遍历四亿号且保留 rootfd'
want '^RAW_OPENAT fd=-1 errno=2'                   'raw syscall 仍然绕过（已知边界，不是 bug）'
want '^SCANDIRAT n=[1-9][0-9]* errno=0'             'scandirat 翻译（自己实现的扫描体）'

nl=$(echo "$OUT" | grep '^CLOSE_RANGE_NETLINK before=' | tail -1)
if [ -n "$nl" ]; then
    before=$(echo "$nl" | sed -n 's/.*before=\([0-9]*\).*/\1/p')
    opened=$(echo "$nl" | sed -n 's/.*opened=\([0-9]*\).*/\1/p')
    after=$(echo "$nl" | sed -n 's/.*after=\([0-9]*\).*/\1/p')
    if [ "$opened" -ne $((before + 2)) ] || [ "$after" -ne "$before" ]; then
        printf 'FAIL close_range netlink fd 净变化   before=%s opened=%s after=%s\n' \
            "$before" "$opened" "$after"
        fail=1
    else
        printf 'ok   close_range netlink client+peer 无残留\n'
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo '--- coverage-probe: FAIL' >&2
    exit 1
fi
echo '--- coverage-probe: PASS'
