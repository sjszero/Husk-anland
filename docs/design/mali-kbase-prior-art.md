# Mali kbase 直连：前人实现与可移植结论

> 目的：把「panfrost/panvk 怎么接到厂商 mali_kbase」这件事从猜测变成抄写。
> 全部内容来自下面三个仓库的**源码**（已 clone 读过），标注 FACT / INFERENCE。

## 1. 三个前人实现（都已读源码）

### A. `manoedinata/mesa-kbase-j4lte` —— **与我们架构完全相同**（DRM 语义层）

- 产物：`libdrm-kbase`（`libdrm-kbase/libdrm_kbase.c`，1785 行 C）= 一个 `libdrm.so` 替代品，
  把 `/dev/mali0` **伪装成 panfrost render node**，让 **未修改思路的 Mesa panfrost gallium** 直接用。
- 另有 `patches/mesa/0001-panfrost-weak-hooks-for-libdrm-kbase.patch`：给 Mesa 加两个弱符号
  `drmKbaseMapBO` / `drmKbaseIsBOMapping`，让 Mesa 用 kbase 自己的 SAME_VA 映射（CPU VA == GPU VA）。
- 还有 `patches/mesa-linux-prototype/0001-drm-shim-panfrost-on-kbase.patch`：把 Mesa **自带的
  `src/drm-shim`** 接到 kbase 上（drivers 侧 `bo->map` 直给、`munmap` 不放掉驱动映射）。
- 平台：Exynos 7570 / Mali-T720（Midgard v4）/ 内核 3.18 / kbase r29p0 UK 11.13 / **bionic**。
- 结论：**"DRM 语义层 + 原版 Mesa panfrost"是走过的路**，不是我们的发明。

### B. `zenithblue-oss/panvk-kbase-android`（174★）+ `faux123/panvk-faux123-driver-rp4pro`（25★）

- 形态：Mesa **PanVK（Vulkan）**patch 系列，新增 `src/panfrost/lib/kmod/kbase_kmod.c`，直接和 kbase 说话。
- `faux123` 版实测硬件：**Mali-G77 MC9**（Retroid Pocket 4 Pro）——**与本机同代同型**，报 Vulkan 1.3。
- 关键 FACT：**上游 Mesa main 没有 kbase 后端**（2026-11-22 用 gitlab API 核过：`src/panfrost/lib/kmod/`
  只有 `panfrost_kmod.c` / `panthor_kmod.c` / `virtio/`，`kbase_kmod.c` 404）。所以它是 fork 补丁，不是上游能力。
- 附带：`Noysz/panvk-g99-jm`（25★，Valhall v9 **JM**，Helio G99）、
  `apexspan-svg/mesa-panvk-mali-g57`（kbase + **X11**）、`m8l8th814n-eng/mesa-gs101`（写明 G77）。

### C. `BkaNeko/panvk-mtk-driver` —— **MTK 内核侧的坑**（`patches/panvk_mtk.patch`，903 行）

- 设备：天玑 8100 / Mali-G610，kernel 5.10，kbase **r32p1**（与本机同版本号）。
- 已在补丁里遇到的问题：`MEM_ALLOC_EX` 在 MTK 上 **ENOTTY**（要回退 `MEM_ALLOC`）；
  MTK gralloc 报 `DRM_FORMAT_MOD_INVALID` 但实际是 AFBC；CSF doorbell 丢作业；超时放宽。
- ⇒ 对本机（r32p1 / JM）**`MEM_ALLOC_EX` 不要假设可用**。

## 2. kbase UAPI 手册（从 A 的源码抄出来的，r29p0 命名 / r32p1 兼容）

```c
VERSION_CHECK  _IOWR(0x80, 0)   {u16 major, minor}          // 本机 11.38
SET_FLAGS      _IOW (0x80, 1)   {u32 create_flags}
JOB_SUBMIT     _IOW (0x80, 2)   {u64 addr; u32 nr_atoms, stride}   // 0x40108002
GET_GPUPROPS   _IOW (0x80, 3)   {u64 buffer; u32 size, flags}      // 0x40108003
MEM_ALLOC      _IOWR(0x80, 5)   in{va_pages,commit_pages,extent,flags} out{flags,gpu_va}
MEM_FREE       _IOW (0x80, 7)   {u64 gpu_addr}
MEM_IMPORT     _IOWR(0x80, 22)  in{flags,phandle,type,pad} out{flags,gpu_va,va_pages}
STICKY_RESOURCE_MAP _IOW(0x80,29) {u64 count, address}
```

