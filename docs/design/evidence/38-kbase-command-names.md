# evidence 38 — 26 条 kbase 命令字对上了名字；MEM_FREE 已实测闭环

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-22 22:52（CST）

## 1. 方法

两条证据交叉：

1. **我们实测到的命令字**（evidence 35）：从厂商 `libGLES_mali.so` 反汇编出的 26 个 `_IOC` 编码；
2. **公开的 kbase UAPI 头**：`drivers/gpu/arm/midgard/mali_kbase_ioctl.h`（rockchip 4.19 树，Arm 原始 DDK 头），
   里面每个 ioctl 都有 `nr + dir + 结构体大小`。

两者编码一致（dir/size/nr 三元组逐条对得上），所以可以直接命名——不是猜。

## 2. 对照表（FACT）

| 命令字 | dir/size | nr | 名称 | 用途 |
|---|---|---|---|---|
| `0xc0048000` | RW,4 | 0 | VERSION_CHECK | 已通 |
| `0x40048001` | W,4 | 1 | SET_FLAGS | 已通（建 kctx） |
| `0x40108002` | W,16 | 2 | **JOB_SUBMIT** | 已通（evidence 37） |
| `0x40108003` | W,16 | 3 | GET_GPUPROPS | 未用 |
| `0xc0208005` | RW,32 | 5 | **MEM_ALLOC** | 已通 |
| `0xc0108006` | RW,16 | 6 | MEM_QUERY | EINVAL，语义待查 |
| `0x40088007` | W,8 | 7 | **MEM_FREE** | ✅ 见 §3 |
| `0x8004800c` | R,4 | 12 | DISJOINT_QUERY | 未用 |
| `0x4010800d` | W,16 | 13 | GET_DDK_VERSION | 未用 |
| `0x4018800e` | W,24 | 14 | MEM_JIT_INIT | 未用 |
| `0x4020800f` | W,32 | 15 | MEM_SYNC | 缓存维护，后面要用 |
| `0x80048011` | R,4 | 17 | GET_CONTEXT_ID | 未用 |
| `0x40048012` | W,4 | 18 | TLSTREAM_ACQUIRE | 未用 |
| `0x40108014` | W,16 | 20 | MEM_COMMIT | 未用 |
| `0xc0208015` | RW,32 | 21 | MEM_ALIAS | 未用 |
| `0xc0188016` | RW,24 | 22 | **MEM_IMPORT** | ← dma-buf 导入，显示零拷贝要用 |
| `0x40188017` | W,24 | 23 | MEM_FLAGS_CHANGE | 未用 |
| `0x40208018` | W,32 | 24 | **STREAM_CREATE** | ← 完成通道（timeline fd） |
| `0x40048019` | W,4 | 25 | FENCE_VALIDATE | 未用 |
| `0x4010801b` | W,16 | 27 | MEM_PROFILE_ADD | 未用 |
| `0x4010801c` | W,16 | 28 | **SOFT_EVENT_UPDATE** | 未用 |
| `0x4010801d` | W,16 | 29 | 未知（比公开头新） | — |
| `0x4010801e` | W,16 | 30 | 未知 | — |
| `0x40088026` | W,8 | 38 | 未知 | — |
| `0xc0208032` | RW,32 | 50 | 未知 | — |
| `0xc0018036` | RW,1 | 54 | 未知 | — |

## 3. MEM_FREE 实测闭环（FACT）

```
gpu|alloc#1|PASS|gpu_va=0x41000
gpu|mem_free 0x40088007|PASS|errno=0 va=0x41000
gpu|alloc#2|PASS|gpu_va=0x41000 reused=true
```

分配 → 释放 → 再分配拿到同一个 VA（0x41000），说明 `0x40088007`（W,8，结构 `{u64 gpu_addr}`）
确实是 MEM_FREE，且 VA 分配器行为符合预期。这一条把"命令字→名字"的映射从"读头对得上"升级成"行为对得上"。

## 4. 对后续的意义

- **完成通道**：`STREAM_CREATE`（24）拿 timeline fd，`FENCE_VALIDATE`（25）验 dma_fence，
  `SOFT_EVENT_UPDATE`（28）推进软事件 —— 这就是"提交后怎么知道 job 跑完"的通道。
- **显示零拷贝**：`MEM_IMPORT`（22）能把 dma-buf fd 导成 GPU VA，正是目标第 3 条要的东西。
- **缓存一致性**：`MEM_SYNC`（15）在 CPU 写完 BO、GPU 读之前必须调，否则 GPU 可能读到旧数据。

## 5. 仍未解决

- `MEM_QUERY`（6）按公开头的 union 打三种 query 全 EINVAL（evidence 37 §4）。可能与 MTK 改动有关，待查。
- nr 29/30/38/50/54 五条新命令没有名字，暂不使用。
- atom 是否真被执行仍 UNKNOWN：需要接上 §4 的完成通道才能判定。
