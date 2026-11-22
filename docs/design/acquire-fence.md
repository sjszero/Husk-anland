# Producer / acquire fence：当前实验、结论与最小探针

> 状态：**未通过，未验收**（2026-11-23）。本文件记录失败现场与下一步可复现实验，
> 不把 `vkGetFenceFdKHR` 返回过 `rc=0` 当作 acquire fence 已打通。

## 1. 要解决的问题

evidence 54 已证明 app→guest 的 AHB ring、GPU 渲染、SurfaceControl 上屏和
SurfaceFlinger→guest 的 release-fence 背压可用。当前 guest 在发送 `READY` 前仍先执行
`vkWaitForFences`，因此 app 拿不到“GPU 尚未完成”的 producer fence；GUI 合成器只能默认
buffer 已就绪，或者在 guest 侧先付出一次 CPU 等待。

目标协议：

1. guest `vkQueueSubmit` 后，不先 CPU wait；
2. 立即把同一个 `VkFence` 导出为 `SYNC_FD`；
3. guest 用 `sendmsg(SCM_RIGHTS)` 把 fd 与 `READY <slot> <frame> <expected>` 一起发给 app；
4. app 用 `recvmsg` 收 payload + fd，并作为
   `ASurfaceTransaction_setBuffer(..., acquire_fd)` 的 acquire fence 交给 SurfaceFlinger；
5. app→guest 的 `RELEASE <slot>` 仍由 SF release fence 驱动，不能省。

验收至少要有一次真实的 pending `SYNC_FD >= 0` 被 SurfaceFlinger 接收；如果 workload
太快、导出时 fence 已完成，`-1` 可以作为诊断事实记录，但不能替代上述验收。

## 2. 已观察事实（FACT）

- PanVK 实例/设备扩展枚举包含 `VK_KHR_external_fence` 与
  `VK_KHR_external_fence_fd`。
- 直接 `dlsym("vkGetFenceFdKHR")` 不存在；通过
  `vkGetDeviceProcAddr(device, "vkGetFenceFdKHR")` 可以取得函数指针。
- 在 `vkQueueSubmit` 成功后、`vkWaitForFences` 之前调用导出，日志为：

  ```text
  vk|dmabuf|fence_export|rc=0 fd=-1
  ```

- 在上述导出返回后继续原先的 `vkWaitForFences`，探针挂起，未继续到
  `image_submit/wait`。因此该组合不能直接进入正式 ring 协议。
- 早先“先 `vkWaitForFences`，完成后再导出”的实验同样得到 `rc=0 fd=-1`；这只能证明
  fence 已经 signal，不能证明 acquire fence 路径可用。
- 当前没有足够证据把挂起归因于 PanVK、Vulkan 调用顺序、同一 fence 被导出后状态变化，
  或测试 workload；根因仍是 **UNKNOWN**。

## 3. 安全的最小探针

不要再次运行“导出后无限 wait 同一个 fence”的组合。先做一个独立、单向、可超时的 fence
探针，每一步单独记录，并按以下顺序执行：

1. 创建带 `VkExportFenceCreateInfo`（`SYNC_FD`）的 fence；
2. submit 前记录 `vkGetFenceStatus`；
3. submit 后记录 `vkGetFenceStatus`；
4. 通过 `vkGetDeviceProcAddr` 调 `vkGetFenceFdKHR`，记录 rc 与 fd；
5. 若 fd >= 0，用 `poll(POLLIN)` 记录其完成状态，不阻塞主线程；
6. 再次记录 `vkGetFenceStatus`；
7. 对同一 fence 的 wait 只允许放在子进程或受硬超时保护，并记录是否挂起；
8. 清理 fd / fence，输出一条机器可读结果。

若第 7 步可稳定复现挂起，应改用“不导出就是普通 fence”与“导出后使用独立第二 fence”
两条对照，确认是否是导出操作改变了原 fence 的可等待性。不要在未确认前扩大进 ring 代码。

## 4. 通过后的接入顺序

1. 将 fence 探针接入单 slot 的 READY 消息，fd 与文本 payload 同一次 `sendmsg` 发出；
2. app 侧 `recvmsg` 收 fd，记录 fd 生命周期，交给 `ASurfaceTransaction_setBuffer`；
3. 保留 release fence 背压，确认 3 个 slot 不串帧；
4. 增加 GPU 重 workload，至少取得一次 pending fd >= 0 的截图与日志；
5. 新建 evidence 55，记录 APK / guest probe / 日志 / 截图哈希；
6. 再做持久资源池、窗口生命周期、旋转 / DPI、多窗口和 10 分钟 fd / 内存泄漏测试。

## 5. 与 evidence 54 的边界

- evidence 54 的产物和哈希只对应“连续帧 + release fence 背压”；
- 本文件中的失败实验不得覆盖或重写 evidence 54；
- producer / acquire fence 未通过前，`docs/GOALS.md` 和 `docs/STATUS.md` 必须继续把它写成
  `UNKNOWN / 未完成`。
