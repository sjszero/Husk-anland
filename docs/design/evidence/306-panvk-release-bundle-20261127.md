> **更正（evidence 314）**：文中“整块 glReadPixels 仍坏”的旧结论来自探针 PixelStore 常量错配；修正后整块读在真机通过（evidence 314）。

# 306 · Mali kbase PanVK 进入发行包，并在干净安装树复验

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`，`untrusted_app`
目标：把上一轮手工补进测试树的 PanVK 驱动变成发行包的一部分，并从 rootfs tar 干净安装后复验。

## 0 · 结论

- **FACT**：发行包现在携带固定版本 FristOneRR PanVK 1.2.0（Mesa 26.3.0-devel）的
  `libvulkan_panfrost_frist370_execva.so.gz`、ICD、MIT 许可证、来源和 SHA-256。
- **FACT**：安装器只在宿主 `/dev/mali0` 可打开且目标树没有 PanVK 驱动时解压快照；不会
  覆盖 rootfs 已有的其他 GPU 驱动。
- **FACT**：从 672 MiB rootfs tar 重新安装到空目录，`INSTALL_RC=0`，树体检
  `PASS=14 FAIL=0`，GPU loader 默认值 `PASS=9 FAIL=0`，门禁 A
  `PASS=34 FAIL=0 SKIP=1`，读回套 `PASS=17 FAIL=0`。
- **FACT**：不带任何 GPU 环境变量的 guest 进程报告
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；`NR_GPU_DEFAULT_OFF=1` 与
  `HUSK_GPU=llvmpipe` 两个反向控制都落到 `llvmpipe`。
- **UNKNOWN**：Adreno/KGSL 仍无真机；本快照只针对 Mali kbase，不能外推。
- **UNVERIFIED**：发行树上的完整 GNOME 桌面长稳和多应用并发本轮没有重新跑；本轮只验证
  干净安装、GPU 默认值、门禁 A 和 GPU 读回套。

## 1 · 为什么不能继续手工补驱动

前一轮 `release-tree-v2` 里的 PanVK 是手工从旧 2.0 树复制进去的。它能通过测试，但发行包
本身没有这颗驱动，换一台机器或重建 rootfs 后就会静默回到 llvmpipe。仅修改 README 或让
测试树继续有效，都不算发行支持；必须把驱动、ICD、许可证和安装路径一起固定下来。

## 2 · 版本选择（FACT）

先用 FristOneRR 当前 release v3.7.0 的 `libvulkan_panfrost.so` 替换同一棵树里的 1.2.0，
其余条件不变。Zink 建实例失败：

```text
FATAL eglInitialize failed 0x3001
MESA: error: ZINK: vkCreateInstance failed (VK_ERROR_INCOMPATIBLE_DRIVER)
```

恢复 1.2.0 后，同一探针立即恢复：

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

因此不能按“版本号更大”更新；固定的是已在 G77 上实际跑通的 1.2.0 快照。

## 3 · 快照内容

目录：`vendor/panvk/fristonerr-1.2.0-mali-g77/`

```text
libvulkan_panfrost_frist370_execva.so.gz   3237797 bytes
  compressed sha256  587d77b5637a1603f84acda1685d4896959459086a648f36e68eb36adc3fc4bd
  decompressed sha256 599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c
  ELF Build ID       56271afde866d837c77baaa1f7ca2b56358c90ed
husk_panvk_guest.json
LICENSE-Mesa.txt
PROVENANCE.md
SHA256SUMS
```

源码固定为 `FristOneRR-Admin/FristOneRR-Panvk-Source` commit
`0c9dce35094dbe89c412b79bf57c6067c7b97e59`，并附本仓库保留的三份补丁。
许可证是 Mesa/FristOneRR 的 MIT 文本。

## 4 · 安装器行为

`tools/guest/install-husk.sh` 在 GPU 默认值生成之前执行：

1. `/dev/mali0` 不可打开：不动树，走软件或其他 GPU 家族路径。
2. 树里已有 PanVK 驱动：不动树，保留已有驱动。
3. Mali kbase 且树里没有 PanVK：解压固定快照，验证解压后 SHA-256，再安装驱动、ICD、许可证和
   `PROVENANCE.md`。

安装器随后解析 ICD 的 `api_version`。第一次重装发现生成器只扫描旧文件名，把快照的
`1.3.0` 降成了 `1.0.0`；把 canonical `husk_panvk_guest.json` 放进扫描列表后，最终树保留：

```json
{
  "ICD": {
    "api_version": "1.3.0",
    "library_arch": "64",
    "library_path": "/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so"
  },
  "file_format_version": "1.0.1"
}
```

## 5 · 干净安装与真机复验（FACT）

发行 bundle：

```text
husk-arm64.tar.gz
size   10293845 bytes
sha256 c822c234984036c13d5af516ae5a8707e600c72e3fe9fe1dbf85ee76c504f7ce
```

从 `release-tree-v2.tar.gz` 重新安装到空目录：

```text
installed bundled PanVK 1.2.0
wrote usr/share/vulkan/icd.d/husk_panvk_guest.json
wrote etc/husk/gpu-env (mali-kbase, ICD: /usr/share/vulkan/icd.d/husk_panvk_guest.json)
INSTALL_RC=0
```

树内驱动 hash：

```text
599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c
```

门禁：

```text
TREE_INTEGRITY PASS=14 FAIL=0
GPU_LOADER_DEFAULTS PASS=9 FAIL=0
Gate A PASS=34 FAIL=0 SKIP=1
readback-suite PASS=17 FAIL=0
```

读回套的 renderer：

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
```

唯一的门禁 A SKIP 仍是离线树没有 apt package index；不是运行时失败。

## 6 · 仍然存在的边界

- 驱动会打印 `KBASE_IOCTL_MEM_EXEC_INIT failed: Operation not permitted`，但本轮 Vulkan/Zink
  建实例、渲染和读回全部通过；这是警告，不是“已消除”的错误。
- **历史记录（已被 evidence 314 更正）**：当时把 16 字节行距误判为运行时缺陷；根因是读回
  探针的 PixelStore 常量错配。修正后整块读通过，读回套为 `PASS=19 FAIL=0`。
- 发行包提供的是 Mali kbase 快照，不是通用 GPU 驱动；Adreno/KGSL 仍为 UNKNOWN。
- 本轮没有把“安装后默认有 GPU”扩大成“所有桌面应用、所有 GPU、所有 Android 内核都通过”。
