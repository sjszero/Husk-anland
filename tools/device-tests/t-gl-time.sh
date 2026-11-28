#!/bin/bash
export DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/runtime-husk HOME=/root
icd=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink VK_ICD_FILENAMES=$icd VK_DRIVER_FILES=$icd PANVK_DXVK_COMPAT=0
echo "--- 试加 1600x900 模式 ---"
/usr/bin/xrandr --newmode "1600x900_60" 0 1 1 1 1 2>&1 | head -2
/usr/bin/xrandr --addmode builtin "1600x900_60" 2>&1 | head -2
/usr/bin/xrandr --output builtin --mode "1600x900_60" 2>&1 | head -2
/usr/bin/xdpyinfo 2>/dev/null | grep dimensions
/usr/bin/xrandr 2>&1 | head -6
echo "--- 恢复 2400x1080 ---"
/usr/bin/xrandr --output builtin --mode 2400x1080 2>&1 | head -2
/usr/bin/xdpyinfo 2>/dev/null | grep dimensions
echo DONE
