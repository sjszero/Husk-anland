#!/bin/sh
# ES3 真 workload 探针（guest 内运行）：VAO + 实例化 divisor + std140 UBO + 纹理采样 + MRT。
# 二进制来自 runtime/crates/gl-probe（Rust），由 build-guest-tools.sh 构建：
#   sh tools/guest-config/build-guest-tools.sh
#   adb push runtime/target/aarch64-unknown-linux-gnu/release/gl-probe <rootfs>/tmp/
# 然后在一个会话里跑：
#   ./husk run ubuntu -c 'sh /tmp/es3-workload.sh'
#
# 三个配置各跑一次，用来区分「声明上限」和「实际能力」：
#   默认（zink）                       -> 需要 MESA_GLES_VERSION_OVERRIDE=3.2 才有 ES3 config
#   GALLIUM_DRIVER=llvmpipe            -> 对照组，它自己就报 ES3.2
#
# 判定：att0 每个纹理色各出现 64 次（4 实例 × 16 像素）、异常像素 0；att1 == 1-att0。
# 逐像素 1x1 读回是故意的：这棵树的 glReadPixels 整块读只写第一行+第一列（evidence 162）。
set -u
BIN=${1:-/tmp/gl-probe}
[ -x "$BIN" ] || { echo "找不到 $BIN（先推 gl-probe）" >&2; exit 64; }

echo '--- 默认（zink 声明的等级）---'
"$BIN" es3; default_rc=$?
echo "--- 默认 rc=$default_rc ---"
echo '--- MESA_GLES_VERSION_OVERRIDE=3.2 ---'
MESA_GLES_VERSION_OVERRIDE=3.2 "$BIN" es3; override_rc=$?
echo "--- override rc=$override_rc ---"
echo '--- 对照：GALLIUM_DRIVER=llvmpipe ---'
GALLIUM_DRIVER=llvmpipe "$BIN" es3; llvmpipe_rc=$?
echo "--- llvmpipe rc=$llvmpipe_rc ---"
