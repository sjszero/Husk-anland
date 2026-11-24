# Evidence 86 — 合成 CSD 修好：不带任何 override 也能解出真实帧

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
域：`untrusted_app`（`com.linuxhub.husk`）／rootfs `files/rootfs-full`
解码器：`c2.mtk.avc.decoder`

## 结论

**FACT：`h264.rs` 的偏移假设是对的，错的是"什么时候读"。修掉取值时序后，
`vaapih264dec ! videoconvert ! video/x-raw,format=I420 ! fakesink num-buffers=10`
在不设置 `HUSK_VA_SPS` / `HUSK_VA_PPS` 的前提下取到 11 帧真实像素，`HOST_RC=0`。**

```text
HUSK_VA_CSD sps=000000016742c028da01e0089a01e30654 pps=0000000168ce0fc8
HUSK_VA_FRAME surface=1 pts=1 size=1920x1088 bytes=3133440 luma_mean=125 min=32 max=211
...
HUSK_VA_FRAME surface=1 pts=11 size=1920x1088 bytes=3133440 luma_mean=125 min=32 max=211
HOST_RC=0
```

真实流的 SPS 头是 `67 42 c0 28`，我们现在合成的头**逐字节相同**
（profile 66 / constraint 0xc0 / level 40）。

## 根因：不是偏移错，是读到了一份还没被写、或者已经被清掉的副本

上一轮把 672 字节的 `VAPictureParameterBufferH264` 原样打出来，链条是：

```text
HUSK_VA_BUFFER context=1 type=0 bytes=672 data_null=true          # create 时 data=NULL
HUSK_VA_MAP_BUFFER id=1 type=0 bytes=672                          # 客户端映射后自己写
HUSK_VA_UNMAP id=1 type=0 bytes=672 prefix=010000000000000008...  # 此刻数据是真的
HUSK_VA_PP_AT_END ... hex=0000...                                 # end_picture 里却是全 0
```

`vaEndPicture` 之前取到的是全 0，于是 `build_csd` 读到
`width_in_mbs_minus1=0 / height_in_map_units_minus1=0 / frame_mbs_only=0 / poc_type=0`，
合成出一个"16x16、场编码"的 SPS 配 1080p 码流——MediaCodec 不报错也不出图。

修法：在 `vaUnmapBuffer` 里对 picture-parameter 类型且内容非全 0 的 buffer 取快照
（`ContextState::pp_snapshots`），`vaEndPicture` 优先用快照。快照拿到的真实字段：

```text
width_mbs_minus1=119 (1920)  height_in_map_units_minus1=67 (1088)
bit_depth 0/0  num_ref_frames=1  chroma_format_idc=1
frame_mbs_only_flag=1  pic_order_cnt_type=2  direct_8x8_inference=1
num_slice_groups_minus1=0  pic_fields=0x0500
```

这就是真实码流的参数集，和 evidence 85 从 mp4 `avcC` 里取出来的 SPS 完全一致。

顺带修掉两个合成错误（FACT）：

1. **level 按 `MaxFS` 算**。旧代码用"像素 > 1920x1080 就报 5.1"，而 coded 尺寸是
   1920x**1088**（2088960 像素），于是报 51；真实是 40。现在按宏块上限：
   ≤921600 → 3.1，≤2097152 → 4.0（4.0 的 MaxFS 8192 MB 完全覆盖 1920x1088），
   再上 5.1。
2. **ConstrainedBaseline 补上 `constraint_set0/1`**（0xc0），和真实流一致。

测试钩子 `HUSK_VA_SPS` / `HUSK_VA_PPS` 已删除（`video_client::override_csd` 整段移除），
现在没有"喂真实 CSD"的旁路，成功就是合成 CSD 的成功。

## 已知偏差（FACT，不掩盖）

- 合成 SPS 没有 `frame_cropping`：我们声明可见高度 1088，真实流是 1080（bottom crop 4）。
  解码器按 1088 出图（`size=1920x1088 bytes=3133440`），比真实多 8 行。
  驱动拿不到"可见高度"（`vaCreateConfig/Context` 传进来的就是 coded 尺寸），
  所以现在只能多给 8 行；影响是每帧多 0.7% 的拷贝，不该当成"已完成的高保真路径"。
- 只测了 H.264 Baseline/ConstrainedBaseline 1080p30 的一个 10 帧片段。

## 复现

```sh
cd $REPO/runtime
NDK=$NDK
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p husk-va-driver --target aarch64-unknown-linux-gnu
cd ..
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "cat > files/rootfs-full/usr/lib/aarch64-linux-gnu/dri/husk_drv_video.so"' \
  < runtime/target/aarch64-unknown-linux-gnu/release/libhusk_va_driver.so
adb -s UKHMMB8PYTGECMNV shell 'am start -n com.linuxhub.husk/android.app.NativeActivity'   # 必须在前台且亮屏
```

guest 侧（`LIB=$(cat files/libdir)`）：

```sh
R=/data/data/com.linuxhub.husk/files/rootfs-full
run-as com.linuxhub.husk sh -c "cd /data/data/com.linuxhub.husk/files && \
  HUSK_LD_PRELOAD=$R/usr/lib/aarch64-linux-gnu/libva_node_shim.so \
  LIBVA_DRIVER_NAME=husk LIBVA_DRIVERS_PATH=$R/usr/lib/aarch64-linux-gnu/dri \
  GST_PLUGIN_PATH=/usr/lib/aarch64-linux-gnu/gstreamer-1.0 GST_REGISTRY=/tmp/husk-gst-registry-smoke.bin \
  GST_REGISTRY_FORK=no GST_VAAPI_DRM_DEVICE=/dev/mali0 GST_VAAPI_ALL_DRIVERS=1 \
  HUSK_VA_TRACE=1 NR_LD=$LIB/libld.so sh gexec-va.sh rootfs-full $R/usr/bin/gst-launch-1.0 -q \
  filesrc location=$R/tmp/husk-video/avc-1080p30.mp4 ! qtdemux ! h264parse ! vaapih264dec ! \
  videoconvert ! video/x-raw,format=I420 ! fakesink num-buffers=10; echo HOST_RC=\$?"
```

## 证据文件

- `86-va-synth-csd-fixed.log` — 本次运行（无 override）的 CSD、AU、11 条 `HUSK_VA_FRAME`、`HOST_RC=0`。
- `86-va-synth-csd-soak.log` — 连续长跑：760 秒墙钟、12,745 帧、零崩溃
  （`fakesink sync=true` + `HUSK_VA_TRACE=1`，trace 开着只有 ≈16.8 fps，所以
  没跑完 600 秒片源就被测试超时收掉；吞吐结论见 evidence 87）。
