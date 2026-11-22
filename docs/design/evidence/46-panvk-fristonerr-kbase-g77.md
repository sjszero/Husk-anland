# evidence 46 — FristOneRR PanVK 在 guest 内枚举 Mali-G77，并提交 compute job

日期：2026-11-23 · 设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191
执行域：`com.linuxhub.husk` 的 `untrusted_app`，guest rootfs：`files/rootfs-full`

## 结论边界

- **FACT**：guest 进程直接打开 `/dev/mali0`，FristOneRR PanVK 成功枚举真实
  `Mali-G77 MC9`（`0x90800011`）。
- **FACT**：纯 Rust `vulkan-probe` 已创建 instance、logical device 和 queue，分配并绑定
  一个 4 KiB host-visible storage buffer，提交一个 `vkCmdDispatch` compute job，等待 fence 后
  读回 GPU 写入的 `0x4855534b`。
- **FACT**：初次设备打开时 `KBASE_IOCTL_MEM_EXEC_INIT` 返回 `EPERM`；按 `jm-v9` 先例把
  `MEM_EXEC_INIT` 移到 `MEM_JIT_INIT` 之前并把 exec VA 设为 4G 后，警告消失，compute job 成功。
- **UNKNOWN**：还没有 graphics pipeline、swapchain 或 `eglinfo`/`glxinfo` renderer 结果；
  因此不能写成 GPU 图形渲染完成，也不能替代第 2 条完整验收。

## 1. 采用的源码与为什么换掉 funnymdzz

主动构建源：

```text
repo:   FristOneRR-Admin/FristOneRR-Panvk-Source
commit: 0c9dce35094dbe89c412b79bf57c6067c7b97e59
Mesa:   26.3.0-devel
```

理由（FACT）：

- `src/panfrost/model/pan_model.c:88` 有 `VALHALL_MODEL(PAN_PROD_ID(9, 0, 0), 0, "G77", ...)`；
  本机 `GPU_ID=0x90800011` 正好对应 `PAN_PROD_ID(9,0,0)`。
- kbase 路径有 CPU-backed sync；源码注释明确说明 kbase fd 上不可用 DRM syncobj。
- 源码包含 JM 提交路径和 `PANVK_ATOM_STRIDE` 探测。

`funnymdzz/mesa` tag `kbase-v0.1.2`（commit
`4e5323ef35dc580a6140f8d433a6d13ae3724c07`）保留为失败对照：它在
`vkCreateDevice` 处报 `VK_ERROR_OUT_OF_HOST_MEMORY`，根因是 kbase fd 上
`drmSyncobjCreate()` 失败，不是设备真的 OOM。

## 2. glibc 交叉构建

FristOneRR 的发布包是 bionic 构建，`DT_NEEDED` 含 Android 专属库，不能直接塞进 glibc guest。
本轮从源码在 `husk-panvk-builder` 容器内重编：

- 构建容器：Ubuntu 24.04 aarch64 cross toolchain
- host tools：`mesa_clc`、`vtn_bindgen2`、`panfrost_compile`
- 首次链接失败（FACT）：Meson 把宿主 x86_64 的
  `/opt/spirv-tools/lib/libSPIRV-Tools*.a` 混入 AArch64 链接，导致架构不匹配。
- 修复：`meson configure /build/frist-target3 -Dspirv-tools=disabled`，重新链接成功。
- 另需移除源码无条件注入的 Android `-llog`。可复现补丁：
  [patches/fristonerr-glibc-llog.patch](../patches/fristonerr-glibc-llog.patch)。

初版产物（未提交二进制，只记录指纹）：

```text
path:   /build/frist-target3/src/panfrost/vulkan/libvulkan_panfrost.so
size:   15,979,120 bytes
sha256: fbfa3b43e69d3fd7f4088fe6b9e94a621e19ca3f95502265281f92ba908b6147
machine: AArch64
type:   DYN shared object
```

产物 `DT_NEEDED`：

```text
libdrm.so.2
libz.so.1
libstdc++.so.6
libm.so.6
libc.so.6
ld-linux-aarch64.so.1
```

没有 bionic 依赖。`strings` 能看到 `Found kbase device`、`FristOneRR 1.2.0 (Mesa 26.3.0-devel)`、
`Mali-G77`、`PANVK_ATOM_STRIDE` 和 `kbase_kmod.c`。

### 2.1 `EXEC_VA` 顺序修复

首轮 probe 的日志出现：

```text
MESA: warning: kbase: KBASE_IOCTL_MEM_EXEC_INIT failed: Operation not permitted
```

源码顺序是 `MEM_JIT_INIT` 先于 `MEM_EXEC_INIT`；旧 JM kbase 在 JIT 的 `CUSTOM_VA` 建立后
会用 `EPERM` 拒绝 EXEC_VA 初始化。按前人 `jm-v9/004-init-exec-va-before-jit.patch` 的顺序修复：

- `MEM_EXEC_INIT` 先执行；`va_pages = 0x100000`（4G）；
- `EPERM/EINVAL` 只作为 debug 日志，GPU_EX BO 仍可走 CUSTOM_VA 回退；
- 然后再执行 `MEM_JIT_INIT`。

