# Producer / acquire fence：现状、可行桥和真正待办

> 状态：**内核层 pending producer fence 已通过（evidence 57，2026-11-23）**。
> guest 直连 `/dev/mali0` 已生成真实 `sync_file`，同一 fd 在事件设置前 pending、设置后
> readable，`SYNC_IOC_FILE_INFO.status` 由 0 变 1。尚未完成的是把它接进 PanVK
> `vkGetFenceFdKHR` / `SYNC_FD`，以及让 SurfaceFlinger 在 GPU 完成前实际等待它。
>
> eventfd completion bridge 仍保留为 fallback，不得把 eventfd 写成 `SYNC_FD`。

## 1. 旧失败的真实原因（FACT）

当前安装的 FristOneRR PanVK（commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`）使用 kbase
CPU-sync 后端。旧探针结果为：

```text
vk|fence_probe|status|after_submit=1
vk|fence_probe|export|rc=0 fd=-1 elapsed_us=49214
vk|fence_probe|status|after_export=1
```

`kbase_cpu_sync_export_sync_file()` 先 CPU 等待 GPU seqno，然后返回 `fd=-1`；Vulkan
`vk_common_GetFenceFdKHR()` 按 `SYNC_FD` 规范重置原 fence。因此“导出后继续 wait 同一个
VkFence”会挂起。不要重跑那个组合。

早先手搓 soft-fence 的 `JOB_CONFIG_FAULT` / `JOB_CANCELLED` 也不是内核不支持，而是两个
ABI 错误：

1. 混用了 64-byte `base_jd_atom_v2` 与 72-byte `base_jd_atom` 的字段偏移；
2. 把 `SAME_VA` 分配在 `mmap` 前返回的 cookie 当成了 GPU VA。cookie 只用于 mmap offset，
   mmap 后的映射地址才是 GPU VA。

## 2. 已打通的最小内核链（evidence 57）

`gpu-probe fence11` 的真实提交：

```text
atom 1: 64-byte v2, core_req=0x205 SOFT_EVENT_WAIT, jc=<mmap 地址>
atom 2: 64-byte v2, core_req=0x202 SOFT_FENCE_TRIGGER,
        pre_dep[0]={atom_id=1,type=ORDER}, jc=&base_fence
JOB_SUBMIT: nr_atoms=2, stride=64
```

事件 BO 使用 `MEM_ALLOC(flags=0x300f)`，即 `CACHED_CPU | SAME_VA | CPU_RD | CPU_WR |
GPU_RD | GPU_WR`。实测：

```text
JOB_SUBMIT ACCEPTED, base_fence.fd=6
SYNC_IOC_FILE_INFO: status=0, num_fences=1
poll(fd, 250ms): timeout
SOFT_EVENT_UPDATE(event=<mmap VA>, status=1): PASS
poll(same fd): readable
SYNC_IOC_FILE_INFO: status=1
kbase events: atom 1 DONE, atom 2 DONE
```

这证明“真实 dma-fence/sync_file 只能由 Android bionic 侧生成”是错误的。kbase 的 soft-job
路径在 glibc guest 进程里就能生成并 signal 真正的 sync_file。

## 3. 当前可用桥（evidence 55 / 56）

eventfd 协议：

1. guest `vkQueueSubmit` 后立即返回，不阻塞 ring 主循环；
2. 创建 `eventfd`，复制一个写端给 Rust completion 线程；
3. completion 线程等待该帧 `VkFence`；
4. GPU 完成后向 eventfd 写 8 字节并关闭写端；
5. `READY <slot> <frame> <expected>` 与 eventfd 同一次 `sendmsg(SCM_RIGHTS)` 发给 app；
6. app 把 fd 交给 `ASurfaceTransaction_setBuffer(..., acquire_fd)`；
7. SurfaceFlinger 通过 `ASurfaceTransaction_setOnComplete` 返回 release fence，app 等到
   signal 后再发 `RELEASE <slot>`。

已通过：120 帧功能验证和 5,600 帧 / 134 秒短 soak，release 背压、fd 回收和最终截图均正常。
边界：eventfd 是 pollable completion bridge，不具备 `sync_file` 的 `SYNC_IOC_FILE_INFO` /
signal-time 语义；大多数帧在 app 收到 READY 前 GPU 已完成，不能写成“SF 等待 pending fence”。

## 4. 下一步（按顺序）

1. **PanVK 接入**：在 FristOneRR kbase 后端建立两条软 atom 链，把 `VkFence` 的设备完成映射到
   真实 `sync_file`；`vkGetFenceFdKHR` 不再 CPU wait 后返回 `fd=-1`。
2. **不改变上层语义**：保留 `VK_KHR_external_fence_fd`，但返回的 fd 必须是本证据验证过的
   `mali-mali.timeline*` sync_file；eventfd 只作 fallback。
3. **SurfaceFlinger pending 验收**：在 `sendmsg` 时 fd 必须仍 pending；app/SF 先看到等待，
   GPU 完成后同一 fd 变 readable；随后完成 transaction 并继续 release 背压。
4. **持久资源池与窗口生命周期**：去掉每帧 image/memory/fence 重建，再做旋转 / DPI / 多窗口 /
   SurfaceView 重建和 10 分钟长稳。
5. **降级路径**：同步对象创建失败时回退 eventfd，日志明确写出“不是 SYNC_FD”的原因；任何一条
   都不能崩。

## 5. 不做什么

- 不再运行“导出后无限 wait 同一个 VkFence”的组合；
- 不用 `vkGetFenceFdKHR` 的 `fd=-1` 替代 acquire fence；
- 不把 eventfd 写成 Vulkan 外部 fence；
- 不为了通过测试删除 release fence 背压；
- 不把 evidence 57 的探针结果夸大成 WSI / SurfaceFlinger 已完成。

## 6. 证据边界

- evidence 54：12 帧 ring + release fence 背压；
- evidence 55：eventfd acquire bridge 120 帧；
- evidence 56：同一 eventfd 路径 5,600 帧 / 134 秒短 soak；
- evidence 57：r32p1 kbase 生成真实 pending `sync_file` 的最小内核链；
- 以上证据互不覆盖，旧失败实验和哈希不改写。
