# Evidence 56 — 3-buffer eventfd ring 5,600-frame soak

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页
时间：2026-11-23 04:43:21–04:45:35（CST）
结论：**当前 eventfd path 的短时稳定性通过（PASS）**：3 个 AHB slot、5,600 帧、
134 秒，无 `FAIL`，app 与 guest 全部完成并在结束时回收 3 个 buffer。它不是 10 分钟
门禁，也不是真正的 Vulkan pending producer fence。

## 1. 测试内容

沿用 evidence 55 的协议：

1. guest `vkQueueSubmit` 后由 Rust completion 线程等待该帧 `VkFence`；
2. GPU 完成后向 eventfd 写 8 字节；
3. `READY <slot> <frame> <expected>` 与 eventfd 同一次 `sendmsg(SCM_RIGHTS)` 发给 app；
4. app 将 acquire fd 交给 `ASurfaceTransaction_setBuffer`；
5. SurfaceFlinger 的 previous-release fence signal 后，app 发 `RELEASE <slot>`；
6. 3 个 slot 循环复用，目标 5,600 帧。

配置：

```text
ring=3
frames=5600
```

## 2. FACT：真机结果

```text
app : ahb|ring|SUMMARY|submitted=5600|completed=5600|guest_done=true
app : ahb|ring|DONE|ok=true|buffers_released=3
guest: vk|ring|PASS|rendered=5600/5600 releases=5597 done=true
guest: vk|ring|teardown|all received AHB fds closed
guest: vk|teardown|PASS|device/instance/loader released
```

- guest 日志有 **5,600** 条 `image_complete`；
- guest 日志 `FAIL` 计数为 0；
- app 日志 `FAIL` 计数为 0（logcat 只保留尾部记录，完整 guest 日志在 `56-...guest.log`）；
- 最后一帧截图 `56-eventfd-ring-soak.png` 正文为绿色，说明 SurfaceFlinger 链路仍在出画；
- 耗时约 **134 秒**。这不是目标要求的连续 10 分钟，不能写成 10 分钟长稳通过。

## 3. 这次没有证明什么

- **不是 `SYNC_FD`**：eventfd 是 completion bridge，不具备 `sync_file` 的 signal-time /
  `SYNC_IOC_FILE_INFO` 语义；
- **不是 pending producer fence**：completion 线程仍先等待 VkFence，eventfd 在
  submit 后才被 signal；
- **不是持久资源池**：每帧仍创建、导入、绑定并销毁 Vulkan image/memory/fence，
  不能据此推断最终性能；
- **不是 10 分钟泄漏门禁**：本次只有 134 秒；旋转 / DPI / 多窗口 / SurfaceView 重建也未测试。

## 4. 产物哈希

```text
app log sha256   : 681699d7cd5eea98b8d4cf98a2e09579fc78a3ad97ee94c11f688ca865a5179b
guest log sha256 : 1c4898d9e0938a9727bfced2fde52fa9ede6375f56fa6944175706a2270cf2b4
screenshot sha256: e4f2769ce16606884fa99542bbac494958fb4120e063e3dc8015265bdf29757f
guest probe sha256: fc395bbe037617ed658a7a751bfbf38a1ea15102baba203e345f572bab7912e6
app APK sha256   : 78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d
```

## 5. 复现命令

```sh
# 配置 3 slots / 5600 frames
printf 'ring=3\nframes=5600\n' | adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "cat > files/ahb-bridge"'

# 启动 app 后，在 guest 中运行
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c '\''\
 F=/data/user/0/com.linuxhub.husk/files; \
 R=$F/rootfs-full; \
 ICD=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json; \
 export VK_ICD_FILENAMES=$ICD VK_DRIVER_FILES=$ICD \
 PANVK_DEBUG=startup PANVK_KBASE_DVFS=none; \
 cd $F; timeout 900 sh gexec2.sh rootfs-full \
 $F/vulkan-probe-ring-async --ahb-socket $F/ahb-bridge.sock'\'''
```

注意：本次 probe 已修正成功 `sendmsg` 后读取陈旧 errno 的日志问题；结果日志中的
`sendmsg ... sent=...` 不应再带伪 `Bad file descriptor`。

## 6. 相关文件

- `docs/design/evidence/56-eventfd-ring-soak-app.log`
- `docs/design/evidence/56-eventfd-ring-soak-guest.log`
- `docs/design/evidence/56-eventfd-ring-soak.png`
