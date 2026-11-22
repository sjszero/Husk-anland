# Evidence 58 — PanVK 真实 SYNC_FD 接入 SurfaceControl ring

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191+ / 4 KiB 页  
时间：2026-11-23 05:19（CST）  
结论：**PASS（PanVK producer fence -> SurfaceControl 120 帧）**。guest 内
`vkGetFenceFdKHR(VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT)` 已返回真实
`mali-mali.timeline*` sync_file，而不是 eventfd 或 `fd=-1`。3-buffer ring 连续 120 帧
全部以 `SYNC_FD` 提交，app 全部完成 transaction 并回收 3 个 buffer。此次仍不是完整
WSI / swapchain 验收，详见 §5 边界。

## 1. 实现（FACT）

FristOneRR PanVK 源码 commit：

```text
0c9dce35094dbe89c412b79bf57c6067c7b97e59
```

改动共 7 个文件，完整 patch：

```text
docs/design/patches/fristonerr-kbase-syncfd-export.patch
```

关键链路：

1. 补齐 `KBASE_IOCTL_STREAM_CREATE`，在 kbase context 内持有 JM out-fence stream；
2. PanVK 的 `kbase_cpu_sync` 保存本次 submit 的最后一个真实 GPU atom；
3. `kbase_cpu_sync_export_sync_file()` 不再 CPU wait 后返回 `-1`，而是提交一个
   `SOFT_FENCE_TRIGGER`，用 `ORDER` 依赖目标 atom，让内核生成并回写真实 sync_file fd；
4. Rust probe 创建带 `VkExportFenceCreateInfo(SYNC_FD)` 的 VkFence，submit 后立即调用
   `vkGetFenceFdKHR`；
5. 把导出 fd 交给 `ASurfaceTransaction_setBuffer`，同时 `dup` 一份给 completion 线程；
   completion 线程 poll 该 sync_file，readable 后才回读并销毁本帧 Vulkan 资源；
6. eventfd 路径保留为显式 fallback；本次 120 帧没有触发 fallback。

## 2. 真机结果（FACT）

guest：

```text
vk|ring|BEGIN|buffers=3 frames=120 64x64
vk|ring|PASS|rendered=120/120 releases=117 done=true
vk|ring|teardown|all received AHB fds closed
vk|teardown|PASS|device/instance/loader released
```

app：

```text
ahb|ring|SUMMARY|submitted=120|completed=120|guest_done=true
ahb|ring|DONE|ok=true|buffers_released=3
```

逐项计数：

| 检查项 | 结果 |
|---|---:|
| `READY` | 120 |
| `kind=SYNC_FD` | 120 |
| export 时立即 poll 为 `PENDING` | 11 |
| export 时立即 poll 为 `ALREADY_SIGNALED` | 109 |
| completion 后同一 fd `status=1` | 120 |
| 9 点 GPU 写回 readback PASS | 120 |
| eventfd / fallback | 0 |
| guest `FAIL` | 0 |
| app `FAIL` | 0 |
| app transaction complete | 120 |
| 最终回收 buffer | 3 |

典型 pending 帧：

```text
vk|dmabuf|sync_file(export frame=118 slot=1)|PASS|rc=0
  name="mali-mali.timeline110151-2" status=0 num_fences=1
vk|dmabuf|sync_file(export poll)|PENDING
vk|dmabuf|image_async|frame=118 slot=1 acquire_fd=18 kind=SYNC_FD submit=PASS
vk|dmabuf|sync_file(signaled frame=118 slot=1)|PASS|rc=0
  name="mali-mali.timeline110151-2" status=1 num_fences=1
vk|dmabuf|image_complete|frame=118 slot=1 sync_file=READABLE elapsed_us=95
vk|ring|READY|frame=118 slot=1 rendered=true acquire_fd=18 sent=true
```

这说明真实 producer fence 不仅“能生成”，而且已在同一条 PanVK → SurfaceControl ring
路径里以 pending fd 交出去。大多数帧因 GPU 在导出前后极快完成而已是
`ALREADY_SIGNALED`；严格 pending 语义另由 evidence 57 的 250 ms 超时测试证明。

最后截图 `58-panvk-syncfd-ring.png` 为蓝色，是 3 色循环中的合法最后一帧，不是黑屏。

## 3. 与旧实现的区别

