#!/bin/sh
# ES3 真 workload 探针（guest 内运行）：VAO + 实例化 divisor + std140 UBO + 纹理采样 + MRT。
#
# 主机侧先把源码推进 rootfs：
#   adb push tools/device-tests/es3-workload.c <rootfs>/tmp/es3-workload.c
# 然后在一个会话里跑：
#   ./husk run ubuntu -c 'sh /tmp/es3-workload.sh'
#
# 两个配置各跑一次，用来区分「声明上限」和「实际能力」：
#   默认（zink）                       -> 需要 MESA_GLES_VERSION_OVERRIDE=3.2 才有 ES3 config
#   GALLIUM_DRIVER=llvmpipe            -> 对照组，它自己就报 ES3.2
#
# 判定：att0 每个纹理色各出现 64 次（4 实例 × 16 像素）、异常像素 0；att1 == 1-att0。
# 逐像素 1x1 读回是故意的：这棵树的 glReadPixels 整块读只写第一行+第一列（evidence 162）。
set -u
SRC=${1:-/tmp/es3-workload.c}
[ -f "$SRC" ] || { echo "找不到 $SRC" >&2; exit 64; }
gcc -O1 -w -o /tmp/es3-workload "$SRC" -ldl || exit 1

echo '--- 默认（zink 声明的等级）---'
/tmp/es3-workload; default_rc=$?
echo "--- 默认 rc=$default_rc ---"
echo '--- MESA_GLES_VERSION_OVERRIDE=3.2 ---'
MESA_GLES_VERSION_OVERRIDE=3.2 /tmp/es3-workload; override_rc=$?
echo "--- override rc=$override_rc ---"
echo '--- 对照：GALLIUM_DRIVER=llvmpipe ---'
GALLIUM_DRIVER=llvmpipe /tmp/es3-workload; llvmpipe_rc=$?
echo "--- llvmpipe rc=$llvmpipe_rc ---"
