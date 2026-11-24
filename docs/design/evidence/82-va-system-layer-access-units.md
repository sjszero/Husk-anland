# Evidence 82 — VA-API 系统层 access-unit 采集冒烟

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
运行域：`untrusted_app`（`com.linuxhub.husk`）  
rootfs：`/data/data/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT — 标准 GStreamer 管线已经能通过 Husk 的 Rust VA-API 驱动进入数据面。**
`vaapih264dec` 在设备上连续采集到 **150 个 H.264 access unit**。首个 access unit：

```text
HUSK_VA_ACCESS_UNIT context=1 surface=1 slices=12 bytes=94924
buffers=4:3128,5:5133,...,4:3128,5:665
prefix=000000016588843a0c6001e202215c215554e857b00c4e00569b06631b6a1010
```

其中 `4` 是 `VASliceParameterBufferType`，`5` 是 `VASliceDataBufferType`；驱动把同一帧的
12 个 slice 按顺序拼成了一个 access unit。后续帧在 5 个 surface 之间轮转，说明
`vaCreateSurfaces`、`vaCreateContext`、`vaCreateBuffer`、`vaRenderPicture` 和 surface
生命周期已经进入实际的 GStreamer 路径，而不是只被 `vainfo` 查询。

**FACT — 这次没有完成解码。** `vaEndPicture` 当前明确返回
`VA_STATUS_ERROR_UNIMPLEMENTED`；app 侧 `AMediaCodec` 传输和 decoded surface / dma-buf
导出还没有接上。因此不能把本证据写成 H.264 播放、Chromium `<video>` 或硬解完成。
`gst-launch` 的进程返回码为 0 只表示本次测试进程退出，不等于有画面输出。

## 系统层含义

**JUDGMENT — 应用侧不需要逐个适配。** Chromium、GStreamer、Totem 继续使用标准 VA-API；
Husk 在 guest 系统层提供 `husk_drv_video.so`。芯片差异只在 VA-API 后面的 adapter / bridge
处理，不能把这个结果误写成“所有 SoC 完全不用适配驱动”。

## 复现命令

构建：

```sh
cd $REPO/runtime
NDK=$NDK
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p husk-va-driver -p va-node-shim \
  --target aarch64-unknown-linux-gnu
```

设备侧（`LIB` 为 `files/libdir` 内容）：

```sh
R=/data/data/com.linuxhub.husk/files/rootfs-full
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files && \
   timeout 20 env HUSK_LD_PRELOAD=$R/usr/lib/aarch64-linux-gnu/libva_node_shim.so \
   LIBVA_DRIVER_NAME=husk LIBVA_DRIVERS_PATH=$R/usr/lib/aarch64-linux-gnu/dri \
   GST_PLUGIN_PATH=/usr/lib/aarch64-linux-gnu/gstreamer-1.0 \
   GST_REGISTRY=/tmp/husk-gst-registry-smoke.bin GST_REGISTRY_FORK=no \
   GST_VAAPI_DRM_DEVICE=/dev/mali0 GST_VAAPI_ALL_DRIVERS=1 \
   HUSK_VA_TRACE=1 GST_DEBUG=0 NR_LD=$LIB/libld.so \
   sh gexec-va.sh rootfs-full $R/usr/bin/gst-launch-1.0 -q \
   filesrc location=$R/tmp/husk-video/avc-1080p30.mp4 ! qtdemux ! h264parse ! \
   vaapih264dec ! fakesink 2>&1; echo HOST_RC=\$?'"
```

## 证据文件

- [`82-va-system-layer-smoke.log`](82-va-system-layer-smoke.log)：157 行原始输出，含 150 条
  `HUSK_VA_ACCESS_UNIT`；不是裁剪后的成功样本。
- `libhusk_va_driver.so` sha256：`a411c4218711c395389c7d6305422b1b496d6f702e4b5325e983f0a30cb92221`
- `va-probe` sha256：`5a9ca9b2568feb4a3135f8f34c35a56485400d8fd2b641b831d3ff614fff5a46`
- `libva_node_shim.so` sha256：`5be483d4f9adbc139cfb24d3a23d89c0fe78256c6d4ab8845d56be24c6eca409`

## 下一步

1. 在 VA `vaEndPicture` 中把 access unit、profile、dimensions、PTS/flags 和 CSD 送入现有
   `video-protocol` / `video-bridge`，复用 app 内唯一的 `AMediaCodec` 解码线程。
2. 让 app 输出 AHardwareBuffer 通过既有 `MEM_IMPORT` / dma-buf 路径回到 guest，再把
   `VASurfaceID` 关联到该 buffer；先允许 CPU 拷贝退化，但必须记录开销。
3. 接 `vaSyncSurface`、`vaQuerySurfaceStatus`、`vaExportSurfaceHandle`，用真实
   `vainfo` + GStreamer + Chromium 验证，不以本证据的 access-unit 采集代替解码验收。
