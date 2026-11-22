# Producer / acquire fence：现状、可行桥和真正待办

> 状态：**部分通过（2026-11-23）**。eventfd 作为 SurfaceControl 的 pollable acquire fd
> 已连续跑完 120 帧；Vulkan `SYNC_FD` / 真正 pending producer fence 仍未打通。
> 不把 `vkGetFenceFdKHR` 返回过 `rc=0` 当作可用 fence，也不把 eventfd 写成 `SYNC_FD`。

## 1. 根因（FACT）

当前安装的 FristOneRR PanVK（commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`）使用 kbase
CPU-sync 后端。真机探针确认：

```text
vk|fence_probe|status|after_submit=1
vk|fence_probe|export|rc=0 fd=-1 elapsed_us=49214
vk|fence_probe|status|after_export=1
```

`kbase_cpu_sync_export_sync_file()` 先 CPU 等待 GPU seqno，然后返回 `fd=-1`；Vulkan
`vk_common_GetFenceFdKHR()` 按 `SYNC_FD` 规范重置原 fence。因此“导出后继续 wait 同一个
VkFence”会挂起，这是预期行为，不是随机驱动 bug。不要重跑那个组合。

## 2. 当前可用桥（evidence 55）

协议：

1. guest `vkQueueSubmit` 后立即返回，不阻塞 ring 主循环；
2. 创建 `eventfd`，复制一个写端给 Rust completion 线程；
3. completion 线程等待该帧 `VkFence`；
4. GPU 完成后向 eventfd 写 8 字节并关闭写端；
5. `READY <slot> <frame> <expected>` 与 eventfd 同一次 `sendmsg(SCM_RIGHTS)` 发给 app；
6. app 用 `recvmsg` 同时收 payload + fd，交给
   `ASurfaceTransaction_setBuffer(..., acquire_fd)`；
7. SurfaceFlinger 继续通过 `ASurfaceTransaction_setOnComplete` 返回上一 buffer 的 release
   fence，app 等它 signal 后再发 `RELEASE <slot>`。

实测结果：

```text
app : ahb|ring|SUMMARY|submitted=120|completed=120|guest_done=true
app : ahb|ring|DONE|ok=true|buffers_released=3
guest: vk|ring|PASS|rendered=120/120 releases=117 done=true
```

3 个 slot、120 帧、release fence 背压、fd 回收和最后一次蓝色截图均通过。app fd 数在 ring
run 前为 141，cleanup 后为 129；没有按帧增长。详细日志、截图和哈希见
`docs/design/evidence/55-eventfd-acquire-fence.md`。

## 3. 边界（不要混淆）

- **FACT**：SurfaceFlinger 在当前 Android 13 / RMX3357 路径上接受 eventfd 并完成 transaction。
- **FACT**：eventfd 是 pollable completion bridge，不是 Vulkan `SYNC_FD`；它没有
  `sync_file` 的 `SYNC_IOC_FILE_INFO` / signal-time 语义。
- **FACT**：本次大多数帧在 app 收到 READY 前 GPU 已完成，因此 eventfd 往往是已 signal 的；
  不能把它说成“SF 等待了一个 pending producer fence 的真实测量”。
- **INFERENCE**：只依赖 poll(fd) 的 Android 合成路径可使用这个桥；要求真正 sync_file 的
  消费者或未来 Android 版本不能外推。
- **UNKNOWN**：kbase `STREAM_CREATE` + `BASE_JD_REQ_SOFT_FENCE_TRIGGER` 是否能在这台 r32p1
  JM 内核上生成真正的 dma-fence/sync_file；需要用真实 atom 布局和最小合法参数做上机实验。
- **UNKNOWN**：修 PanVK kbase 后端直接返回真实 sync_file 的可行性和成本。

## 4. 下一阶段

### 4.1 真正 pending fence 的验收

需要一个在 `sendmsg` 时仍 pending、GPU 完成后才 readable 的 fd，且 SurfaceFlinger 能读取。
至少记录：

1. `submit` 后 `vkGetFenceStatus == VK_NOT_READY`；
2. 发出的 fd `>= 0`；
3. app/SF 侧 poll 在 GPU 完成前超时或 pending；
4. GPU 完成后同一 fd readable；
5. release backpressure 和截图仍通过。

### 4.2 优先尝试方向

1. 从 MTK / Arm r32p1 的 `base_jd_atom_v2`、soft fence trigger 和 `STREAM_CREATE` 真实布局，
   做一个只产生 completion fence 的最小 atom；全部参数取自厂商客户端或源码，不盲扫。
2. 若 kbase 用户态接口不能直接给 sync_file，评估修 FristOneRR PanVK 的 kbase 后端，让
   `vkGetFenceFdKHR` 返回真实 dma-fence；不把 CPU wait 后伪造 fd 当成完成。
3. 在真 fence 打通前，eventfd 桥保留为当前可用 fallback，并在日志中标记其非 `SYNC_FD` 性质。

### 4.3 不做什么

- 不再运行“导出后无限 wait 同一个 VkFence”的组合；
- 不用 `vkGetFenceFdKHR` 的 `fd=-1` 替代 acquire fence；
- 不把 eventfd 写成 Vulkan 外部 fence；
- 不为了通过测试删除 release fence 背压。

## 5. 与旧 evidence 的边界

- evidence 54 的产物只对应 12 帧 ring 和 release fence 背压；
- evidence 55 的产物只对应 eventfd acquire bridge 的 120 帧；
- 旧的失败实验和哈希不改写；
- 在真正 pending `SYNC_FD` 验收前，`docs/GOALS.md` 和 `docs/STATUS.md` 必须继续把
  “真正 producer fence”写成未完成。
