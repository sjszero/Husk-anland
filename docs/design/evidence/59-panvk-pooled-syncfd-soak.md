# Evidence 59 — PanVK SYNC_FD 持久资源池与 10 分钟长稳

设备：RMX3357 / MT6893 / Mali-G77 MC9 / 4 KiB / Android 13 / kernel 4.19.191 / arm64  
时间：2026-11-23 05:30:50–05:41:55（CST）  
结论：**PASS**。3-buffer SurfaceControl ring 使用固定 Vulkan 资源池连续运行
**10,000 帧 / 665.385 秒（11 分 5 秒）**；10,000 帧全部是真实 PanVK `SYNC_FD`，
completion 与 9 点 GPU 回读均 10,000/10,000，事件 fallback 为 0，app 完成
10,000/10,000 transaction，最终回收 3 个 AHB。

## 1. 实现（FACT）

实现文件：

```text
runtime/crates/vulkan-probe/src/dmabuf.rs
```

每个 AHB slot 固定持有一套：

```text
external VkImage + imported dma-buf VkDeviceMemory
readback VkBuffer + VkDeviceMemory
VkCommandPool + VkCommandBuffer
exportable SYNC_FD VkFence
```

生命周期规则：

1. 只有 app 发出 `RELEASE <slot>` 后，该 slot 才允许再次使用；
2. 复用前必须 `join` 上一帧 completion 线程，确认 GPU fence 已完成；
3. completion 完成后才 `vkResetCommandPool` / `vkResetFences`，重新录制与提交；
4. `PooledCompletion` 只关闭私有的 completion fd，不销毁或重置共享 VkFence；
5. 最终先 join 所有 completion，再按 Vulkan 规则销毁 buffer/image 与其绑定 memory；
6. 最终 `PASS` 强制要求 `completion_ok == rendered`、`readback_ok == rendered`、
   `completion_fail == 0`、`readback_fail == 0`、`fallback == 0`。

新增 `--pace-ms=` 运行参数，用 Rust `thread::sleep` 控制测试帧率；它只用于长稳门禁，
不代表生产路径限速，也不是性能上限测试。

## 2. 真机结果（FACT）

guest：

```text
vk|ring|BEGIN|buffers=3 frames=10000 64x64 log_every=100 pace_ms=60
vk|pool|SUMMARY|slots=3 image_imports=3 readback_allocations=3
vk|ring|STATS|pending=7106 already_signaled=2894 completion_ok=10000 completion_fail=0 readback_ok=10000 readback_fail=0 fallback=0 stats_ok=true
vk|ring|PASS|rendered=10000/10000 releases=9997 done=true elapsed_s=665.385 fps=15.029 stats_ok=true
vk|pool|teardown|all pooled Vulkan slots destroyed
vk|teardown|PASS|device/instance/loader released
```

app：

```text
ahb|ring|complete|...|completed=10000
ahb|ring|recv|...|text=DONE 10000
ahb|ring|SUMMARY|submitted=10000|completed=10000|guest_done=true
ahb|ring|DONE|ok=true|buffers_released=3
ahb|probe|RING|ok=true
```

门禁计数：

| 检查项 | 结果 |
|---|---:|
| `pool SUMMARY` image imports | 3 |
| `pool SUMMARY` readback allocations | 3 |
| 每帧 `image_create` / `image_import` / `image_bind` 日志 | 0 |
| 真实 `SYNC_FD` | 10,000 / 10,000 |
| export 时仍 `PENDING` | 7,106 |
| export 时已 signal | 2,894 |
| completion | 10,000 / 10,000 |
| GPU 9 点回读 | 10,000 / 10,000 |
| eventfd fallback | 0 |
| guest `FAIL` | 0 |
| app `FAIL` | 0 |
| app transaction complete | 10,000 / 10,000 |
| 最终 AHB 回收 | 3 |
| 连续时长 | 665.385 s |

`fps=15.029` 是 60 ms pacing 的预期值，不是硬件吞吐成绩。此前无 pacing 的 120 帧门禁
记录为 `70.128 fps`，但该数字也不是正式 benchmark。

## 3. fd / RSS 曲线（FACT）

每 60 秒同时采样 app 与 guest 进程：

```text
05:32:07 sample=0 app_fds=145 app_rss_kb=141972 guest_fds=21 guest_rss_kb=26996
05:33:07 sample=1 app_fds=146 app_rss_kb=141404 guest_fds=21 guest_rss_kb=26932
05:34:08 sample=2 app_fds=145 app_rss_kb=141368 guest_fds=21 guest_rss_kb=26932
05:35:08 sample=3 app_fds=146 app_rss_kb=141316 guest_fds=21 guest_rss_kb=26932
05:36:08 sample=4 app_fds=146 app_rss_kb=140680 guest_fds=21 guest_rss_kb=26932
05:37:09 sample=5 app_fds=148 app_rss_kb=140676 guest_fds=21 guest_rss_kb=26932
05:38:09 sample=6 app_fds=145 app_rss_kb=140556 guest_fds=21 guest_rss_kb=26932
05:39:10 sample=7 app_fds=145 app_rss_kb=140404 guest_fds=21 guest_rss_kb=26932
05:40:10 sample=8 app_fds=145 app_rss_kb=140404 guest_fds=21 guest_rss_kb=26932
05:41:10 sample=9 app_fds=145 app_rss_kb=140392 guest_fds=21 guest_rss_kb=26932
05:42:11 sample=10 app_fds=134 app_rss_kb=140372 guest_fds=0 guest_rss_kb=
```

