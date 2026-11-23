# Evidence 85 — 标准 GStreamer 经 Husk VA 驱动拿到真实解码帧

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
域：`untrusted_app`（`com.linuxhub.husk`），rootfs `files/rootfs-full`
解码器：`c2.mtk.avc.decoder`（真实 MTK 硬解）

## 结论

**FACT：`vaapih264dec ! videoconvert ! video/x-raw,format=I420 ! fakesink num-buffers=10`
在这台机器上跑通，取到 11 帧真实解码像素，`HOST_RC=0`，gst-vaapi 不再报
`failed to make image current`。**

每帧事实（trace 原样）：

```text
HUSK_VA_FRAME surface=1 pts=1 size=1920x1080 bytes=3110400 luma_mean=125 min=32 max=211
HUSK_VA_FRAME surface=2 pts=2 size=1920x1080 bytes=3110400 luma_mean=125 min=32 max=211
HUSK_VA_FRAME surface=3 pts=3 size=1920x1080 bytes=3110400 luma_mean=125 min=32 max=210
HUSK_VA_FRAME surface=4 pts=4 size=1920x1080 bytes=3110400 luma_mean=125 min=32 max=210
```

`luma_mean=125 / min=32 / max=211` 是抽样亮度统计（每 997 字节采一个），用来区分
"拿到帧"和"拿到一张黑帧"。黑帧会是 `mean=16 max=16`（limited-range 黑电平），这里不是。

## 这轮补上的四件事

1. **VA image 全链路**：`vaCreateImage` / `vaDeriveImage` / `vaDestroyImage` / `vaGetImage`
   都实现了，image 就是一个装 NV12 的普通 buffer，所以 `vaMapBuffer` 不需要第二条路径。
   gst-vaapi 的输出池正是按 VA image 建的（evidence 84 的 `Cannot create VA image` 已消失）。
2. **异步解码**：不再假设"一次输入一次输出"。`SAMPLE` 只回 ACK，解码帧进 app 侧有界队列
   （8 帧），guest 用 `FRAME pts=N` 按 PTS 领取。首个 POC 实测前 4 帧才凑够参考帧。
3. **几何按解码器自己报的来**：MTK 的 ByteBuffer 输出是 **1920x1080 / stride 1920**，
   不是 VA surface 的 1920x1088。之前沿用 surface 高度，直接踩出
   `I420 output too small: cap=3110400 need=3133440`，一帧都出不来。
4. **`vaGetImage` 接受编码尺寸**：gst 要的是 `1920x1088+0+0`（编码高度），帧只有 1080 行。
   现在的实现按行拷贝并用 0 填充剩余行，返回 SUCCESS；之前一律拒绝，导致
   `map_vaapi_memory: failed to make image current`。

## 一个必须写清楚的"假成功"

只跑 `vaapih264dec ! fakesink`（不接 `videoconvert`）同样会 `HOST_RC=0`，但
**一帧都没被 map**：gst-vaapi 只是把 surface 打包成 buffer 传下去，fakesink 不解引用。
所以那次 `HOST_RC=0` 不能当解码证据。本证据用的是强制下载路径
（`! videoconvert ! video/x-raw,format=I420`），它必须真的读像素。

## 还没解决：合成 CSD 是错的（FACT）

不带 override 跑同一条管线：150 个 access unit 全部提交成功、`HOST_RC=0`，但解码器
**一张图都没出**（连 `INFO_OUTPUT_FORMAT_CHANGED` 都没来，`cpu-format` 计数 0）。

把两边的 SPS 拿出来逐字段解析（自写 bit 解析器）：

| 字段 | 真实 SPS | husk 合成 SPS |
|---|---|---|
| profile_idc | 66 (Baseline) | 66 |
| constraint flags | 0xc0 | 0x00 |
| level_idc | 40 (4.0) | 51 (5.1) |
| pic_order_cnt_type | **2** | **0** |
| max_num_ref_frames | 1 | 1 |
| width_in_mbs_minus1 | **119** | **0** |
| height_in_map_units_minus1 | **67** | **0** |
| frame_mbs_only_flag | **1** | **0** |
| frame_cropping | bottom=4 | 无 |
| VUI | 有 | 有 |

