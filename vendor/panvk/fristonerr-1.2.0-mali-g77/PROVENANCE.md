# FristOneRR PanVK 1.2.0（Mali kbase / glibc guest）

这是 Husk 发行包随附的 Mali kbase PanVK 驱动快照。它不是发行版 Mesa 自带的
DRM/Panthor ICD；它让 guest 进程直接打开 `/dev/mali0`，再经 kbase 执行 Vulkan，
GL 由 Zink 落到这颗 PanVK 上。

## 固定标识

| 项 | 值 |
|---|---|
| 驱动报告版本 | FristOneRR 1.2.0 (Mesa 26.3.0-devel) |
| 上游源码 | `FristOneRR-Admin/FristOneRR-Panvk-Source` |
| 源码 commit | `0c9dce35094dbe89c412b79bf57c6067c7b97e59` |
| 直接基线 | `funnymdzz/mesa` @ `6598829019c0746aa8e473b4ae1c980cbfa6ea4b` |
| ELF Build ID | `56271afde866d837c77baaa1f7ca2b56358c90ed` |
| 解压后 SHA-256 | `599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c` |
| 压缩包 SHA-256 | `587d77b5637a1603f84acda1685d4896959459086a648f36e68eb36adc3fc4bd` |
| 目标 ABI | Linux aarch64 / glibc，不是 Android bionic |
| 已实测 GPU | Mali-G77 MC9（RMX3357，Android 13，kernel 4.19.191） |

## 本仓库施加的源码补丁

构建快照包含本仓库保留的三份可复现补丁：

- `docs/design/patches/fristonerr-exec-va-order.patch`
- `docs/design/patches/fristonerr-glibc-llog.patch`
- `docs/design/patches/fristonerr-kbase-syncfd-export.patch`

补丁说明和实测证据见 `docs/design/evidence/46-panvk-fristonerr-kbase-g77.md`、
`docs/design/evidence/58-panvk-syncfd-ring.md` 与
`docs/design/evidence/241-panvk-icd-path-fix-20261126.md`。

## 为什么固定 1.2.0

2026-11-27 在同一台 Mali-G77 真机上，用 FristOneRR v3.7.0 的
`libvulkan_panfrost.so` 替换本快照后，Zink 建实例失败：

```text
FATAL eglInitialize failed 0x3001
MESA: error: ZINK: vkCreateInstance failed (VK_ERROR_INCOMPATIBLE_DRIVER)
```

恢复 1.2.0 快照后，同一命令、同一树、同一 loader 立即恢复为
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。因此发行包不能因为版本号更大
就替换这颗已在 G77 上验证过的驱动。

## 分发边界

- 该快照只覆盖 Mali kbase 家族；Adreno/KGSL 不使用它。
- 发行版根文件系统仍可自带其他 Mesa ICD；安装器只在目标树没有 PanVK 驱动且
  `/dev/mali0` 存在时解压本快照，不覆盖已有驱动。
- Mesa/FristOneRR 代码按 MIT 分发，许可证原文见 `LICENSE-Mesa.txt`。
- 这里不声称它是上游 Mesa 官方构建，也不把它描述成通用的 Android GPU 驱动。