| 旧 eventfd bridge（evidence 55 / 56） | 本次 SYNC_FD（evidence 58） |
|---|---|
| VkFence 完成后由 Rust 线程写 eventfd | 导出内核生成的 `mali-mali.timeline*` sync_file |
| 不具备 sync_file signal-time 语义 | `SYNC_IOC_FILE_INFO.status` 0 -> 1 |
| GPU 完成后才可通知 | fd 在 GPU 完成前即可 pending |
| 只证明 SurfaceControl 接受 pollable fd | 证明 SurfaceControl 路径收到真实 Vulkan producer fence |

## 4. 复现命令

```sh
# 1. 构建 probe
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu \
-C link-arg=--sysroot=/tmp/gccroot \
-C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
-C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
cargo build --release --target aarch64-unknown-linux-gnu -p vulkan-probe

# 2. 部署并配置 3-buffer / 120 帧
adb -s UKHMMB8PYTGECMNV push \
  target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-syncfd-ring3
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk cp /data/local/tmp/vulkan-probe-syncfd-ring3 files/rootfs-full/usr/bin/vulkan-probe-syncfd-ring3'
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk chmod 755 files/rootfs-full/usr/bin/vulkan-probe-syncfd-ring3'
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "printf \"ring=3\\nframes=120\\n\" > files/ahb-bridge"'
adb -s UKHMMB8PYTGECMNV shell \
  'am force-stop com.linuxhub.husk; logcat -c; am start -n com.linuxhub.husk/android.app.NativeActivity'

# 3. guest 运行
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "\
cd /data/user/0/com.linuxhub.husk/files; \
export VK_ICD_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_syncfd.json \
VK_DRIVER_FILES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_syncfd.json \
PANVK_DEBUG=startup PANVK_KBASE_DVFS=none; \
timeout 180 sh gexec2.sh rootfs-full \
/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/bin/vulkan-probe-syncfd-ring3 \
--ahb-socket /data/user/0/com.linuxhub.husk/files/ahb-bridge.sock 2>&1"'
```

## 5. 边界（UNKNOWN / 未完成）

- **FACT**：本次 120 帧都走真实 `SYNC_FD`，app/SF transaction 全部完成，release fence 背压
  正常，3 个 buffer 最终回收。
- **UNKNOWN**：没有 SurfaceFlinger 内部 trace，不能断言 SF 对每一个 pending 帧都显式等待了
  producer fence；能确认的是真实 sync_file fd 已被交给 `ASurfaceTransaction_setBuffer`，
  且 transaction 成功完成。
- **未完成**：持久 Vulkan image / memory / command buffer / fence 资源池；当前仍逐帧创建。
- **未完成**：`VK_KHR_swapchain` / Wayland / X11 WSI、SurfaceView 重建、旋转 / DPI / 多窗口。
- **未完成**：10 分钟持续帧流、泄漏门禁，以及新的 16 KiB page-size 设备验证。
- **未完成**：视频硬解、音频、2.0 proot 替换。本证据只关闭“PanVK producer fence 能接进
  SurfaceControl ring”这一段。

## 6. 产物与哈希

```text
driver source commit: 0c9dce35094dbe89c412b79bf57c6067c7b97e59
driver patch sha256 : 0afcb634a99fb253dfe2b80b607dce8262ee0d5b6f9ba6ad513751d5fe4cbe47
driver .so sha256   : e6d1b6fed4b09ff1e84474b160b8898b8adcc4effd01d08c87d129091c838f5e
probe sha256        : 28095769e2313e6fa65a9bad7748dce94fe1d05381cb2bec468bbe24a2ca6ac0
app APK sha256      : 78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d
guest log sha256    : 8f2aab3f754e2d1375b460373221c8bcbf5af31bb97dcc5485ec76f637217bb0
app log sha256      : 0295e08abdd745316182fe5ace8fd428c779634e20d2884091fec6a133fd7881
screenshot sha256   : 2390f61c8e60146e8da7a14124af9fde5a95eb62e38f9472242879c7f07ecc98
```

相关文件：

- `docs/design/evidence/58-panvk-syncfd-ring-guest.log`
- `docs/design/evidence/58-panvk-syncfd-ring-app.log`
- `docs/design/evidence/58-panvk-syncfd-ring.png`
- `docs/design/evidence/58-panvk-syncfd-ring.sha256`
- `docs/design/patches/fristonerr-kbase-syncfd-export.patch`
- `docs/design/patches/fristonerr-kbase-syncfd-export.meta`