`width/height = 0` 和 `frame_mbs_only=0` 说明 `h264.rs` 读 `VAPictureParameterBufferH264`
的偏移/位域假设和这台系统上 libva 真正交给驱动的布局对不上：读到的是一片 0。
一个描述成 16x16、场编码的 SPS 配上真实的 1080p 码流，解码器不报错、也不出图——
这正是观察到的行为。**这不是"解码器不行"，是我们的 SPS 写错了。**

为了让传输层先能被验证，驱动加了两个测试钩子：`HUSK_VA_SPS` / `HUSK_VA_PPS`
（Annex-B hex）。本证据的成功运行用的是从 mp4 `avcC` 取出的真实 SPS/PPS：

```text
sps=6742c028da01e0089f970110000003001000000303c8f1832a
pps=68ce0fc8
```

下一步（未做）：把 `h264.rs` 的偏移改成运行时实测/按 libva 结构体用
`core::mem::offset_of!` 推导，并把 level 从**可见**尺寸算（1920x1080 应得 40，不是 51）。
`HUSK_VA_DUMP=1` 会把 picture parameter buffer 落到 guest 的 `/tmp/husk-pp.bin`，
下一轮先拿它对齐偏移，再删掉合成路径的猜测。

## 复现

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p husk-va-driver --target aarch64-unknown-linux-gnu
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "cat > files/rootfs-full/usr/lib/aarch64-linux-gnu/dri/husk_drv_video.so"' \
  < target/aarch64-unknown-linux-gnu/release/libhusk_va_driver.so
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk touch files/video-cpu'   # CPU 帧回传模式
adb -s UKHMMB8PYTGECMNV shell 'am start -n com.linuxhub.husk/android.app.NativeActivity'  # 必须在前台，否则进程被冻结
```

guest 侧（`LIB` 为 `files/libdir`）：

```sh
R=/data/data/com.linuxhub.husk/files/rootfs-full
run-as com.linuxhub.husk sh -c "cd /data/data/com.linuxhub.husk/files && \
  HUSK_VA_SPS=6742c028da01e0089f970110000003001000000303c8f1832a HUSK_VA_PPS=68ce0fc8 \
  HUSK_LD_PRELOAD=$R/usr/lib/aarch64-linux-gnu/libva_node_shim.so \
  LIBVA_DRIVER_NAME=husk LIBVA_DRIVERS_PATH=$R/usr/lib/aarch64-linux-gnu/dri \
  GST_PLUGIN_PATH=/usr/lib/aarch64-linux-gnu/gstreamer-1.0 GST_REGISTRY=/tmp/husk-gst-registry-smoke.bin \
  GST_REGISTRY_FORK=no GST_VAAPI_DRM_DEVICE=/dev/mali0 GST_VAAPI_ALL_DRIVERS=1 \
  HUSK_VA_TRACE=1 NR_LD=$LIB/libld.so sh gexec-va.sh rootfs-full $R/usr/bin/gst-launch-1.0 -q \
  filesrc location=$R/tmp/husk-video/avc-1080p30.mp4 ! qtdemux ! h264parse ! vaapih264dec ! \
  videoconvert ! video/x-raw,format=I420 ! fakesink num-buffers=10; echo RC=\$?"
```

## 证据文件

- `85-va-image-decode-real-csd.log` — 成功那次完整输出（含 11 条 `HUSK_VA_FRAME`）。
- `85-va-image-decode-synth-csd.log` — 合成 CSD 那次，证明"提交成功但零输出"。

## 不要写成已完成的部分

- 合成 CSD 仍然坏，去掉 override 就没有帧。
- 只测了 H.264 Baseline 1080p 的一个 10 帧片段；没有 10 分钟长稳、没有 HEVC/VP9/AV1、
  没有 Chromium `<video>`、没有 AV 同步、没有硬解 vs 软解 CPU 对比。
- 显示路径没接：本证据止步于 `fakesink`，没有上屏。
