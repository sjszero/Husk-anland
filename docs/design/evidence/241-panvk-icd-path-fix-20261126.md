# 241 · PanVK ICD 宿主路径导致 SIGSEGV（guest 相对路径修复）

日期：2026-11-26 02:43–02:45（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 /
4 KiB 页 / Mali-G77 MC9。测试包 `com.linuxhub.husk`，rootfs `files/rootfs-full`。
`com.linuxhub.myth.husk` 未启动、未停止、未替换资产。

## 1 · 结论

- **FACT**：0.0.15 发布说明里“独立测试树的 PanVK 旧、`gl-probe` / `vulkan-probe` SIGSEGV”
  不是驱动版本本身造成的。直接原因是 PanVK ICD JSON 的 `library_path` 写成了宿主绝对路径
  `/data/data/com.linuxhub.husk/files/rootfs-full/usr/lib/...`。guest 里的路径 shim 会把
  `/data/...` 当成 guest 路径再次翻译，Vulkan loader 最终拿不到真实驱动；故障地址为 `0`。
- **FACT**：同一份 `libvulkan_panfrost_frist370_execva.so`，只把 ICD 的 `library_path`
  改成 `/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so`，Vulkan 和
  Zink/ES3 都通过。
- **FACT**：修复后的 App 初始终端不需要手工 export。`/proc/10732/environ` 直接给出
  `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json`；该 manifest 是 App
  自动生成的 guest 相对路径版本。
- **UNKNOWN**：X11 窗口内容进入 X server 仍然不是零拷贝；本次只修 GPU 驱动选择，不改
  DRI3 / dma-buf 呈现路径。`glxgears` 在集成树上的 PanVK/Bifrost 编译器崩溃也没有被本次
  探针重测，不能据此写成已修。

## 2 · 根因 A/B（FACT，真机）

测试树里原来的 manifest：

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

使用原 manifest 跑 `vulkan-probe` 和 `gl-probe es3`：

```text
VULKAN
vk|loader|PASS|libvulkan.so.1
vk|symbols|PASS|core entrypoints resolved
segv|sig=11|fa=0|pc=536692387652|lr=367498347248|x0=0|x1=0|x2=0|x8=117|ld=536694419456|lc=0
rc=139
GL
segv|sig=11|fa=0|pc=536085655364|lr=367472597504|x0=0|x1=0|x2=0|x8=117|ld=536087531520|lc=0
rc=139
```

改成 guest 相对路径的 manifest 后：

```json
{
  "ICD": {
    "api_version": "1.4.352",
    "library_arch": "64",
    "library_path": "/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so"
  },
  "file_format_version": "1.0.1"
}
```

同一个驱动、同一个 loader、同一个 guest rootfs，结果变成：

```text
vulkan-probe:
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|compute|PASS|readback=0x4855534b
vk|graphics|PASS|pixel=0xff0000ff expected=0xff0000ff
VULKAN_RC=0

gl-probe es3:
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
GLSL=OpenGL ES GLSL ES 3.20
MRT(2 附件) FBO 完整 OK
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
GL_RC=0
```

由此可以排除“独立树驱动必然崩溃”的判断。它是 ICD 内容错误，不是 GPU 设备初始化失败。

## 3 · 代码修复

- App `gpu_env()` 不再挑一个已有 `husk_*.json` 后直接把宿主路径交给 loader。它按固定优先级
  检查真实 PanVK 驱动，只在驱动存在时生成
  `usr/share/vulkan/icd.d/husk_panvk_guest.json`；manifest 的 `library_path` 固定使用 guest
  路径。生成文件用同目录临时文件 + rename，避免启动中的半写文件。
- `install-husk.sh` 改用同样的 canonical manifest，不再按 glob 第一个文件盲选。
- `/etc/profile.d/husk-gpu.sh`、`husk-session.sh` 和设备测试 GNOME 启动脚本都改指 canonical
  manifest。
- 没有驱动时拒绝选择 manifest，不再出现“manifest 存在但驱动不存在”的假硬件路径。

## 4 · 无手工变量的真机验证（FACT）

冷启动新 APK 后，初始 guest shell 是独立的 `libld.so` 进程。没有在测试命令里手工 export
Vulkan/GL 变量；`husk-run-guest.sh` 只读取该进程的 `/proc/<pid>/environ`。观察到：

```text
NR_ROOTFS=/data/user/0/com.linuxhub.husk/files/rootfs-full
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

App 自动生成的文件：

```text
/data/data/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_guest.json
```

内容已在本文件 §2 给出，其中没有 `/data/`，也没有当前安装目录。

## 5 · 边界

- **FACT**：这次修的是 Vulkan ICD 的驱动定位；GPU 渲染仍然经过 Zink → PanVK。
- **FACT**：X11 窗口内容的 SHM 回读和拷贝仍存在（evidence 174/228），本次没有把呈现改成
  DRI3 / dma-buf 零拷贝。
- **FACT**：`MESA_GLES_VERSION_OVERRIDE=3.2` 仍是兼容性覆盖参数，不是“设备原生默认报 3.2”。
- **UNKNOWN**：Adreno/KGSL 和无 PanVK 的其他 GPU 家族没有真机验证。
- **UNKNOWN**：本修复对 `glxgears` 在 Bifrost 编译器上的稳定崩溃没有帮助；那是另一条路径，
  本次没有用一次探针通过就宣布长跑通过。

## 6 · 最终 0.0.16 重跑（FACT，真机）

发布 APK：

```text
versionName=0.0.16
versionCode=16
sha256=cf789180c62562d5fdea3518f11c31a053dcbb6b120348e753ae678d61bb571a
```

安装后先删除旧 canonical manifest，再 force-stop、冷启动。App 重新生成 manifest，初始 guest
shell（新 PID 11614）的环境为：

```text
NR_ROOTFS=/data/user/0/com.linuxhub.husk/files/rootfs-full
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

最终构建的 `vulkan-probe`：

```text
vk|instance|PASS|VkResult=0
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|compute|PASS|readback=0x4855534b
vk|graphics|PASS|pixel=0xff0000ff expected=0xff0000ff
vk|teardown|PASS|device/instance/loader released
VULKAN_RC=0
```

最终构建的 `gl-probe es3`：

```text
EGL 1.5
ES3 位 config: 有
ES3 上下文建立 OK
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
GLSL=OpenGL ES GLSL ES 3.20
MRT(2 附件) FBO 完整 OK
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
GL_RC=0
```

因此 0.0.16 的验证不是手工拼环境，也不是只验证变量存在：初始会话由 App 默认注入
canonical ICD，随后 Vulkan compute/graphics 与 Zink/ES3 都实际执行到回读并通过。
