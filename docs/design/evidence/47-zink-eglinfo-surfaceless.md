# Evidence 47 — Zink/PanVK 在 surfaceless EGL 上报出 Mali-G77

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通应用域，UID `u0_a302`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

- **FACT**：FristOneRR glibc 版 PanVK 在 guest 进程内成功创建 Vulkan instance / physical device，
  识别到真实 `Mali-G77 MC9`（deviceID `0x90800011`），驱动为 `DRIVER_ID_MESA_PANVK`。
- **FACT**：发行版 Mesa 的 Zink 通过该 PanVK device 创建了 surfaceless EGL 1.5 上下文；
  `eglinfo -B` 与纯 Rust `gl_probe` 都报出：
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。
- **FACT**：壳应用启动的 guest shell 实际使用 `rootfs-full`，且子进程环境里已带上
  `__EGL_VENDOR_LIBRARY_FILENAMES`、`VK_ICD_FILENAMES`、`VK_DRIVER_FILES`、
  `MESA_LOADER_DRIVER_OVERRIDE=zink`、`GALLIUM_DRIVER=zink`。
- **UNKNOWN / 未完成**：GBM、Wayland、X11 三个 platform 当前仍 `eglInitialize failed`；
  swapchain、WSI、Android Surface、SurfaceFlinger queueBuffer、实际窗口显示都还没有验收。
  因此本条只证明 **headless/surfaceless 渲染与 renderer 身份**，不能写成“GPU/显示全部完成”。
- **UNKNOWN / 未完成**：硬件 vs llvmpipe 帧率对比尚未做。

## 复现环境

`VK_DRIVER_FILES` 必须设置为完整路径。若它被展开成空字符串，Vulkan loader 会返回
`VK_ERROR_INCOMPATIBLE_DRIVER / Found no drivers!`，即使 `VK_ICD_FILENAMES` 正确。
这是 shell 引号造成的假阴性，不是 PanVK 回归。

```sh
R=/data/user/0/com.linuxhub.husk/files/rootfs-full
ICD=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cd files
  export VK_ICD_FILENAMES=$ICD
  export VK_DRIVER_FILES=$ICD
  export PANVK_DEBUG=startup
  export PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink
  export GALLIUM_DRIVER=zink
  export __EGL_VENDOR_LIBRARY_FILENAMES=$R/usr/share/glvnd/egl_vendor.d/50_mesa.json
  timeout 60 sh gexec2.sh rootfs-full usr/bin/eglinfo -B 2>&1
'"
```

## `eglinfo -B` 原始输出（关键段）

```text
MESA: info: panvk_debug: startup|noafbc
MESA: info: Created an instance
MESA: info: Found kbase device '/dev/mali0'.
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk (Mali-G77 MC9) FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk: gpu_id=0x90800011 variant=0x0 texture_features0=0xf7fffffe afbc=1 afrc=0

GBM platform:
eglinfo: eglInitialize failed

Wayland platform:
eglinfo: eglInitialize failed

X11 platform:
eglinfo: eglInitialize failed

Surfaceless platform:
EGL API version: 1.5
EGL vendor string: Mesa Project
EGL version string: 1.5
EGL client APIs: OpenGL OpenGL_ES
OpenGL compatibility profile vendor: Mesa
OpenGL compatibility profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL compatibility profile version: 2.1 Mesa 25.2.8-0ubuntu0.24.04.5
OpenGL compatibility profile shading language version: 1.20
OpenGL ES profile vendor: Mesa
OpenGL ES profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile version: OpenGL ES 2.0 Mesa 25.2.8-0ubuntu0.24.04.5
OpenGL ES profile shading language version: OpenGL ES GLSL ES 1.0.16
```

`eglinfo` 整体退出码为 3，因为 GBM/Wayland/X11 平台失败；surfaceless 和 Device platform
两段都成功，所以退出码不能当作“完全失败”。

## 纯 Rust `gl_probe` 原始输出

