# /etc/profile.d/husk-gpu.sh - GPU 运行时默认值（由 Husk 安装器写入）
#
# 目的：**不需要给单个应用配置**。登录 shell 一起手就把 Vulkan ICD 与 GL 驱动指对，
# 之后这个 shell 里启动的任何程序（GTK、Electron、python、游戏）都继承同一份配置。
# 这和"每个应用单独装一套驱动"是两件事：驱动只部署一份，配置只写一次。
#
# 为什么必须有环境变量、不能纯靠"文件放对就行"：
#   * 同一个 rootfs 里装了上游 `mesa-vulkan-drivers` 的全套 ICD（asahi/broadcom/intel/
#     radeon/freedreno/lvp/panfrost...），Vulkan loader 会挨个试；其中 `panfrost_icd.json`
#     指向的是**没有 x11 平台**的库，会和我们的 ICD 抢设备。所以必须显式选。
#   * 我们的 PanVK 只编了 Vulkan（`-Dgallium-drivers=`），没有 gallium panfrost，
#     所以 GL 必须走 zink -> Vulkan。`MESA_LOADER_DRIVER_OVERRIDE=zink` 就是这个意思。
#
# 逃逸开关（任一即可回到软件渲染，用于 bisect 渲染问题或在没有 ICD 的树上跑）：
#   HUSK_GPU=llvmpipe        或        NR_GPU_DEFAULT_OFF=1
#
# 已经自己 export 过同名变量的，这里一律不覆盖。

if [ "${HUSK_GPU:-zink}" = llvmpipe ] || [ -n "${NR_GPU_DEFAULT_OFF:-}" ]; then
    return 0 2>/dev/null || true
fi

if [ -z "${VK_ICD_FILENAMES:-}" ] && [ -z "${VK_DRIVER_FILES:-}" ]; then
    for _husk_icd in /usr/share/vulkan/icd.d/husk_*.json; do
        case "$_husk_icd" in *.bak*) continue ;; esac
        if [ -r "$_husk_icd" ]; then
            export VK_ICD_FILENAMES="$_husk_icd"
            export VK_DRIVER_FILES="$_husk_icd"
            break
        fi
    done
    unset _husk_icd
fi

[ -z "${MESA_LOADER_DRIVER_OVERRIDE:-}" ] && export MESA_LOADER_DRIVER_OVERRIDE=zink
[ -z "${GALLIUM_DRIVER:-}" ] && export GALLIUM_DRIVER=zink
# GLES 版本上限：这一版组合是 3.2（docs/LIMITATIONS.md）。写在这里是为了让
# "桌面 + 里面的应用"看到同一个上限，而不是每个启动脚本各写一遍。
[ -z "${MESA_GLES_VERSION_OVERRIDE:-}" ] && export MESA_GLES_VERSION_OVERRIDE=3.2