运行期间（sample 0–9）：

- app fd：145–148，无单调增长；结束后 134；
- app RSS：141,972 → 140,392 KiB，无增长；
- guest fd：恒为 21；
- guest RSS：26,996 → 26,932 KiB，无增长；
- guest 进程结束后 fd/RSS 清零。

结论：本次 10 分钟路径没有发现按帧累积的 fd 或 RSS 泄漏。该结论只覆盖本次 3-buffer、
64×64、线性 AHB、单 SurfaceControl 场景。

## 4. 第一次运行失败与排除（FACT）

第一次池化上机时 Activity 在锁屏/息屏状态启动，`am start` 后立刻 `onPause/onStop`；
SurfaceView 未进入前台发布状态，app 没有进入 ring receive，guest 在等待 `RELEASE` 时
15 秒超时。唤醒设备、保持 Activity 前台后，同一 probe 立即通过 120 帧门禁。因此该次
失败是测试环境状态，不是 Vulkan 池化或 SYNC_FD 回归。

## 5. 边界（UNKNOWN / 未完成）

- **不是吞吐 benchmark**：pacing 固定为 60 ms，`fps=15.029` 不能代表最高性能。
- **completion worker 仍是每帧创建并 join**；已证明无泄漏，但线程创建/回收仍是后续可优化项。
- **没有 SurfaceFlinger 内部 trace**：不能声称 SF 对每个 PENDING 帧都显式阻塞等待；
  能确认的是真实 `mali-mali.timeline*` fd 已交给 `ASurfaceTransaction_setBuffer`，
  app transaction 全部完成且有 release-fence 背压。
- **不是完整 WSI**：`VK_KHR_swapchain` / Wayland / X11 仍未接。
- **窗口生命周期未验收**：旋转、DPI、多窗口、SurfaceView / Surface 重建仍待测。
- **单一尺寸与格式**：本次为 64×64、线性、RGBA8；未覆盖分层 AHB、非 4 KiB 对齐尺寸、
  16 KiB 页设备。
- 视频硬解、音频、2.0 proot 替换仍未完成。

## 6. 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu \
-C link-arg=--sysroot=/tmp/gccroot \
-C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
-C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
cargo build --release --target aarch64-unknown-linux-gnu -p vulkan-probe

adb -s UKHMMB8PYTGECMNV shell \
  'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; input keyevent 82'
adb -s UKHMMB8PYTGECMNV push \
  target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-pooled-soak
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk cp /data/local/tmp/vulkan-probe-pooled-soak files/rootfs-full/usr/bin/vulkan-probe-pooled-soak'
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk chmod 755 files/rootfs-full/usr/bin/vulkan-probe-pooled-soak'
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "printf \"ring=3\\nframes=10000\\n\" > files/ahb-bridge"'
adb -s UKHMMB8PYTGECMNV shell \
  'am force-stop com.linuxhub.husk; logcat -c; am start -W -n com.linuxhub.husk/android.app.NativeActivity'

adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "cd /data/user/0/com.linuxhub.husk/files; \
  export VK_ICD_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_syncfd.json \
  VK_DRIVER_FILES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_syncfd.json \
  PANVK_DEBUG=startup PANVK_KBASE_DVFS=none; \
  timeout 900 sh gexec2.sh rootfs-full \
  /data/user/0/com.linuxhub.husk/files/rootfs-full/usr/bin/vulkan-probe-pooled-soak \
  --ahb-socket /data/user/0/com.linuxhub.husk/files/ahb-bridge.sock \
  --log-every=100 --pace-ms=60 2>&1"'
```

测试期间必须保持 Activity 前台并唤醒屏幕；否则 SurfaceView 不会发布 SurfaceControl，
app 不会进入 ring receive。长稳时用独立的 20 秒 `KEYCODE_WAKEUP` 循环防止系统息屏。

## 7. 产物与哈希

```text
probe sha256        : 10a1fd47dc8eab25a3275ea47805146c50156337523938887c2b53e23f012ab7
guest log sha256    : 0ec47b63711e7d2b53eba858f983990d71e38db4b65efeb7da8a70ca77152bc8
app log sha256      : 88c41bcc4676785faa649a0ee384351dc9571b2819039c019be981260b79bd5c
metrics sha256      : 12c1e723d3f744b6ccedff1d9189772b5dc2c42694029d159a32f6923d1f3c73
screenshot sha256   : 434b2ce9db0a84fd3b32c84b5f068938d7d984f60095f7f84084d8644a59167a
driver .so sha256   : e6d1b6fed4b09ff1e84474b160b8898b8adcc4effd01d08c87d129091c838f5e
app APK sha256      : 78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d
```

app log 受 Android logcat 环形缓冲限制，只保留了最后 1,208 帧的逐帧记录；app 自己的
`submitted=10000 / completed=10000` 汇总计数是最终判定依据，完整 guest 事件流在
`59-panvk-pooled-syncfd-soak-guest.log`。

## 8. 相关文件

- `docs/design/evidence/59-panvk-pooled-syncfd-soak-guest.log`
- `docs/design/evidence/59-panvk-pooled-syncfd-soak-app.log`
- `docs/design/evidence/59-panvk-pooled-syncfd-soak-metrics.log`
- `docs/design/evidence/59-panvk-pooled-syncfd-soak.png`
- `docs/design/evidence/59-panvk-pooled-syncfd-soak.sha256`
- `docs/design/acquire-fence.md`
- `runtime/crates/vulkan-probe/src/dmabuf.rs`