可复现补丁：[patches/fristonerr-exec-va-order.patch](../patches/fristonerr-exec-va-order.patch)。

修复后产物：

```text
path:   /build/frist-target3/src/panfrost/vulkan/libvulkan_panfrost.so
size:   16,044,856 bytes
sha256: ead96201dc84b653f9b643db00e2af239b763c3ecf61f5cf2caa6f44c96de5ec
```

修复后 probe 日志中不再出现 `MEM_EXEC_INIT` warning。

部署到设备私有目录：

```text
files/rootfs-full/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so
files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
```

设备侧 sha256 与宿主产物一致。ICD JSON 的 `library_path` 使用 Android 绝对路径：

```json
{
  "ICD": {
    "api_version": "1.4.352",
    "library_arch": "64",
    "library_path": "/data/data/com.linuxhub.husk/files/rootfs-full/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so"
  },
  "file_format_version": "1.0.1"
}
```

## 3. 纯 Rust `vulkan-probe`

源码：`runtime/crates/vulkan-probe/src/main.rs`。它 `dlopen("libvulkan.so.1")`，动态解析
Vulkan 入口，不链接 Vulkan loader/ICD；流程是：

```text
instance -> physical device -> queue family -> logical device -> queue
-> storage buffer -> allocate/bind/map -> SPIR-V compute pipeline -> descriptor set
-> command buffer -> vkQueueSubmit -> fence -> map/readback -> teardown
```

本阶段**没有** graphics draw、swapchain 或窗口；compute job 只证明 shader 管线和 queue submit
已经到达硬件并完成，不能替代 renderer 验收。

设备运行命令：

```sh
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "cd files && \
  export VK_ICD_FILENAMES=/data/data/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json && \
  export VK_DRIVER_FILES=\$VK_ICD_FILENAMES && \
  export PANVK_DEBUG=startup && \
  export PANVK_KBASE_DVFS=none && \
  timeout 30 sh gexec2.sh rootfs-full usr/bin/vulkan-probe-v3"'
```

实际输出（FACT）：

```text
MESA: info: panvk_debug: startup|noafbc
MESA: info: Created an instance
MESA: info: Found kbase device '/dev/mali0'.
vk|loader|PASS|libvulkan.so.1
vk|symbols|PASS|core entrypoints resolved
vk|instance|PASS|VkResult=0
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk (Mali-G77 MC9) FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk: gpu_id=0x90800011 variant=0x0 texture_features0=0xf7fffffe afbc=1 afrc=0
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|queue_family|PASS|index=0 flags=0x7 queues=1
vk|device|PASS|queue=0x55905633f0 VkResult=0
vk|memory_types|PASS|count=3
vk|buffer|PASS|created/bound/mapped size=4096 type=2 readback=0x4855534b
vk|compute|PASS|shader words=98
vk|compute|PASS|pipeline
vk|compute|PASS|descriptor
vk|compute|PASS|commands
vk|compute|PASS|queue_submit+fence
vk|compute|PASS|readback=0x4855534b
vk|teardown|PASS|device/instance/loader released
```

第一次运行时 compute readback 是 `0x4855634b`；原因是手写的 SPIR-V 常量十进制值写错，
不是 GPU 结果错误。把它修正为 `0x4855534b` 后重复运行两次，均得到期望值。

`vulkaninfo --summary` 也在同一 ICD 下 exit 0，并报出：

```text
GPU0:
    apiVersion       = 1.3.354
    driverVersion    = 26.2.99
    vendorID         = 0x13b5
    deviceID         = 0x90800011
    deviceType       = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
    deviceName       = Mali-G77 MC9
    driverID         = DRIVER_ID_MESA_PANVK
    driverName       = panvk
    driverInfo       = FristOneRR 1.2.0 (Mesa 26.3.0-devel)
```

## 4. 已知边界与下一步

### `KBASE_IOCTL_MEM_EXEC_INIT` 历史告警

- **FACT**：未改顺序前该 ioctl 返回 `EPERM`，日志说 “executable BO allocation will not work”。
- **FACT**：改成 EXEC_VA 先于 JIT 后，本机 compute probe 已成功提交并由 GPU 写回 storage buffer；
  因此该告警不是“GPU 不可用”的充分证据。
- **UNKNOWN**：graphics pipeline、tiler/IDVS、swapchain 和 WSI 尚未验证；这些仍需单独上机取证。

### atom stride

先前命令设置过 `PANVK_ATOM_STRIDE=72`，但 FristOneRR 当前代码只接受 `48/56/64`，
实际日志归一到 `64`。真正提交 job 前需要根据内核接受值和源码探测逻辑确认 stride，不要继续传 72。

### 下一步顺序

1. 继续验证 atom stride / JM 提交路径，记录 compute job 的实际 submit 计数和完成事件。
2. 做最小 graphics draw（离屏三角形或 `vkcube`），验证 tiler/IDVS 与像素回读。
3. 接上 surface / Zink，最终通过 `eglinfo`/`glxinfo` renderer、硬件 vs llvmpipe
   和长稳测试，才算 GPU 完成。
4. 显示零拷贝、视频硬解、音频 AAudio 按 `docs/GOALS.md` 继续，不能拿软件渲染或软解冒充验收。
