# evidence 54 — 3-buffer AHB ring 连续帧与 release 复用

- 日期：2026-11-23 04:04–04:05 CST
- 设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
- 状态：**PASS（连续 12 帧、3-buffer ring、backpressure、buffer 复用、SF release fence）**
- 产物：
  - APK `ef6d88c866494aa3f12e151c1a515a1a8e3109f08aed2805806473892c36573f`
  - `libhuskapp.so` `e4f9b6eb1be42534b2675bb063513ac3b7849c46459d3318dc9157bf917304fc`
  - guest `vulkan-probe` `d939442c51c77f77845d9cc303d25e5f80063040c68c363130a3b2e499af22ae`
  - 截图 `370cde3fbc9223063354c243139eec32497cc7593d443e446fda64a394347570`
  - app logcat `573ba54c4cdc30edc4c816f93b44ea5b1e38b69ff4dcbf2c44fdd28168109ccf`
  - guest log `70aa6f96f48470bb92cdd0d0d2eee96802de11573b9987db99f4069ccab624e5`

截图：[`54-surfacecontrol-ring.png`](54-surfacecontrol-ring.png)

原始日志：[`54-surfacecontrol-ring-app.log`](54-surfacecontrol-ring-app.log)、
[`54-surfacecontrol-ring-guest.log`](54-surfacecontrol-ring-guest.log)

## 结论

这一轮不是单帧证明。app 一次分配 3 个 AHardwareBuffer，经同一条 `AF_UNIX/SOCK_SEQPACKET`
连接把 3 个 native handle 交给 guest；guest 对每个 dma-buf 建立 LINEAR 2D image，连续渲染
12 帧，并按 slot 做背压：

```text
app allocate AHB[3]
  -> SCM_RIGHTS send x3
  -> guest imports dmabuf x3
  -> GPU clear frame N into AHB[N % 3]
  -> vkWaitForFences: GPU work complete
  -> guest sends READY <slot> <frame>
  -> app ASurfaceTransaction_setBuffer + setOnComplete + apply
  -> SurfaceFlinger presents
  -> app waits ASurfaceTransactionStats_getPreviousReleaseFenceFd
  -> release fence signals
  -> app sends RELEASE <previous_slot>
  -> guest may overwrite that slot again
```

实测结果：

- app：`submitted=12 completed=12 guest_done=true`。
- guest：`rendered=12/12 releases=9 done=true`。
- 3 个 slot 各复用 4 次；guest 每轮重新导入同一 slot 的 dma-buf 并 GPU clear。
- GPU 回读与 CPU mmap 对每帧 9/9 点匹配；颜色按红 / 绿 / 蓝循环，预期像素分别为
  `0xff0000ff` / `0xff00ff00` / `0xffff0000`。
- app 收到的真实 SF release fence 包括 fd `143`、`153`、`150`；回调内 poll 均返回已 signal。
- 最终第 12 帧是蓝色。截图正文按每隔 8 像素采样，`37,800 / 37,800` 个采样点为纯蓝
  `(0, 0, 255)`。

关键 app 日志：

```text
ahb|ring|send|slot=0|rc=0
ahb|ring|send|slot=1|rc=0
ahb|ring|send|slot=2|rc=0
ahb|ring|complete|frame_slot=1|previous=Some(0)|release_fd=143|released=true|completed=2
ahb|ring|release_sent|slot=0
...
ahb|ring|complete|frame_slot=2|previous=Some(1)|release_fd=150|released=true|completed=12
ahb|ring|release_sent|slot=1
ahb|ring|SUMMARY|submitted=12|completed=12|guest_done=true
ahb|ring|DONE|ok=true|buffers_released=3
```

关键 guest 日志：

```text
vk|dmabuf|meta|PASS|64x64 stride=64 format=1 usage=0x322 count=3 frames=12
vk|ring|BEGIN|buffers=3 frames=12 64x64
vk|ring|linear|slot=0 fd=8 ok=true
vk|ring|linear|slot=1 fd=11 ok=true
vk|ring|linear|slot=2 fd=14 ok=true
vk|ring|WAIT_RELEASE|frame=9 slot=0
vk|ring|RELEASE|frame=9 slot=0
vk|ring|READY|frame=11 slot=2 rendered=true sent=true
vk|ring|PASS|rendered=12/12 releases=9 done=true
vk|teardown|PASS|device/instance/loader released
```

## 实现位置

- app ring、SCM_RIGHTS 多 handle、SurfaceControl transaction、on-complete 与 release fence：
  `app-rs/src/ahb.rs`
- guest 多 handle 接收、3-buffer 背压、GPU clear 与像素校验：
  `runtime/crates/vulkan-probe/src/dmabuf.rs`

## 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin

CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
(cd app && ./gradlew :app:assembleDebug -q)

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu \
  --manifest-path runtime/Cargo.toml

adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/husk-vulkan-ring
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cat /data/local/tmp/husk-vulkan-ring > files/vulkan-probe-ring
  chmod 755 files/vulkan-probe-ring
  [ ! -e files/ahb-bridge.sock ] || unlink files/ahb-bridge.sock
  printf \"ring=3\\nframes=12\\n\" > files/ahb-bridge
'"
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV logcat -c
adb -s UKHMMB8PYTGECMNV shell am start \
  -n com.linuxhub.husk/android.app.NativeActivity

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/user/0/com.linuxhub.husk/files
  R=\$F/rootfs-full
  ICD=\$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
  export VK_ICD_FILENAMES=\$ICD VK_DRIVER_FILES=\$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink
  cd \$F
  timeout 120 sh gexec2.sh rootfs-full \$F/vulkan-probe-ring \
    --ahb-socket \$F/ahb-bridge.sock 2>&1
'"
```

截图窗口期：app 在 12 次 complete 后保留 SurfaceControl 3 秒，期间执行 `screencap`。

## 边界（不能升级成“显示完成”）

- **已实测**：连续 12 帧、3 buffer ring、slot 复用、app→guest RELEASE 背压、SF 上一 buffer
  release fence、最终窗口画面。
- **未实测**：producer / acquire fence 仍为 `-1`。guest 在发送 READY 前用 `vkWaitForFences`
  等 GPU 完成，因此这轮证明的是正确性，不是最低延迟或最高吞吐。
- guest 每帧重新创建 Vulkan image / command buffer / fence，未做资源池化；本轮数字不能当性能基准。
- 10 分钟泄漏、旋转 / DPI、SurfaceView 重建、多窗口、GBM / Wayland / X11 与 Vulkan WSI /
  swapchain 均仍未验证。
- 下一步应把 guest GPU fence 导出成可传递的 acquire fence，替换 CPU wait；再做持久资源池和长稳。