- `BASE_MEM_*`：CPU_RD 1<<0、CPU_WR 1<<1、GPU_RD 1<<2、GPU_WR 1<<3、GPU_EX 1<<4、
  GROW_ON_GPF 1<<9、SAME_VA 1<<13、IMPORT_TYPE_UMM = 2。
- `BASE_JD_REQ_*`：FS 1<<0、CS 1<<1、T 1<<2、COHERENT_GROUP 1<<6、
  SOFT_FENCE_TRIGGER (1<<9)|0x2、SOFT_FENCE_WAIT (1<<9)|0x3。
- `base_jd_atom_v2`（Samsung r29p0 = 64 B；**本机 r32p1 实测 stride 72**，见 evidence 40）：
  `jc, udata[2], extres_list, nr_extres, compat_core_req, pre_dep[2](atom_id,dep_type), atom_number, prio, device_nr, pad, core_req, ...`
- 依赖类型：`DATA = 1`、`ORDER = 2`；完成事件 `BASE_JD_EVENT_DONE = 0x01`。

## 3. 提交模型（A 的实现，可直接照抄逻辑）

1. **握手**：`VERSION_CHECK` → `SET_FLAGS(0)` → `mmap(4096, PROT_NONE, fd, 3<<12)`（**跟踪页，不映射则
   `MEM_ALLOC` 返回 EINVAL**）→ `GET_GPUPROPS`（变长记录流：`header=(id<<2)|size_code`）→
   `fcntl(F_SETFL, fl|O_NONBLOCK|O_CLOEXEC)`（kbase 的 check_flags 要求带 O_CLOEXEC）。
2. **BO**：SAME_VA 分配，CPU VA == GPU VA；exec BO 从**一个 16 MiB exec 区**里子分配（Midgard 24 位 shader PC 限制；
   Valhall 是否仍有此限制 = 待验）。关闭忙 BO 要延迟到提交完成（zombie 表）。
3. **提交**：每个 DRM submit → 1..N 个 atom，串成依赖链：第一个 atom 与前一个 submit 的末 atom
   `ORDER` 依赖，链内用 `DATA`。FS(片元) 用 `COHERENT_GROUP|FS`，其它用 `COHERENT_GROUP|CS|T`。
4. **in-fence** → `SOFT_FENCE_WAIT` 软 atom（GPU 等，不让 CPU 等）；**out-fence** →
   `SOFT_FENCE_TRIGGER` 软 atom 打在一个 `STREAM_CREATE` 的 kbase stream 上，syncobj 等该 stream。
5. **等 BO 空闲**：不靠内核，用自己的 submit seq / done seq 簿记（事件从 kbase fd 读）。
6. **链拆分**（Midgard 必需，Valhall 待验）：tiler job 与 vertex job 同链时 kbase 下 tiler 不出图元，
   需拆成两个 atom（先非 tiler 后 tiler，跨组的 dep 字段清 0）。
7. **dma-buf 导入**：`MEM_IMPORT(UMM)` + 每个导入必须 `STICKY_RESOURCE_MAP`，否则 GPU 翻译故障；
   按 ION handle 去重；kbase 对 UMM 的 SAME_VA 映射**不可 CPU 访问（SIGBUS）**，CPU 侧要映射 dma-buf 本身。

## 4. 对本项目的结论（判断 JUDGMENT）

- **主路径不变**：Ubuntu 24.04 自带 Mesa 25.2.8 的 panfrost gallium + 我们的 Rust DRM 语义层。
  理由：唯一不需要重编 Mesa、不需要 bionic、不引入 C/Python 的路线；而且 A 证明它在 kbase 上可行。
- **A 的实现直接当规格书**：我们 Rust shim 的 submit / fence / BO 生命周期按它写，不再自己猜 atom。
- **兜底路径（不改目标，只留后手）**：若 Valhall 上 panfrost gallium 走不通，改走 B ——
  把 panvk kbase fork 编成 **glibc** 版 Vulkan 驱动，GL 交给发行版自带的 **Zink**（GL on Vulkan）。
  这条已有 G77 实机验证（faux123），代价是要重编 Mesa。
- **待验清单（上机）**：① 本机 atom stride 72 与 r32p1 的 `base_jd_atom_v2` 字段排布；
  ② Valhall 是否需要链拆分；③ exec 区是否仍受 16 MiB / 24 位 PC 限制；④ `MEM_ALLOC_EX` 是否 ENOTTY（MTK 已知会）。
