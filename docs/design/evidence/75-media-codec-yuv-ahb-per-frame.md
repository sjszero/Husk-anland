# Evidence 75 — 真实 MediaCodec 三平面 YUV AHB 的逐帧交接（150/150）

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
包：`com.linuxhub.husk`（普通 `untrusted_app`）
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT**：在 evidence 74 的单帧诊断之上，app 与 guest 已完成 **150/150 帧**的真实
`c2.mtk.avc.decoder` 三平面 YUV AHB 交接。每帧都重新建立一次 listener/连接，guest 在
下一帧产生前完成 PanVK 导入、Y/plane1/plane2 三个 plane 的 GPU 回读哈希校验、ACK 和资源
释放。

**FACT**：app 侧计数 `BEGIN=150`、`HANDLE rc=0=150`、`ACK=150`、`DONE=150`、`FAIL=0`；
guest 侧 `gpu_luma PASS=150`、`gpu_plane1 PASS=150`、`gpu_plane2 PASS=150`，
`vk|video-ahb|SUMMARY|PASS|completed=150/150`。

**FACT**：这是一条逐帧交接诊断路径，不是最终持久 AHB ring，也不是已经完成的视频播放
链路。每帧仍然建立一次 socket，创建/导入/销毁一次 Vulkan image 和 memory；YUV→RGB、
guest WSI/swapchain、VA-API、GStreamer/Chromium 和 AV 同步仍未完成。

## 组件哈希

```text
a18df07fcbdac20508472e0768030b79cbab64802cdc014293a8ed6b60049d75  app-debug.apk
f13fc4d632ad7e10b644942606451f0d32cabe6bad79dbb4de41d4477d8944e7  libhuskapp.so
834dc347a4a07a37a461428195a5d4cd9d02691629d0e7bc2aae030c46058587  vulkan-probe
```

## 真机关键输出

### app 侧：每帧导出

以下为日志计数；完整原始日志见 [`75-video-ahb-app.log`](75-video-ahb-app.log)。

```text
ahb-export|BEGIN      150
ahb-export|HANDLE rc=0 150
ahb-export|ACK byte=0x44 150
ahb-export|DONE       150
FAIL                  0
```

最后一帧的 app 侧 layout：

```text
video|bridge|ahb-layout|n=150|image_planes=0(rc=0)[]|lock_planes_rc=0|planes=3
  p0:data=...,row=1920,pixel=1
  p1:data=...,row=960,pixel=1
  p2:data=...,row=960,pixel=1
  plane_fnv=0x2de7f38d259a6e33/0x5d1b68abc3eda0d0/0xc3b0fee494cf5024
  plane_bytes=2073600/518400/518400|unlock_rc=0|num_fds=3
```

### guest 侧：每帧三平面 GPU 回读

以下为日志计数；完整日志见 [`75-video-ahb-guest.log`](75-video-ahb-guest.log)。

```text
vk|video-ahb|frame_summary|PASS       150
vk|video-ahb|gpu_luma|PASS            150
vk|video-ahb|gpu_plane1|PASS          150
vk|video-ahb|gpu_plane2|PASS          150
vk|video-ahb|SUMMARY|PASS|completed=150/150
```

最后一帧：

```text
vk|video-ahb|cpu_luma|PASS|fnv=0x2de7f38d259a6e33|expected=0x2de7f38d259a6e33
vk|video-ahb|gpu_luma|PASS|plane=0|fnv=0x2de7f38d259a6e33|expected=0x2de7f38d259a6e33
vk|video-ahb|gpu_plane1|PASS|plane=1|fnv=0x5d1b68abc3eda0d0|expected=0x5d1b68abc3eda0d0
vk|video-ahb|gpu_plane2|PASS|plane=2|fnv=0xc3b0fee494cf5024|expected=0xc3b0fee494cf5024
vk|video-ahb|teardown|image/memory/native fds released
vk|video-ahb|SUMMARY|PASS|completed=150/150
```

### 解码端：150 个 AVC access unit

```text
OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=25944
RESULT PASS
```

完整解码端日志见 [`75-video-ahb-bridge.log`](75-video-ahb-bridge.log)。

## 实现要点

