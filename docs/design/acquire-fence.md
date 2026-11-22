# Producer / acquire fence：现状、可行桥和真正待办

> 状态：**PanVK → SurfaceControl ring 已通过（evidence 58，2026-11-23）**。
> FristOneRR PanVK 的 `vkGetFenceFdKHR(SYNC_FD)` 已返回真实 `mali-mali.timeline*`
> sync_file；3-buffer ring 连续 120 帧全部使用该 fd，其中 11 帧导出时仍 pending。
> app 完成 120/120 transaction，eventfd fallback 为 0。
>
> 尚未完成的是持久资源池、`VK_KHR_swapchain` / Wayland / X11 WSI、窗口生命周期、
> 10 分钟长稳，以及 SurfaceFlinger 内部等待行为的 trace。eventfd 仍只保留为 fallback，
> 不得写成 `SYNC_FD`。

## 1. 旧失败的真实原因（FACT）

旧版 FristOneRR PanVK（commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`）使用 kbase
CPU-sync 后端。旧探针结果：

```text
vk|fence_probe|status|after_submit=1
vk|fence_probe|export|rc=0 fd=-1 elapsed_us=49214
vk|fence_probe|status|after_export=1
```

旧的 `kbase_cpu_sync_export_sync_file()` 先 CPU 等待 GPU seqno，然后返回 `fd=-1`；Vulkan
`vk_common_GetFenceFdKHR()` 按 `SYNC_FD` 规范重置原 fence。因此“导出后继续 wait 同一个
VkFence”会挂起。不要重跑那个组合。

早先手搓 soft-fence 的 `JOB_CONFIG_FAULT` / `JOB_CANCELLED` 也不是内核不支持，而是两个
ABI 错误：

1. 混用了 64-byte `base_jd_atom_v2` 与 72-byte `base_jd_atom` 的字段偏移；
2. 把 `SAME_VA` 分配在 `mmap` 前返回的 cookie 当成了 GPU VA。cookie 只用于 mmap offset，
   mmap 后的映射地址才是 GPU VA。

## 2. kbase pending sync_file（evidence 57）

`gpu-probe fence11` 的真实提交：

```text
atom 1: 64-byte v2, core_req=0x205 SOFT_EVENT_WAIT, jc=<mmap 地址>
atom 2: 64-byte v2, core_req=0x202 SOFT_FENCE_TRIGGER,
        pre_dep[0]={atom_id=1,type=ORDER}, jc=&base_fence
JOB_SUBMIT: nr_atoms=2, stride=64
```

实测：

```text
JOB_SUBMIT ACCEPTED, base_fence.fd=6
SYNC_IOC_FILE_INFO: status=0, num_fences=1
poll(fd, 250ms): timeout
SOFT_EVENT_UPDATE(event=<mmap VA>, status=1): PASS
poll(same fd): readable
SYNC_IOC_FILE_INFO: status=1
kbase events: atom 1 DONE, atom 2 DONE
```

这证明“真实 dma-fence/sync_file 只能由 Android bionic 侧生成”是错误的。

## 3. PanVK 导出真实 fence（evidence 58）

7 文件 driver patch：

```text
docs/design/patches/fristonerr-kbase-syncfd-export.patch
```

链路：

1. 通过 `KBASE_IOCTL_STREAM_CREATE` 在 kbase context 内建立 JM out-fence stream；
2. PanVK submit 保存最后一个真实 GPU atom；
3. `kbase_cpu_sync_export_sync_file()` 为目标 atom 提交
   `SOFT_FENCE_TRIGGER(ORDER dep)`，由内核生成 sync_file 并回写 fd；
4. Rust probe 用带 `VkExportFenceCreateInfo(SYNC_FD)` 的 VkFence 调
   `vkGetFenceFdKHR`；
5. 导出的 fd 交给 `ASurfaceTransaction_setBuffer`，`dup` 的一份给 completion 线程；
6. completion 线程 poll 到 readable 后才回读并销毁本帧资源。

120 帧结果：

```text
READY / kind=SYNC_FD                120 / 120
export poll: PENDING                 11
export poll: ALREADY_SIGNALED       109
completion: same fd status=1        120
GPU 9-point readback PASS           120
eventfd fallback                      0
guest FAIL / app FAIL              0 / 0
app transaction complete        120 / 120
final buffers released                 3
```

结论（FACT）：真实 PanVK producer fence 已进入 SurfaceControl ring，release 背压和 fd 回收
正常。这个证据关闭“`vkGetFenceFdKHR` 只能返回 `-1`”和“SF ring 只能吃 eventfd”的问题。

## 4. eventfd fallback（evidence 55 / 56）

eventfd 协议仍保留：

1. guest `vkQueueSubmit` 后立即返回；
2. 创建 `eventfd`，复制写端给 Rust completion 线程；
3. completion 线程等待该帧 `VkFence`；
4. GPU 完成后写 8 字节并关闭写端；
5. payload 与 eventfd 通过同一次 `sendmsg(SCM_RIGHTS)` 发给 app；
6. app 把 fd 交给 `ASurfaceTransaction_setBuffer`；
7. release fence signal 后 app 才发 `RELEASE <slot>`。

已通过 120 帧功能验证和 5,600 帧 / 134 秒短 soak。边界不变：eventfd 不是 `sync_file`，
不具备 signal-time / `SYNC_IOC_FILE_INFO` 语义，只能在真实 fence 创建失败时降级使用。

## 5. 下一步（按顺序）

1. **持久资源池**：去掉每帧创建 / 导入 / 销毁 image、memory、command buffer、fence，
   建立 3-slot 固定资源与 fence 生命周期。
2. **SurfaceFlinger pending 验证**：用系统 trace 或更慢的 GPU payload 捕捉 SF 真正等待
   pending sync_file 的时间段；当前只证明 fd 已提交且 transaction 完成。
3. **窗口系统**：推进 `VK_KHR_swapchain` / Wayland / X11 WSI，再测旋转、DPI、多窗口与
   SurfaceView 重建。
4. **长稳**：持久资源池完成后跑连续 10 分钟、每帧释放与泄漏门禁。
5. **降级路径**：真实 fence 创建失败时回退 eventfd，日志必须明确写出降级原因；任何一条
   都不能崩。

## 6. 不做什么

- 不再运行“导出后无限 wait 同一个 VkFence”的组合；
- 不用 `vkGetFenceFdKHR` 的 `fd=-1` 替代 acquire fence；
- 不把 eventfd 写成 Vulkan 外部 fence；
- 不为了通过测试删除 release fence 背压；
- 不把 evidence 58 的 120 帧 ring 写成完整 WSI / swapchain / 10 分钟验收。

## 7. 证据边界

- evidence 54：12 帧 ring + release fence 背压；
- evidence 55：eventfd acquire bridge 120 帧；
- evidence 56：eventfd 路径 5,600 帧 / 134 秒短 soak；
- evidence 57：r32p1 kbase 生成真实 pending `sync_file`；
- evidence 58：PanVK 导出真实 `SYNC_FD`，SurfaceControl ring 连续 120 帧；
- 以上证据互不覆盖，旧失败实验和哈希不改写。
