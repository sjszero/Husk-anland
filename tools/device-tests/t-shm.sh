#!/bin/bash
export DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/runtime-husk HOME=/root
G=/etc/husk/gpu-env
cp -f $G $G.bak-exp 2>/dev/null
sed -i 's/^!MESA_LOADER_DRIVER_OVERRIDE=zink/!MESA_LOADER_DRIVER_OVERRIDE=panfrost/; s/^!GALLIUM_DRIVER=zink/!GALLIUM_DRIVER=panfrost/' $G
echo "=== 改后的 gpu-env ==="; grep -E "MESA_LOADER|GALLIUM" $G
echo "=== 直接跑 panfrost Gallium ==="
/usr/bin/glxinfo -B 2>&1 | grep -E "OpenGL renderer|OpenGL version|failed|error|Error|swrast|Panfrost|kms" | head -6
echo "=== 离屏 2400x1080 渲染（pbuf）==="
/root/gl-probe pbuf 2400x1080 20 2>&1 | tail -2
echo "=== 满屏帧率 ==="
timeout 9 /usr/bin/glxgears -geometry 2400x1080 2>/dev/null | grep "frames in" | head -2
echo "=== 恢复 ==="
mv -f $G.bak-exp $G && grep -E "MESA_LOADER|GALLIUM" $G
echo DONE
