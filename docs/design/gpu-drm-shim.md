# GPU：把 Mesa panfrost 接到 kbase 上（DRM 语义层）

> 状态：设计定稿；**M2 的前置（硬件参数来源）已实测通过**，见 [evidence 42](evidence/42-gpuprops-feeds-panfrost.md)。
> 依据：[evidence 41](evidence/41-csf-dead-import-only-panfrost.md)、[evidence 36](evidence/36-gpu-route-reality-check.md)。

## 0. 一句话

Mesa 只会说 DRM（`/dev/dri/renderD128` + `PANFROST_*` ioctl）；本机只有 `/dev/mali0` + kbase（厂商私有 ioctl）。
**在中间放一层把 DRM 语义翻译成 kbase**，Mesa 一行不改、不重编，job chain 由 Mesa 的 panfrost 后端生成。

## 1. 为什么是 panfrost，不是 panthor

- FACT（evidence 41）：`CS_GET_GLB_IFACE`（`0xc0188033`）= **ENOTTY**，这棵内核只编了 JM。
- FACT（evidence 41）：rootfs 里有 Mesa **25.2.8** 的 `panfrost_dri.so` 和 `panthor_dri.so`。
- ⇒ panthor（CSF）在本机不可能工作；panfrost（JM）就是 Valhall（G77）的正路。

## 2. 接在哪一层

```
guest 里的 Mesa (panfrost_dri.so)
   │  open("/dev/dri/renderD128")      ← 节点不存在 → shim 的 interpose 造一个假 fd
   │  ioctl(fd, DRM_IOCTL_PANFROST_*)  ← shim 的 dispatcher 翻成 kbase ioctl
   │  mmap(fd, offset, len)            ← shim 的 mmap 钩子翻成 kbase mem_alloc + 真 mmap
   ▼
/dev/mali0 (kbase r32p1, JM)
```

全部在 **guest 进程内**完成：没有中转进程、没有 socket 转发像素、没有额外服务。这一层是
运行时 `path-shim` 里的一个模块（不是独立 daemon）。

## 3. ioctl 映射表

DRM 编码：`_IOC(dir, 'd', 0x40+nr, size)`；`SUBMIT` 是 `IOW`，其余 `IOWR`。

| DRM（panfrost_drm.h） | nr | 翻成 kbase | 备注 |
|---|---|---|---|
| `SUBMIT` | 0x00 | `KBASE_IOCTL_JOB_SUBMIT`（`0x40108002`） | atom：`jc` = DRM 给的 GPU VA，`core_req` = `BASE_JD_REQ_FS`（3D）/`CS`（compute），`core_req@0x34` |
| `WAIT_BO` | 0x01 | 无（用 kinstr 记录等） | panfrost 只等"这个 BO 已空闲"，用我们的完成簿记回答 |
| `CREATE_BO` | 0x02 | `KBASE_IOCTL_MEM_ALLOC`（`0xc0208005`） | 返回的 **offset 直接用 kbase 回写的 GPU VA** |
| `MMAP_BO` | 0x03 | 同上 + mmap 钩子 | DRM 语义：这里只返回 offset，真 mmap 在下一步 |
| `GET_PARAM` | 0x04 | `KBASE_IOCTL_GET_GPUPROPS`（**`0x40108003`，_IOW**）+ 变长记录流 | 见 §4；24/24+16/16 已实测可答（evidence 42） |
| `GET_BO_OFFSET` | 0x05 | 查表 | 同 CREATE_BO 的 VA |
| `MADVISE` | 0x08 | `MEM_FLAGS_CHANGE`（`0x40088017`）/ 忽略 | 提示性质 |
| `SET_LABEL_BO` | 0x09 | 忽略 | 调试用 |
| `JM_CTX_CREATE/DESTROY` | 0x0a/0x0b | 建/销毁 kbase context | 对应 `SET_FLAGS` + 我们的 kctx 表 |
| `SYNC_BO` | 0x0c | `MEM_SYNC`（`0x4020800f`）或忽略 | 缓存维护 |
| `QUERY_BO_INFO` | 0x0d | 查表 + `MEM_QUERY` | |
| `PERFCNT_ENABLE/DUMP` | 0x06/0x07 | ENOSYS | Mesa 容忍失败 |

## 4. `GET_PARAM` 是生死线

panfrost 起来第一件事就是问一堆硬件参数（GPU_ID / revision / features / thread 限制 / tiler / L2）。
答错或答 0，Mesa 直接放弃 GPU、退回 swrast。

- **已实测（evidence 42）**：panfrost 要的 24 个主参数 + 16 个 `JS_FEATURES` 全部能从 kbase `GET_GPUPROPS` 拿到，不需要伪造。
- 能从 kbase `GET_GPUPROPS` 直接拿的：`JS_PRESENT`、`AS_PRESENT`、`GPU_FEATURES`、`GPU_ID`、`THREAD_MAX_*`、`L2_FEATURES`、`TILER_FEATURES`、`CORE_FEATURES`。
- 拿不到的（寄存器级的 tiler heap 大小、L2 slice 数等）：先按 **同代 G77 的公开值**填，并在日志里标 `SYNTHETIC`。
- 验收标准不是"值对"，而是"`panfrost_dri` 能过 probe 阶段并建出 kctx"。

## 5. 零拷贝：方向已经反了（evidence 41）

r32 UAPI **没有 export**，只有 `MEM_IMPORT`（`0xc0188016`，实测 REACHED/ENOMEM）。
所以显示那条路是：

```
app 分配 AHardwareBuffer（gralloc dma-buf fd）
   → fd 交给 guest（运行时通道）
   → guest MEM_IMPORT 拿到 GPU VA
   → GPU 直接渲染进去
   → app 把同一块 AHardwareBuffer queueBuffer 给 SurfaceFlinger（零拷贝）
```

kbase 自己分配的 BO（渲染中间缓冲）走 CPU 拷贝回 app，这一步的开销要实测并写进 evidence。

## 6. 里程碑（每步都可上机验，不许跳）

| # | 目标 | 判据 |
|---|---|---|
| M1 | 假 `renderD128` + `CREATE_BO`/`MMAP_BO` 通 | guest 里 `crate` 小程序能 alloc + 读写一块 BO，内容正确 |
| M2 | `GET_PARAM` 全表答得出 | Mesa 日志里 panfrost probe 通过，不再 fallback 到 swrast |
| M3 | `SUBMIT` 出 START | kinstr 记录里出现 `state=START(1)`（第一次证明我们的 atom 真的进了硬件槽） |
| M4 | 画一帧 | panfrost 跑 `kmscube`/`glmark` 的最小场景，CPU 回读像素非空 |
| M5 | `eglinfo` 报 `Mali-G77` | 第 2 条验收 |

M3 顺带回答 evidence 41 留的那个问题（"硬件 atom 到底进没进槽"）——用 Mesa 生成的真 job chain，
不再手搓。

## 7. 不做的事

- 不实现 panthor/CSF（CSF 已死，evidence 41）。
- 不改 Mesa、不重编 Mesa、不写 C。
- 不引入 virgl / virpipe / 任何 GPU 中转进程（那是 2.0 的做法，要拿掉）。
