#!/bin/bash
# 25-self-exe-exec.sh — `/proc/self/exe` 的 execve 语义（H51 / evidence 362）
#
# 加载器模型下 `/proc/self/exe` 指向 libld.so。`readlink` 那一侧 evidence 217
# 已经换成 guest 程序路径，`execve` 那一侧原来没有：Chrome / Electron 的
# zygote、network service 都是 `execve("/proc/self/exe", ["--type=utility"])`，
# 直接交给内核就会把加载器当程序跑（`-c: error while loading shared libraries`）。
#
# 期望：execve("/proc/self/exe") 重新进入**本进程自己的 guest 程序**。
export PATH=/usr/bin:/bin:$PATH
PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf 'ok   %s\n' "$*"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$*"; }

out=$(/usr/bin/env bash -c 'exec /proc/self/exe -c "echo INNER_SELF_EXEC_OK"' 2>&1)
case "$out" in
    *INNER_SELF_EXEC_OK*) ok "exec /proc/self/exe 重新进入 bash" ;;
    *)                    bad "exec /proc/self/exe 没有重新进入 bash: ${out:-<empty>}" ;;
esac
case "$out" in
    *"cannot open shared object"*|*"error while loading shared libraries"*|*"unrecognized option '--type="*)
        bad "加载器被当成程序跑了: $out" ;;
    *) ok "没有把加载器当程序跑" ;;
esac

rl=$(readlink /proc/self/exe 2>/dev/null)
case "$rl" in
    /*) ok "readlink /proc/self/exe 给 guest 绝对路径: $rl" ;;
    *)  bad "readlink /proc/self/exe 给了怪路径: ${rl:-<empty>}" ;;
esac

# 负向：带一个本程序不认识的选项时，报错必须来自**该程序自己**（bash 的
# getopt），而不是加载器的选项解析。
out2=$(/usr/bin/env bash -c 'exec /proc/self/exe --type=utility' 2>&1)
case "$out2" in
    *"invalid option"*) ok "选项错误来自程序自己（bash 的 getopt）" ;;
    *) bad "选项错误不是程序自己报的: ${out2:-<empty>}" ;;
esac

echo "SELF_EXE PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
