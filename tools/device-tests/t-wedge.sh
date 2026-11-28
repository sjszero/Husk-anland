#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
GP=/root/gl-probe
echo "=== zink / mip"
$GP scale-wedge 348x156 mip 2>&1 | grep -E "renderer|variant|dark_cells|near_black|glGetError|FAIL"
echo "=== zink / nomip"
$GP scale-wedge 348x156 nomip 2>&1 | grep -E "dark_cells|near_black|glGetError|FAIL"
echo "=== zink / nearest"
$GP scale-wedge 348x156 nearest 2>&1 | grep -E "dark_cells|near_black|FAIL"
echo "=== zink / list(mip)"
$GP scale-wedge 348x156 list 2>&1 | grep -E "dark_cells|near_black|FAIL"
echo "=== zink / mip-noblend"
$GP scale-wedge 348x156 mip-noblend 2>&1 | grep -E "dark_cells|near_black|FAIL"
echo "=== llvmpipe / mip"
env LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=swrast $GP scale-wedge 348x156 mip 2>&1 | grep -E "renderer|dark_cells|near_black|FAIL"
