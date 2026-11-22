# evidence 36 — C5 路线现实核对：Mesa 上游没有 kbase 后端；kbase 内存分配可用

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-22 22:14–22:20（CST）

## 1. 目标里那条路线，上游不存在（FACT）

`docs/GOALS.md` C5 写的是"走 Mesa panfrost/panvk 的 kbase 后端"。实测核对：

| 检查 | 方法 | 结果 |
|---|---|---|
| Mesa 25.2.0 的 panfrost kmod 后端 | `gitlab.freedesktop.org` API 列 `src/panfrost/lib/kmod/` @ `mesa-25.2.0` | 只有 `panfrost_kmod.c`（DRM）、`panthor_kmod.c`（Panthor）、`pan_kmod.c`。**没有 kbase 后端** |
| Mesa main 分支 | 同上 @ `main` | 多了 `virtio/`。**依然没有 kbase** |
| Ubuntu 24.04 rootfs 里的 Mesa（25.2.8） | 拉下 `libgallium-25.2.8-*.so` 查字符串 | 只有 `DRM_IOCTL_PANFROST_*`、`panfrost_kmod_*`；"kbase" 的两处命中是 `BlockBased` 的误匹配 |
| Vulkan | `ls files/rootfs-full/usr/lib/aarch64-linux-gnu/libvulkan_panfrost.so` | 不存在；`/usr/share/vulkan/icd.d` 整个目录都没有 |

结论（FACT）：**用发行版自带的 Mesa 在这台机器上驱动 Mali-G77 是不可能的** —— panfrost 需要
`/dev/dri/renderD128` + panfrost 内核驱动，而这台设备只有 `/dev/mali0`（厂商 kbase）。上游 Mesa 没有
kbase 后端，装再多包也补不上。

## 2. 但这扇门本身是开的（FACT，新增实测）

`gpu-probe alloc1`（guest / untrusted_app）：

    version_check            PASS   -> 11.38
    set_flags                PASS   errno=0
    mem_alloc(va=1,commit=1,flags=0xd)   ret=0  back=[0x200d, 0x41000, 0x0, 0xd]
    mem_alloc(va=1,commit=0,flags=0xd)   ret=0  back=[0x200d, 0x42000, 0x0, 0xd]
    mem_query(gpu_va=0x41000)            EINVAL(22)
    mmap(len=4096, off=0x41000)          EPERM(1)

由此得到两条 ABI 事实：
1. `mem_alloc`（`0xc0208005`）是 **in/out 结构**：返回 0，驱动把"实际 flags"写回字段 0（`0x200d`），
   把 **GPU VA 写回字段 1**（`0x41000`、`0x42000` 连续递增，正是 VA 分配器的行为）。
   flags=0 时返回 ENOMEM ⇒ flags 不是"可选"，必须至少带 `CPU_RW|GPU_RD|GPU_WR`（0xD）。
2. `mmap(2)` 拿这块显存返回 **EPERM**。kbase 只允许对带特定标志的区域做 CPU 映射，
   或 r38 起改用专门的 map 命令（待查：命令面里 nr=13/14/20/23 这些 16/24 字节的写命令）。

## 3. 路线修正（JUDGMENT，需要写进 GOALS）

原路线（发行版 Mesa + kbase 后端）不可行。可行的是 **Rust 侧做 DRM 语义层**：

    panfrost/panvk（发行版 Mesa，未修改）
              ↓  只认 DRM_IOCTL_PANFROST_* 与 mmap(fd, bo_offset)
    husk GPU shim（纯 Rust，: 拦截 open("/dev/dri/renderD128") + ioctl + mmap）
              ↓  翻译成证据 35 的命令字
    /dev/mali0（厂商 kbase 11.38）

为什么这条能成立：panfrost 本来就是"构造 jc + 提交 atom + 等 fence"，DRM 层只是内存管理外壳；
kbase 的 JOB_SUBMIT 收的也正是 jc 指针。两端语义对得上，工作量大头在 BO/VA 管理与 fence。

代价与风险：
* Mesa 的 `drm_panfrost_*` 结构体布局必须逐字段对齐（可从 libdrm 头取，或从 libgallium 反读）；
* mmap 语义（DRM 的 bo_offset）要映射到 kbase 的 VA 映射，EPERM 这一关必须先解决；
* 工作量按周计，不是按小时计。

## 4. 立即可做的下一步

1. 解掉 `mmap EPERM`：从厂商 .so 找它怎么把显存映射给 CPU（找 mmap@plt 调用点，看 offset 里塞了什么标志位）。
2. `mem_query` 结构按实测重新对齐（现在 16 字节结构是猜的，返回 EINVAL）。
3. 只有这两步通了，才谈 BO 管理与 atom 提交。
