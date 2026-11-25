# /etc/profile.d/husk-gpu.sh - GPU 运行时默认值（由 Husk 安装器写入）
#
# 目标：不需要给单个应用配置。登录 shell 一开始就把 Vulkan ICD 与 GL 驱动指对，
# 之后这个 shell 里启动的程序共享同一份硬件接口；非登录进程由加载器读
# /etc/husk/gpu-env，使用同一套决策。
#
# 这个脚本会按设备节点选择家族，不按机型名：
#   Mali + kbase   -> PanVK（Vulkan）-> zink（GL）
#   Adreno + KGSL  -> Turnip/freedreno；有原生 Gallium 驱动时走 kgsl，否则 zink
#   真 DRM node    -> 不覆盖，交给发行版 Mesa
#   无可用节点     -> 明确 llvmpipe
#
# 逃逸开关：HUSK_GPU=llvmpipe，或 NR_GPU_DEFAULT_OFF 取真值（1/true/yes/on）。
# NR_GPU_DEFAULT_OFF=0/false/no/off/空串不关闭。程序自己 export 的同名值优先。

_husk_gpu_off() {
    [ "${HUSK_GPU:-}" = llvmpipe ] && return 0
    case "${NR_GPU_DEFAULT_OFF:-}" in
        ""|0|false|FALSE|False|no|NO|off|OFF) return 1 ;;
        *) return 0 ;;
    esac
}

_husk_gpu_soft() {
    export LIBGL_ALWAYS_SOFTWARE=1
    export GALLIUM_DRIVER=llvmpipe
    export MESA_LOADER_DRIVER_OVERRIDE=swrast
    if [ -r /usr/share/vulkan/icd.d/lvp_icd.json ]; then
        export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
        export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
    fi
}

if _husk_gpu_off; then
    _husk_gpu_soft
    unset -f _husk_gpu_off _husk_gpu_soft 2>/dev/null || true
    return 0 2>/dev/null || true
fi

_husk_gpu_family=software
if ( : <> /dev/mali0 ) 2>/dev/null; then
    _husk_gpu_family=mali-kbase
elif ( : <> /dev/kgsl-3d0 ) 2>/dev/null; then
    _husk_gpu_family=adreno-kgsl
elif ( : <> /dev/dri/renderD128 ) 2>/dev/null; then
    _husk_gpu_family=drm-node
fi

_husk_icd=
case "$_husk_gpu_family" in
    mali-kbase)
        # The installer creates this manifest with a guest-relative
        # library_path. Never fall back to arbitrary husk_*.json files: some of
        # them contain host /data/data/... paths and crash the Vulkan loader.
        if [ -r /usr/share/vulkan/icd.d/husk_panvk_guest.json ]; then
            _husk_icd=/usr/share/vulkan/icd.d/husk_panvk_guest.json
        fi
        if [ -n "$_husk_icd" ]; then
            [ -z "${VK_ICD_FILENAMES:-}" ] && export VK_ICD_FILENAMES=$_husk_icd
            [ -z "${VK_DRIVER_FILES:-}" ] && export VK_DRIVER_FILES=$_husk_icd
            [ -z "${MESA_LOADER_DRIVER_OVERRIDE:-}" ] && export MESA_LOADER_DRIVER_OVERRIDE=zink
            [ -z "${GALLIUM_DRIVER:-}" ] && export GALLIUM_DRIVER=zink
            # PanVK/zink 当前上限 3.2；真实应用上限仍未验证。
            [ -z "${MESA_GLES_VERSION_OVERRIDE:-}" ] && export MESA_GLES_VERSION_OVERRIDE=3.2
            # GTK4 的 GL 后端在 PanVK/zink 上有三角缺口，只把 GTK 自己钉到 cairo。
            [ -z "${GSK_RENDERER:-}" ] && export GSK_RENDERER=cairo
        else
            _husk_gpu_family=software
        fi
        ;;
    adreno-kgsl)
        for _husk_c in \
            /usr/share/vulkan/icd.d/freedreno_icd.aarch64.json \
            /usr/share/vulkan/icd.d/freedreno_icd.json \
            /usr/share/vulkan/icd.d/*freedreno*.json \
            /usr/share/vulkan/icd.d/*turnip*.json
        do
            if [ -r "$_husk_c" ]; then _husk_icd=$_husk_c; break; fi
        done
        if [ -n "$_husk_icd" ]; then
            [ -z "${VK_ICD_FILENAMES:-}" ] && export VK_ICD_FILENAMES=$_husk_icd
            [ -z "${VK_DRIVER_FILES:-}" ] && export VK_DRIVER_FILES=$_husk_icd
            [ -z "${TURNIP_KMD:-}" ] && export TURNIP_KMD=kgsl
            [ -z "${FD_FORCE_KGSL:-}" ] && export FD_FORCE_KGSL=1
            [ -z "${MESA_VK_WSI_PRESENT_MODE:-}" ] && export MESA_VK_WSI_PRESENT_MODE=mailbox
            [ -z "${vblank_mode:-}" ] && export vblank_mode=0
            _husk_native_gl=0
            for _husk_d in /usr/lib/*/dri/kgsl_dri.so /usr/lib/*/dri/freedreno_dri.so; do
                if [ -r "$_husk_d" ]; then _husk_native_gl=1; break; fi
            done
            if [ "$_husk_native_gl" = 1 ]; then
                [ -z "${MESA_LOADER_DRIVER_OVERRIDE:-}" ] && export MESA_LOADER_DRIVER_OVERRIDE=kgsl
                [ -z "${GALLIUM_DRIVER:-}" ] && export GALLIUM_DRIVER=freedreno
            else
                # 只有 Turnip ICD、没有原生 Gallium 时才走 zink；这是硬件路径，不是 llvmpipe。
                [ -z "${MESA_LOADER_DRIVER_OVERRIDE:-}" ] && export MESA_LOADER_DRIVER_OVERRIDE=zink
                [ -z "${GALLIUM_DRIVER:-}" ] && export GALLIUM_DRIVER=zink
            fi
            unset _husk_native_gl _husk_d
        else
            _husk_gpu_family=software
        fi
        ;;
    drm-node)
        # 真 DRM 节点由发行版 Mesa 自己枚举，不在这里硬塞 ICD。
        ;;
    *)
        _husk_gpu_family=software
        ;;
esac

if [ "$_husk_gpu_family" = software ]; then
    _husk_gpu_soft
fi

unset _husk_c _husk_gpu_family _husk_icd
unset -f _husk_gpu_off _husk_gpu_soft 2>/dev/null || true
