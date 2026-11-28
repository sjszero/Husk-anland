#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
GP=/root/gl-probe
echo "=== zink cpumips"
$GP scale-wedge 348x156 cpumips 2>&1 | grep -E "renderer|cpumips|dark_cells|near_black|FAIL"
echo "=== zink dumpmips (writes /tmp/mip-lv*.ppm)"
$GP scale-wedge 348x156 dumpmips 2>&1 | grep -E "mip level|FAIL"
ls -la /tmp/mip-lv*.ppm 2>/dev/null
