#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
GP=/root/gl-probe
echo "=== zink dumpmips"
$GP scale-wedge 348x156 dumpmips 2>&1 | grep -E "renderer|mip level|FAIL"
echo "=== llvmpipe(HUSK_GPU) mip"
env HUSK_GPU=llvmpipe $GP scale-wedge 348x156 mip 2>&1 | grep -E "renderer|variant|dark_cells|near_black|FAIL"
echo "=== llvmpipe(HUSK_GPU) dumpmips"
env HUSK_GPU=llvmpipe $GP scale-wedge 348x156 dumpmips 2>&1 | grep -E "renderer|mip level|FAIL"
