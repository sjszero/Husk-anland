# Evidence 48 — PanVK 表面与 dma-buf 扩展能力扫描

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通应用域，UID `u0_a302`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`  
探针二进制：`vulkan-probe-v7`  
sha256：`e7b9b6936cef9cf354e81200450b82ea6a5b888930f74814e06ad761f92b2175`

## 结论

- **FACT**：PanVK 实例扩展 18 项、设备扩展 171 项。
- **FACT**：支持 `VK_KHR_surface`、`VK_KHR_swapchain`、`VK_KHR_external_memory`、
  `VK_KHR_external_memory_fd`、`VK_EXT_external_memory_dma_buf`、
  `VK_EXT_external_memory_host`、`VK_EXT_image_drm_format_modifier`、
  `VK_KHR_external_fence(_fd)`、`VK_KHR_external_semaphore(_fd)`、
  `VK_EXT_queue_family_foreign`、`VK_KHR_timeline_semaphore`、
  `VK_KHR_synchronization2`、`VK_EXT_swapchain_maintenance1`、
  `VK_KHR_present_id`、`VK_KHR_present_wait`。
- **FACT**：缺失 `VK_KHR_android_surface` 和
  `VK_ANDROID_external_memory_android_hardware_buffer`。
- **INFERENCE**：guest 内不能直接创建 Android Vulkan Surface，也不能直接导入
  `AHardwareBuffer`；但可以通过 raw dma-buf fd 导入外部内存。
- **UNKNOWN / 未验证**：`AHardwareBuffer_getNativeHandle()` 拿到的 handle 在所有目标 ROM 上
  是否都含可直接导入的 dma-buf fd；本文件的扫描只证明 Vulkan 扩展面，不证明 AHB 导入已经成功。
- **UNKNOWN / 未验证**：真实 Android Surface 的 swapchain、queueBuffer、present fence 和窗口显示尚未做。

## 路线含义

因为 `VK_KHR_android_surface` 缺失，窗口路线不能写成“guest 直接创建 Android Vulkan surface”。
当前设计是：

1. app/bionic 侧分配 `AHardwareBuffer`；
2. 从 native handle 取 dma-buf fd；
3. 通过 `SCM_RIGHTS` Unix socket 把 fd 交给 guest；
4. guest 用 `VK_EXT_external_memory_dma_buf` +
   `VK_KHR_external_memory_fd` 导入 dma-buf 并创建 image；
5. GPU 直接渲染；
6. app 用同一块 `AHardwareBuffer` 交给 `ANativeWindow_queueBuffer`。

如果第 2 或第 4 步失败，必须记录原始错误，转 CPU 回读/上传降级并实测开销；
不能把“支持 dma-buf 扩展”冒充“零拷贝已经完成”。

## 复现命令

```sh
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-v7

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cat /data/local/tmp/vulkan-probe-v7 > files/vulkan-probe-v7
  chmod 755 files/vulkan-probe-v7
'"

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/user/0/com.linuxhub.husk/files
  R=\$F/rootfs-full
  ICD=\$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
  export VK_ICD_FILENAMES=\$ICD VK_DRIVER_FILES=\$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink
  cd \$F
  timeout 60 sh gexec2.sh rootfs-full \$F/vulkan-probe-v7 2>&1
'"
```

注意：`gexec2.sh` 会 `cd` 到 rootfs；探针必须用 files 目录下的绝对路径，否则 loader 会把它
当成 rootfs 内不存在的共享库路径，报 `cannot open shared object file`。

## 原始输出

```text
vk|loader|PASS|libvulkan.so.1
vk|symbols|PASS|core entrypoints resolved
MESA: info: panvk_debug: startup|noafbc
MESA: info: Created an instance
vk|instance|PASS|VkResult=0
MESA: info: Found kbase device '/dev/mali0'.
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk (Mali-G77 MC9) FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk: gpu_id=0x90800011 variant=0x0 texture_features0=0xf7fffffe afbc=1 afrc=0
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|ext|INFO|instance=18 device=171
vk|ext|support|VK_KHR_surface
vk|ext|missing|VK_KHR_android_surface
vk|ext|support|VK_KHR_get_physical_device_properties2
vk|ext|support|VK_KHR_external_memory_capabilities
vk|ext|support|VK_KHR_external_fence_capabilities
vk|ext|support|VK_KHR_external_semaphore_capabilities
vk|ext|support|VK_KHR_get_surface_capabilities2
vk|ext|support|VK_EXT_surface_maintenance1
vk|ext|support|VK_KHR_swapchain
vk|ext|missing|VK_ANDROID_external_memory_android_hardware_buffer
vk|ext|support|VK_KHR_external_memory
vk|ext|support|VK_KHR_external_memory_fd
vk|ext|support|VK_EXT_external_memory_dma_buf
vk|ext|support|VK_EXT_external_memory_host
vk|ext|support|VK_EXT_image_drm_format_modifier
vk|ext|support|VK_KHR_external_fence
vk|ext|support|VK_KHR_external_fence_fd
vk|ext|support|VK_KHR_external_semaphore
vk|ext|support|VK_KHR_external_semaphore_fd
vk|ext|support|VK_EXT_queue_family_foreign
vk|ext|support|VK_KHR_timeline_semaphore
vk|ext|support|VK_KHR_synchronization2
vk|ext|support|VK_EXT_swapchain_maintenance1
vk|ext|support|VK_KHR_present_id
vk|ext|support|VK_KHR_present_wait
vk|queue_family|PASS|index=0 flags=0x7 queues=1
vk|device|PASS|queue=0x55900ee4b0 VkResult=0
vk|memory_types|PASS|count=3
vk|buffer|PASS|created/bound/mapped size=4096 type=2 readback=0x4855534b
vk|compute|PASS|shader words=98
vk|compute|PASS|pipeline
vk|compute|PASS|descriptor
vk|compute|PASS|commands
vk|compute|PASS|queue_submit+fence
vk|compute|PASS|readback=0x4855534b
vk|graphics|PASS|pipeline
vk|graphics|PASS|commands
vk|graphics|PASS|queue_submit+fence
vk|graphics|PASS|pixel=0xff0000ff expected=0xff0000ff
vk|graphics|PASS|teardown
vk|teardown|PASS|device/instance/loader released
```