```text
gl|dlopen|PASS|libEGL.so.1
gl|get_platform_display|PASS|surfaceless 0x55589714f0
gl|open_render|FAIL|errno=2
gl|query_devices|PASS|n=1
gl|device_display|PASS|0x5558972400
MESA: info: panvk_debug: startup|noafbc
MESA: info: Created an instance
MESA: info: Found kbase device '/dev/mali0'.
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk (Mali-G77 MC9) FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk: gpu_id=0x90800011 variant=0x0 texture_features0=0xf7fffffe afbc=1 afrc=0
gl|initialize|PASS|EGL 1.5
gl|EGL_VENDOR|INFO|Mesa Project
gl|EGL_VERSION|INFO|1.5
gl|EGL_EXTENSIONS|INFO|33 项
gl|bind_api|PASS|ES
gl|choose_config|PASS|n=1
gl|create_context|PASS|0x55589b04c0
gl|make_current|PASS|surfaceless
gl|dlopen|PASS|libGLESv2.so.2
gl|GL_VENDOR|INFO|Mesa
gl|GL_RENDERER|INFO|zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
gl|GL_VERSION|INFO|OpenGL ES 2.0 Mesa 25.2.8-0ubuntu0.24.04.5
gl|verdict|PASS|renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

`open_render|FAIL|errno=2` 是 `gl_probe` 优先尝试打开不存在的
`/dev/dri/renderD128`；它随后走 `eglQueryDevicesEXT` 拿到 `n=1` 并成功创建
device display。该失败是非致命探测分支，不是最终 renderer 失败。

## `vulkaninfo --summary`（设备身份）

```text
GPU0:
    apiVersion         = 1.3.354
    driverVersion      = 26.2.99
    vendorID           = 0x13b5
    deviceID           = 0x90800011
    deviceType         = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
    deviceName         = Mali-G77 MC9
    driverID           = DRIVER_ID_MESA_PANVK
    driverName         = panvk
    driverInfo         = FristOneRR 1.2.0 (Mesa 26.3.0-devel)
    conformanceVersion = 0.0.0.0
    deviceUUID         = b5130000-1100-8090-0000-000000000000
    driverUUID         = ecd9a801-fa66-41b3-862c-76fe15d78f2a
```

## 壳应用会话与 GPU 环境（FACT）

启动后 logcat：

```text
I Husk: spawned pid=20071 fd=95 tee=96
I Husk: session: running pid=20071 (pgid=sid=pid by setsid) 80x24 fds=98
I Husk: session: resize 80x24 -> 67x58 (TIOCSWINSZ, pid=20071)
```

`ps`：

```text
20071 22273 libld.so libld.so --library-path \
  /data/user/0/com.linuxhub.husk/files/rootfs-full/usr/lib/aarch64-linux-gnu \
  /data/user/0/com.linuxhub.husk/files/rootfs-full/usr/bin/bash -i
```

`/proc/20071/environ` 中的相关项：

```text
NR_ROOTFS=/data/user/0/com.linuxhub.husk/files/rootfs-full
NR_LD=/data/app/~~ZC0td_eLKeO2-YMChNvURg==/com.linuxhub.husk--K6kRu1dbjRtwGWK0e7jgw==/lib/arm64/libld.so
__EGL_VENDOR_LIBRARY_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
VK_DRIVER_FILES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
```

## 壳应用复验（03:00）

重新构建并安装 APK（sha256 `b89eda73cec8e5c5b000dadca90e105faaf85add10c2f9175623f67ca5ece0e9`）后再次冷启动：

```text
I Husk: spawned pid=20799 fd=95 tee=96
I Husk: session: running pid=20799 (pgid=sid=pid by setsid) 80x24 fds=98
I Husk: session: resize 80x24 -> 67x58 (TIOCSWINSZ, pid=20799)
```

`/proc/20799/environ` 同样包含上表的环境变量，`NR_ROOTFS` 仍为 `files/rootfs-full`。
设备上的 Android 目标测试二进制 13/13 通过（包括两个 `gpu_env` 单元测试）。

## 下一步边界

1. 枚举 PanVK / Vulkan external-memory / dma-buf 扩展，确认 Android Surface 零拷贝的可行性。
2. 在 app 侧用纯 Rust `AHardwareBuffer_allocate` + `ANativeWindow_queueBuffer` 建最小窗口；
   guest 侧优先导入同一 dma-buf，不能导入时做 CPU 拷贝降级并记录开销。
3. 在窗口路径跑通前，不把 surfaceless renderer 当作“桌面/浏览器已硬件加速”的证据。
