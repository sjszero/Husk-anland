#!/bin/bash
# 24 的运行时段：loader 自己扫 icd.d 时也必须选中 canonical 驱动。
export PATH=/usr/bin:/bin:$PATH
echo "== vulkan-probe with the environment ICD pins removed (loader must scan icd.d) =="
env -u VK_ICD_FILENAMES -u VK_DRIVER_FILES /usr/bin/vulkan-probe 2>&1 | grep -E "^(vk|VK)" | head -8
echo "== vulkan-probe with the legacy manifest name the host pins (H40 path) =="
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json \
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json \
  /usr/bin/vulkan-probe 2>&1 | grep -E "^(vk|VK)" | head -8
