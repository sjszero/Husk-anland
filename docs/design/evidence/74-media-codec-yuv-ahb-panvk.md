# Evidence 74 — 真实 MediaCodec 三平面 YUV AHB 回导 PanVK，三平面 GPU 哈希校验

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通 `untrusted_app`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 组件哈希

```text
2e551f5aa285e6a11f594901434de51af126b5d13ab649c51bdb26fa4965f923  app-debug.apk
3d8b0caf8bf899206d15c2d89c2ef30faeca30a4969872e00beed9c33f834603  libhuskapp.so
8658fcbd56fb56e413eb9771b31ae1a09b8a0529c971565dff02b95749ac895b  vulkan-probe
```

## 结论

**FACT**：guest 在正常 `rootfs-full` 隔离下接收真实 MediaCodec AVC 解码 AHB，PanVK 将
`0x32315659`（`"YV12"` private FourCC，描述值为 `842094169`）导入为
`VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM` 三平面 external image；GPU 分别读出 Y、plane1、
plane2 三个平面后，FNV-1a 哈希与 app 侧 `AHardwareBuffer_lockPlanes` 读取结果全部一致。

**FACT**：本次真实硬解为 `c2.mtk.avc.decoder`，1920x1080@30，150 个 access unit，
150 个输出 image；EOS、stop、delete 均成功。guest 侧 150/150 样本发送完成，`RESULT PASS`。

**FACT**：Vulkan probe 的 socket 连接竞态已修复。现场日志里先出现
`socket_wait|retry=1|... No such file or directory`，随后成功连接，证明 probe 不再依赖
“解码器刚好先创建 socket”的时序碰巧成功。

## 真机关键输出

### app 侧：真实 AHB layout 与三平面哈希

```text
video|bridge|codec|requested=c2.mtk.avc.decoder|actual=c2.mtk.avc.decoder
video|bridge|ahb|n=1|format=842094169|usage=0x103|stride=1920|num_fds=3
video|bridge|ahb-layout|n=1|image_planes=0(rc=0)|lock_planes_rc=0|planes=3
  p0:data=...,row=1920,pixel=1
  p1:data=...,row=960,pixel=1
  p2:data=...,row=960,pixel=1
  plane_fnv=0xd4abebfa3f8cb0fc/0xa8a073543b9ed2db/0xf164ae94471897d1
  plane_bytes=2073600/518400/518400
video|ahb-export|HANDLE|rc=0
video|ahb-export|ACK|byte=0x44
video|ahb-export|DONE
```

`AImage_getNumberOfPlanes` 对这种 `AIMAGE_FORMAT_PRIVATE` 返回 0，这是 Android 的正常行为；
本证据使用的 plane metadata 全部来自 `AHardwareBuffer_lockPlanes`，没有猜测 fd[0] 或猜测布局。

### guest 侧：三平面导入与 GPU 回读

```text
vk|dmabuf|socket_wait|retry=1|/run/husk/video-ahb.sock|No such file or directory
vk|dmabuf|socket|PASS|/run/husk/video-ahb.sock
vk|dmabuf|meta|PASS|1920x1080 stride=1920 format=842094169 usage=0x103 count=1 frames=1
vk|dmabuf|fd_count|3
vk|dmabuf|fd|fd=6 target=anon_inode:gralloc_extra
vk|dmabuf|found|fd=7 target=/dmabuf:G:ImageReader-1920x1080f22u256m
vk|video-ahb|layout|format=0x32315659 dmabuf_size=3137536 planes=3
vk|video-ahb|plane|index=0 offset=0 row_stride=1920 pixel_stride=1
vk|video-ahb|plane|index=1 offset=2592000 row_stride=960 pixel_stride=1
vk|video-ahb|plane|index=2 offset=2073600 row_stride=960 pixel_stride=1
vk|video-ahb|image_create|VkResult=0|format=G8_B8_R8_3PLANE_420_UNORM|1920x1080|modifier=LINEAR|planes=3
vk|video-ahb|image_import|VkResult=0|type=2 size=3110400 fd=7
vk|video-ahb|image_bind|VkResult=0
vk|video-ahb|cpu_luma|PASS|fnv=0xd4abebfa3f8cb0fc|expected=0xd4abebfa3f8cb0fc|bytes=2073600
vk|video-ahb|gpu_luma|PASS|plane=0|fnv=0xd4abebfa3f8cb0fc|expected=0xd4abebfa3f8cb0fc|bytes=2073600|size=1920x1080
vk|video-ahb|gpu_plane1|PASS|plane=1|fnv=0xa8a073543b9ed2db|expected=0xa8a073543b9ed2db|bytes=518400|size=960x540
vk|video-ahb|gpu_plane2|PASS|plane=2|fnv=0xf164ae94471897d1|expected=0xf164ae94471897d1|bytes=518400|size=960x540
vk|video-ahb|display_handoff|PASS
vk|video-ahb|teardown|image/memory/native fds released
```

