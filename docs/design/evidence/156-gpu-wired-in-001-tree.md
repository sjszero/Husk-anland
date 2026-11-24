# evidence 156 — GPU 在 0.01 的树里接通了：PanVK 计算 + Zink 的 GL 走 GPU

设备：RMX3357 / MT6939→MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64。
包 `com.linuxhub.myth.husk`，rootfs `files/linux/distributions/ubuntu-24.04/rootfs`。
时间：2026-11-24 17:59–18:01（CST）。会话由 `husk-session.sh` 起，域 `untrusted_app`，无 root。

## 做了什么

0.01 的树里原本只有发行版自带的 Mesa ICD。发行版的 panvk 走 DRM（`/dev/dri/renderD*`），
本机只有 kbase（`/dev/mali0`），所以 GL 只能落到 `llvmpipe`。这一步把 kbase 后端的
PanVK（FristOneRR，Mesa 26.3.0-devel，`libvulkan_panfrost_frist370_execva.so`）与它的
ICD json 放进树里，再让 Zink 通过它跑 GL。

```sh
# 树里新增
usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so
usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json     # library_path 用 guest 内的 /usr/... 路径
```

## 结果（FACT）

**1. 设备能在 guest 里打开。** `exec 3<>/dev/mali0` → `open ok`。

**2. PanVK 在 0.01 的树里枚举到真 GPU，并跑完 compute。**（`usr/bin/vulkan-probe`）

```
vk|loader|PASS|libvulkan.so.1
vk|instance|PASS|VkResult=0
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|queue_family|PASS|index=0 flags=0x7 queues=1
vk|device|PASS|queue=... VkResult=0
vk|buffer|PASS|created/bound/mapped size=4096 type=2 readback=0x4855534b
vk|teardown|PASS|device/instance/loader released
```

`0x4855534b` 是 GPU 写回的值（"HUSK"），不是 CPU 预先写的。

**3. GL 走 GPU。** `MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink`，surfaceless：

```
gl|get_platform_display|PASS|surfaceless
gl|query_devices|PASS|n=1
gl|device_display|PASS|
gl|initialize|PASS|EGL 1.5
gl|create_context|PASS|
gl|GL_RENDERER|INFO|zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
gl|GL_VERSION|INFO|OpenGL ES 2.0 Mesa 27.0.0-devel (git-b84f02a451)
gl|verdict|PASS|renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

同一个 EGL 在**不设** zink 时是 `llvmpipe (LLVM 20.1.2, 128 bits)`；发行版自带
`panfrost_icd.json` 走 zink 时是 `vkCreateInstance failed (VK_ERROR_INCOMPATIBLE_DRIVER)`，
因为它要的是 DRM render node。

## 还没做到的（UNKNOWN / 未验证）

- **API 上限是 GLES 2.0 / GL 2.1。** 这个配置下 Zink 拿不到更高版本，GNOME 46 的
  Clutter/Cogl 要 GLES 3.0+ 或 GL 3.3+，所以「桌面整条走 GPU」还差这一级。原因待查
  （Zink 对 PanVK feature 的判定 vs EGL 平台），**不是**设备不支持。
- **WSI / swapchain 仍未接。** 窗口表面的呈现仍走 `AHardwareBuffer` → dma-buf →
  `SurfaceControl`（evidence 59），GPU 直接上交窗口表面的那条链没做。
- 没测帧率、没测功耗；这里只证明「GL 调用真的落在 Mali-G77 上」。
