#!/bin/sh
# hook-coverage.sh - 把「需要路径翻译的 libc 符号」和 path-shim 实际导出/实现的符号做差。
#
# 背景：path-shim 是按函数名改写 GOT/PLT 的，覆盖范围等于「导出符号表」。
# 漏一个符号 = 那个函数拿到的 guest 路径会原封不动交给 Android 内核，
# 要么 ENOENT，要么命中宿主 /（静默错）。这个脚本把「漏了哪些」变成可枚举的输出。
#
# usage: sh tools/dev/hook-coverage.sh [path-shim 源目录]
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CRATE=${1:-$ROOT/runtime/crates/path-shim/src}
SYMS="$ROOT/tools/dev/path-symbols.txt"
[ -f "$SYMS" ] || { echo "missing $SYMS" >&2; exit 2; }

grep -ho 'pub unsafe extern "C" fn [a-zA-Z0-9_]*' "$CRATE"/*.rs 2>/dev/null \
  | awk '{print $6}' | sort -u > /tmp/husk-hooks.txt
grep -v '^#' "$SYMS" | grep -v '^$' | sort -u > /tmp/husk-need.txt

have=$(comm -12 /tmp/husk-need.txt /tmp/husk-hooks.txt | wc -l | tr -d ' ')
miss=$(comm -23 /tmp/husk-need.txt /tmp/husk-hooks.txt | wc -l | tr -d ' ')

echo "need=$have+$miss hooks=$(wc -l < /tmp/husk-hooks.txt | tr -d ' ')"
echo "--- 已覆盖 ($have)"
comm -12 /tmp/husk-need.txt /tmp/husk-hooks.txt | tr '\n' ' ' | fold -w 160
echo
echo "--- 未覆盖 ($miss)"
comm -23 /tmp/husk-need.txt /tmp/husk-hooks.txt | tr '\n' ' ' | fold -w 160
echo
