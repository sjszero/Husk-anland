# Evidence 55 — eventfd acquire bridge passes the SurfaceControl ring; true Vulkan SYNC_FD remains blocked

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页
时间：2026-11-23 04:32–04:33（CST）
结论：**部分通过（PASS）**。SurfaceFlinger 接受 eventfd 作为 pollable acquire fd；3-buffer ring 连续 120 帧、release fence 背压和资源回收通过。它不是 Vulkan `SYNC_FD`，不能写成“厂商 GPU producer fence 已完成”。

## 1. 这次解决的是什么

evidence 54 证明了 AHB ring、SurfaceControl 上屏和 Android→guest 的 release fence 背压，但 READY 仍把 acquire fence 写成 `-1`。旧实验直接调用 `vkGetFenceFdKHR`：

- FristOneRR PanVK 的 kbase CPU-sync 后端会先 CPU 等待 GPU seqno；
- 导出时 `vkGetFenceFdKHR` 返回成功但 `fd=-1`；
- Vulkan `SYNC_FD` 导出按规范重置原 fence，因此“导出后继续 wait 同一个 fence”会挂起。

这里不再把 `fd=-1` 当成可用 fence，也不重复这个失败组合。当前实现改为：

1. guest submit 后由 Rust completion 线程等待 `VkFence`；
2. 线程持有 `eventfd` 的写端副本；
3. GPU 完成后写 8 字节到 eventfd；
4. `READY <slot> <frame> <expected>` 与 eventfd 通过同一个 `sendmsg(SCM_RIGHTS)` 发给 app；
5. app 把收到的 fd 交给 `ASurfaceTransaction_setBuffer(..., acquire_fd)`；
6. app→guest 的 `RELEASE <slot>` 仍由 SurfaceFlinger 的 previous-release fence 驱动。

`eventfd` 在这里是 **pollable completion bridge**，不是 Vulkan `SYNC_FD`。它满足当前 SurfaceControl 路径的 poll 语义，但消费者若要求 sync_file 的 `SYNC_IOC_FILE_INFO` / signal-time 查询，仍需后续真 fence。

## 2. FACT：真机通过

app 和 guest 均使用 3 个 slot、120 帧：

```text
app : ahb|ring|SUMMARY|submitted=120|completed=120|guest_done=true
app : ahb|ring|DONE|ok=true|buffers_released=3
guest: vk|ring|PASS|rendered=120/120 releases=117 done=true
guest: vk|teardown|PASS|device/instance/loader released
```

每个 READY 的 app 日志都能看到 `control_len=24|fds=1`，随后成功 present；每个 frame 的 completion 线程都返回 `wait=VkResult=0 write=8`，readback 9/9 匹配。app 的 `on_transaction_complete` 实际收到 release fd（例如 `147/150/151/152/153`），并回发 RELEASE；最终 `completed=120`。

资源回收：

- ring 前 app fd 数：141；
- 等待 ring cleanup 后：129（本次运行没有按帧增长的 fd 泄漏）；
- guest 收到并关闭所有 AHB handle、eventfd 本地副本和 Vulkan 对象；
- 没有出现 `avc: denied ... fifo_file`；eventfd 路径通过 SELinux。

截图：`docs/design/evidence/55-eventfd-acquire-fence.png`，最后一帧为蓝色（GPU clear 后的合法最后一帧），不是黑屏或未合成窗口。

## 3. 现场边界（不能写成什么）

- **FACT**：eventfd fd 被 SurfaceFlinger 接收并完成 transaction；release fence 背压保持。
- **FACT**：这不是 `vkGetFenceFdKHR` 返回的 pending `SYNC_FD`；本次日志中 completion 往往在 app 收到 READY 前已经完成，因此不能把 eventfd 的 signal 时刻当作 GPU 尚未完成的证明。
- **UNKNOWN**：SurfaceFlinger 的部分消费者可能对 eventfd 做 `sync_file` 专用操作；当前实测路径没有触发，不能外推到所有 Android 版本。
- **UNKNOWN / 未完成**：真正无 CPU 等待的 producer fence。仍需要 kbase `STREAM_CREATE` + 正确的 `BASE_JD_REQ_SOFT_FENCE_TRIGGER`，或修 PanVK kbase 后端让 `vkGetFenceFdKHR` 返回真实 dma-fence/sync_file。
- **未验证**：持久资源池、旋转 / DPI / 多窗口、SurfaceView 重建、10 分钟长稳和真实窗口交互。

## 4. 产物哈希

```text
guest probe sha256: 42a1cbd79086a4bb3199cc21383ccf8c8212a43fc9b74487d2adc25c20b304e5
app APK sha256   : 78d97c9360e4f0489377453d8514a728f73bcf0c4564685ab122c8b9b1c2935d
app .so sha256   : 28e2b15023d33d6935ac2d1deccc7f3f4517ceadd0e89440aed6c8a38ac97b0d
screenshot sha256: 9002cd2ed4e5e6abbdc245cb4589c75f27fa15b57f6d55d9bb6a6747e3b8294
```

## 5. 可复现命令（主机侧）

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk

# build guest probe
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu \
  --manifest-path runtime/Cargo.toml

# build + install app
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
   app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
(cd app && ./gradlew :app:assembleDebug -q)
adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk

# install probe into Husk private rootfs
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/husk-vulkan-ring-async
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c \
  "cat /data/local/tmp/husk-vulkan-ring-async > files/vulkan-probe-ring-async && \
   chmod 755 files/vulkan-probe-ring-async && \
   printf \"ring=3\\nframes=120\\n\" > files/ahb-bridge"'

# wake + start app
adb -s UKHMMB8PYTGECMNV shell input keyevent KEYCODE_WAKEUP
adb -s UKHMMB8PYTGECMNV shell wm dismiss-keyguard
adb -s UKHMMB8PYTGECMNV shell am start -n com.linuxhub.husk/android.app.NativeActivity

# run guest probe (after app logs LISTEN and onNativeWindow)
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c '"'"'F=/data/user/0/com.linuxhub.husk/files; \
  R=$F/rootfs-full; ICD=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json; \
  export VK_ICD_FILENAMES=$ICD VK_DRIVER_FILES=$ICD PANVK_DEBUG=startup PANVK_KBASE_DVFS=none; \
  cd $F; timeout 180 sh gexec2.sh rootfs-full $F/vulkan-probe-ring-async \
  --ahb-socket $F/ahb-bridge.sock'"'"''
```

## 6. 关键日志

- `docs/design/evidence/55-eventfd-acquire-fence-app.log`
- `docs/design/evidence/55-eventfd-acquire-fence-guest.log`
- `docs/design/evidence/55-eventfd-acquire-fence.png`

注意：旧 evidence 54 的哈希和产物不被本文件覆盖。
