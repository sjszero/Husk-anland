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
| 解压后 SHA-256 | `5e06324783ec7947d48782441fd1cac23953c90bd0b31663f98f08b22344c1c3` |
| 压缩包 SHA-256 | `47d11cd2fa0cfc36a7d48028c4d81d0a1a920b9b772c5583fc1e8c0c7b84140b` |
| 目标 ABI | Linux aarch64 / glibc，不是 Android bionic |
| 已实测 GPU | Mali-G77 MC9（RMX3357，Android 13，kernel 4.19.191） |

## 本仓库施加的源码补丁

构建快照包含本仓库保留的三份可复现补丁：

- `docs/design/patches/fristonerr-exec-va-order.patch`
- `docs/design/patches/fristonerr-glibc-llog.patch`
- `docs/design/patches/fristonerr-kbase-syncfd-export.patch`
- `docs/design/patches/fristonerr-kbase-warmup-skip.patch`（0.0.37：热身改配置项）
- `docs/design/patches/fristonerr-blit-single-triangle.patch`（0.0.37：vk_meta blit 修复）

补丁说明和实测证据见 `docs/design/evidence/46-panvk-fristonerr-kbase-g77.md`、
`docs/design/evidence/58-panvk-syncfd-ring.md` 与
`docs/design/evidence/241-panvk-icd-path-fix-20261126.md`。

## 0.0.37 起：驱动由源码构建（不再就地改字节）

0.0.36 及以前的发行包是"上游快照 + 安装时按偏移改 16 字节"。0.0.37 起改掉了这个做法，
原因有两条：

1. **blit 修复改不了字节**。`vk_meta` 的缩放 blit 在 Mali v9 (G77) 上会把第二个三角形
   画歪，目标里留下三角形黑洞（H57，详见 evidence 365）。那是控制流/几何改动，只能重编。
2. 一旦重编，**就地改字节的补丁就失去意义**：源码里已经改过了。留着它等于埋一个
   "偏移对不上"的雷。

所以：驱动由源码构建，安装器只做一件事——核对 SHA-256。需要恢复历史行为时用**配置**，
不再改字节。

### 两处源码改动

**a. kbase 2 秒热身 → 配置项**（`src/panfrost/lib/kmod/kbase_kmod.c`）

上游在每次建 kbase context 之后无条件：

```c
/* Empirically, this Mali-G57 kbase JM backend needs ~2s ... */
struct timespec warmup_ts = { .tv_sec = 2, .tv_nsec = 0 };
nanosleep(&warmup_ts, NULL);
```

那是作者在 **Mali-G57** 上绕"第一个 job 被终止"的办法。本机 Mali-G77 MC9
（Android 13，kernel 4.19.191）**不需要**：去掉之后 ES3 离屏读回结果一致、整屏抓帧
逐字节相同（evidence 357）。而代价是每个 GL/Vulkan 进程建两次 context 就白等 4 秒，
桌面冷启动、DING、浏览器全部照付。

改成读环境变量（默认不睡）：

| 值 | 行为 |
|---|---|
| 未设置 / `0` / 空 | 不睡（默认） |
| `PANVK_KBASE_WARMUP_MS=2000` | 上游原行为，Mali-G57 一类 kbase JM 需要时打开 |
| `PANVK_KBASE_WARMUP_MS=500` | 半程热身 |

**b. vk_meta 缩放 blit → 单三角形**（`src/vulkan/runtime/vk_meta_draw_rects.c`）

`vk_meta_draw_volume()` 原来用 `create_vertex_buffer()` 写 6 个顶点（两个三角形）再
`CmdDraw(6)`。在 Mali v9 的 jm 绘制路径上，**第一个三角形画对、第二个画歪**，于是
`vkCmdBlitImage` 的缩放结果里有一块三角形没被着色；又因为这条 render pass 不加载旧内容，
那块就露出 0（黑）。zink 把 `glBlitFramebuffer` 映射到 `vkCmdBlitImage`，所以
**桌面一吃 GPU 加速，应用盘缩略图就出现黑三角**。

改成：一个覆盖整个视口的单三角形 + 精确到目标矩形的 scissor。单三角形在同一条路径上
是精确的，scissor 负责把范围收回到目标矩形。这也是 CSF（`PAN_ARCH >= 10`）路径
`cmd_run_fullscreen()` 原本的形态，属于**驱动通用**修法，不按 SoC 分支。

### 构建配方（可复现）

```sh
# 源码：FristOneRR-Panvk-Source @ 0c9dce35094dbe89c412b79bf57c6067c7b97e59
# 交叉文件：aarch64-linux-gnu（glibc），meson：
#   --buildtype=release --cross-file=/build/aarch64-cross.ini \
#   -Dplatforms=x11 -Dvulkan-drivers=panfrost -Dpanfrost-kmds=kbase \
#   -Dgallium-drivers= -Dllvm=disabled -Dopengl=false -Degl=disabled -Dglx=disabled
# 应用上面 5 个补丁后：ninja src/panfrost/vulkan/libvulkan_panfrost.so
```

| 项 | 值 |
|---|---|
| 未打补丁的上游 `.so` SHA-256 | `599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c` |
| 发行包 `.so` SHA-256 | `5e06324783ec7947d48782441fd1cac23953c90bd0b31663f98f08b22344c1c3` |
| 发行包 `.gz` SHA-256 | `47d11cd2fa0cfc36a7d48028c4d81d0a1a920b9b772c5583fc1e8c0c7b84140b` |
| 机械描述 | `docs/design/patches/fristonerr-blit-single-triangle.meta`、`docs/design/patches/fristonerr-kbase-warmup-skip.meta` |

安装器解压后立即核对上面那个 `.so` SHA-256，不一致就拒绝安装（`install-husk.sh`，
退出码 76）。它**不再**做任何就地字节改动。

**风险与边界**：别的 kbase 设备（典型是 Mali-G57）可能真的需要那段热身，表现是
"第一个 GPU job 被终止"。那种机器上设 `PANVK_KBASE_WARMUP_MS=2000`。本仓库只在
Mali-G77 MC9 上验证过。

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
  `/dev/mali0` 存在时安装本快照。固定安装到 Husk 专用路径
  `usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so`，不覆盖发行版的
  `libvulkan_panfrost.so` 或其他厂商文件。
- Mesa/FristOneRR 代码按 MIT 分发，许可证原文见 `LICENSE-Mesa.txt`。
- 这里不声称它是上游 Mesa 官方构建，也不把它描述成通用的 Android GPU 驱动。