- app 侧 `export_if_enabled()` 不再受一次性 `EXPORTED` 门闩限制；marker
  `files/video-ahb` 存在时每帧都执行一次导出。
- 每帧 listener 只接受一个 guest 连接，发送完整 AHB native handle，等待 guest 完成
  三平面 GPU 校验后返回单字节 ACK，再结束该帧。
- guest 侧新增 `--video-ahb-frames N` 与 `run_video_many()`；每帧重新连接、导入 AHB、
  回读三平面，close 全部 native-handle fd，然后进入下一帧。
- app 侧每帧用 `AHardwareBuffer_lockPlanes` 生成 layout 和三平面 FNV-1a；guest 侧在
  Vulkan image 上分别执行 plane 0/1/2 的 copy-to-buffer 回读并计算同一哈希。
- 测试使用 `HUSK_VIDEO_REALTIME=0`，所以 app 日志里的 `video|bridge|present|ok=false`
  是探针模式的预期结果，不代表真实渲染失败。

## 边界：这还不是“视频完成”

- 本次是**一帧一连接的诊断路径**，不是持久多 slot AHB ring；每帧仍创建、导入、绑定和
  销毁 Vulkan image/memory。下一次迭代要改为 slot 复用、release fence 背压和持久 socket。
- 已验证的是 YUV AHB 的导入和三平面 GPU 内容一致性；**还没有**做 YUV→RGB 的 GPU
  sampling/render，也没有把该帧作为真实播放帧走 guest WSI 展示。
- 还没有 guest WSI / swapchain / Wayland / X11 合流。
- 还没有 VA-API `husk` 驱动，因此 Chromium / GStreamer 还不能通过标准接口使用本路径。
- 本证据只验证 AVC；HEVC/VP9/AV1 的逐帧 AHB 路径未验收。
- 还没有 10 分钟播放、丢帧率、硬解 vs 软解 CPU 对比、AV 同步 <40ms。
- 因此只能写：**真实硬解 AHB 已完成 150 帧逐帧交接和三平面 GPU 校验**；
  不能写：视频播放已完成。

## 复现命令

构建：

```sh
cd $REPO
NDK=$NDK

cd app-rs
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android
cp target/aarch64-linux-android/release/libhuskapp.so \
  ../app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so

cd ../app
./gradlew :app:assembleDebug

cd ../runtime
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu
```

部署并启动：

```sh
cd $REPO

adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe /data/local/tmp/vulkan-probe-video
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "
  cat /data/local/tmp/vulkan-probe-video > files/rootfs-full/usr/bin/vulkan-probe-video
  chmod 755 files/rootfs-full/usr/bin/vulkan-probe-video
  rm -f files/rootfs-full/run/husk/video.sock files/rootfs-full/run/husk/video-ahb.sock
  touch files/video-bridge files/video-ahb
"'
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start -n com.linuxhub.husk/android.app.NativeActivity
```

终端 A：先启动 guest Vulkan reader。

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full
ICD=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cd /data/user/0/com.linuxhub.husk/files
  export VK_ICD_FILENAMES=$ICD VK_DRIVER_FILES=$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  timeout 240 env NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full \
    $R/usr/bin/vulkan-probe-video --video-ahb-socket /run/husk/video-ahb.sock \
    --video-ahb-frames 150 2>&1'"
```

终端 B：再启动真实 AVC 解码。

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cd /data/user/0/com.linuxhub.husk/files
  HUSK_VIDEO_REALTIME=0 timeout 240 env NR_LD=$LIB/libld.so \
    sh gexec2.sh rootfs-full $R/usr/local/bin/video-bridge \
    /run/husk/video.sock /tmp/husk-video/avc-1080p30.mp4 150 2>&1'"
```

## 哈希

```text
1553980bbc2ba0aca42dfb1aa25bcf11d962dd6316d5bc4f78ed3275e48f249a  75-video-ahb-app.log
a148345ae3470cacb2b205328d102044bb4354189a7c15154a60106c6752c888  75-video-ahb-guest.log
fa03f0c00985d5833a554a7bea60355df09bd4473444728e0fe9cf0fa19108d0  75-video-ahb-bridge.log
```