### 解码端：150 帧完成

```text
guest|OPEN|media=/tmp/husk-video/avc-1080p30.mp4|track=1|size=1920x1080|samples=150
OPEN_READY slots=4 slot_size=2097152 codec=c2.mtk.avc.decoder
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=5112
RESULT PASS
```

## 实现要点

1. app 用 `AHardwareBuffer_lockPlanes` 读取真实三平面 pointer、row stride、pixel stride；
   对每个平面的有效像素区按行计算 FNV-1a，而不是把整个 stride 当成连续内存。
2. app 通过 `AHardwareBuffer_sendHandleToUnixSocket` 发送完整 native handle；guest 扫描
   `/proc/self/fd`，找到真正的 `/dmabuf:` fd，不依赖 fd[0] 顺序。
3. guest 用 `VK_EXT_image_drm_format_modifier` + `DRM_FORMAT_MOD_LINEAR` 创建三平面 image，
   分别用 `VK_IMAGE_ASPECT_PLANE_0/1/2_BIT` 执行 `vkCmdCopyImageToBuffer`。
4. 每个平面都从 host-visible Vulkan buffer 回读并计算 FNV-1a；Y 平面额外做 guest CPU
   mmap 哈希对照。
5. Vulkan probe 的连接改为 90 秒重试，专门覆盖 app 在首帧解码完成后才创建 socket 的时序。

## 边界：这还不是“视频完成”

- 本次是**首帧一次性诊断通道**，不是逐帧 AHB ring handoff。
- 已验证的是 YUV AHB 的导入和三平面 GPU 内容一致性；**还没有**做 YUV→RGB 的 GPU
  sampling/render，也没有把该帧作为真实播放帧走 SurfaceControl 展示。
- 还没有 guest WSI / swapchain / Wayland / X11 合流。
- 还没有 VA-API `husk` 驱动，因此 Chromium / GStreamer 还不能通过标准接口使用本路径。
- 本次只验证 AVC；HEVC/VP9/AV1 的三平面回读未在本证据中验收。
- 还没有 10 分钟播放、丢帧率、硬解 vs 软解 CPU 对比、AV 同步 <40ms。
- 因此只能写：**真实硬解 AHB 三平面已成功进入 guest，并由 PanVK GPU 完整读回校验**；
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

终端 A：先启动 guest Vulkan reader（可早于解码器创建 socket）：

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full
ICD=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cd /data/user/0/com.linuxhub.husk/files
  export VK_ICD_FILENAMES=$ICD VK_DRIVER_FILES=$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  timeout 100 env NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full \
    $R/usr/bin/vulkan-probe-video --video-ahb-socket /run/husk/video-ahb.sock 2>&1'"
```

终端 B：再启动真实 AVC 解码：

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cd /data/user/0/com.linuxhub.husk/files
  HUSK_VIDEO_REALTIME=1 timeout 60 env NR_LD=$LIB/libld.so \
    sh gexec2.sh rootfs-full $R/usr/local/bin/video-bridge \
    /run/husk/video.sock /tmp/husk-video/avc-1080p30.mp4 150 2>&1'"
```

完整日志：[`74-video-ahb-app.log`](74-video-ahb-app.log)、
[`74-video-ahb-guest.log`](74-video-ahb-guest.log)、
[`74-video-ahb-bridge.log`](74-video-ahb-bridge.log)。
