# 视频这块的前人工作：termux-va / droidspaces / hybris / droidmedia

日期：2026-11-23（本轮按用户要求先查现成方案，再动手）

## 一句话

**我们要做的"VA-API → Android MediaCodec 桥"已经有人完整做出来了，而且思路和我们一致：
自己合成 CSD，因为 VA 前端根本不给 SPS/PPS。** 差别在流水线深度和客户端选择。

## 1. `lfdevs/termux-va`（GPL-3.0，20★，2026-11-12 仍在更新）——最直接的对照

- 上游血统：`Re-s/droidspaces-media-decode`（Apache-2.0）→ termux-va 扩展了编码与 profile。
- 结构：**Android 侧一个 daemon**（`daemon/termux-va.c`，122 KB，C）+ **容器侧 Mesa VA 驱动**
  （在 `lfdevs/mesa-for-android-container` 的 `dev/adreno-main` 分支，
  `src/gallium/frontends/va/tva_bridge.c` 203 KB）。两端用一条 Unix socket，
  协议 v3（`HELLO_MAGIC 0x444D4400`，大端）。
- 能力：AVC / HEVC / VP9 / VP8 / AV1 解码，AVC / HEVC / VP9 **编码**；Android 10+；免 root；
  PRoot / chroot / LXC / Droidspaces 都支持；FFmpeg / Chromium / Firefox 不改不重编。

### 1.1 他们同样在"合成 CSD"（对我们最重要的一条）

`tva_bridge.c:1990` 原文（注释）：

> FFmpeg's VAAPI H.264 path does not deliver SPS/PPS as slice-data buffers; the VA frontend
> exposes only the parsed pipe_h264_sps/pps structures. The daemon's MediaCodec needs the
> parameter sets as CSD, so the bridge regenerates them from those parsed structures.
> (Same role as upstream vaapi-driver's h264_bitstream.c)

也就是说：**我们这几天争论的"合成 CSD 对不对"在最强的前人实现里就是标准做法**，
他们也是从结构体字段重建 SPS/PPS，不是去等真实 NAL。我们已经修好的
`runtime/crates/va-driver/src/h264.rs` 和它是同一套思路，字段级也几乎一一对应
（profile 来自 context、level 优先用结构体里的再退回推算、`max_num_ref_frames` 取
picture param 的真值、裁剪按可见尺寸算）。

**可直接借的部分**：`tva_bridge.c:2253` 起是 **HEVC（VPS/SPS/PPS）合成**的完整实现
（含 `profile_tier_level`、scaling list、short-term RPS、VUI），我们缺 HEVC，这段就是蓝本。
`picture_vp9.c` / `av1_bitstream.c` 是 VP9 / AV1 的对应参考。

我们目前**没有**照抄的两处细节：
1. 裁剪（crop）：他们能拿到"应用要求的可见尺寸"，我们是 coded 尺寸、只能声明 1088。
2. level：他们优先用结构体里的 `level_idc`；我们按 `MaxFS` 反推（结论相同）。

### 1.2 流水线深度：他们 6 深，我们 1 深——这就是 18 fps 的来源

- 桥里 `tva_pipeline_depth_default = 6`（上限 `DMD_PIPELINE_DEPTH_MAX`），
  `tva_pend_reserve_locked()` 在 `pend_count >= pipeline_depth` 之前**不等帧**，
  一直往前提交；帧由**独立的 reader 线程**收、应用线程只做拷贝。
- 我们的取帧模型是"提交一帧 → 立刻要这一帧的像素"，所以每帧都吃满一次解码时延
  （实测提交侧 33 fps、取帧 18 fps，见 evidence 87）。
- **但深度不是我们单方面能加的**：VA 客户端必须愿意先提交后取帧。实测 gst-vaapi 是
  一帧一结的（在 `vaapih264dec` 后面插 `queue max-size-buffers=8` 想让它流水，
  结果 200 秒一帧都没出，直接挂死——surface 池被 queue 吃光，死锁）。FFmpeg 的
  vaapi 解码器才是流水的（它用 6+ 的 frame pool）。**结论：要跑满 1080p30，
  验收客户端应该用 FFmpeg 系（mpv/VLC/ffplay/GNOME 的 ffmpeg 后端），不是 gst-vaapi。**

### 1.3 兼容性事实（直接影响我们的验收口径）

`doc/compatibility_zh.md`：

| 应用 | 骁龙 | 非骁龙 |
|---|---|---|
| FFmpeg | ✅ | ✅ |
| Chromium | ✅ | **❌** |
| Firefox | ✅ | **❌** |
| OBS / VLC（打补丁版） | ✅ | **❌** |

我们是 MT6939/天玑 —— **属于"非骁龙"**。也就是说：**"Chromium 硬解在 MTK 上跑不起来"是
这个领域已知的上游限制**，不是我们实现得不好。所以：
- 不要把"Chromium `<video>` 硬解"当成 MTK 上的必达验收项（可以试，失败要写成"平台已知限制"）；
- 视频类的验收客户端改成 FFmpeg 系 + 我们自己的 AHB 上屏路径（evidence 79/80 已经 30 fps）。

### 1.4 其它可以直接抄的协议决定

- **PTS 用提交序号**：`presentationTimeUs = unit_index * 1000`，随帧回传，
  "消费者据此配对 surface，完全无需知道解码器的输出顺序"——和我们 `FRAME pts=N` 同构。
- **可逆排空**：`长度 0` → 送 EOS、等 EOS、`AMediaCodec_flush`、重送 CSD，会话不重连。
- **编码会话**：握手追加 `bitrate / fps_num / fps_den`（分数帧率不取整），上行是紧凑 NV12，
  下行 `[len][flags][unit_index][AU]`；输入缓冲要按解码器报的 stride/slice 填。
  显式 profile 用新 codec ID（8/9/10 = AVC CBP/Main/High），旧 ID 5 = 设备默认。
- **平面式 YUV 要转 NV12**（AV1 的 `color-format=0x13`），线路上永远是线性 NV12。
- **几何走哨兵帧头**：`[0][caps][0xFFFFFFFF]` 后跟 buffer 宽高、stride、slice_height、crop 矩形。
- **不要 bind mount socket 文件，要 mount 目录**（daemon 启动会 unlink 重建，inode 变），
  他们为此在握手响应里加了 endpoint 的 `dev/ino` 供客户端对账。

## 2. `libhybris` / `droidmedia`（另一个方向的对照）

- `libhybris/libhybris`（866★）：在 glibc 进程里加载 bionic 的 HAL/EGL/gralloc。
  Droidspaces-Mali 就是靠它在容器里跑 Mali。**它解决的是我们笔记里那句
  "厂商 libGLES_mali.so 带 25 个 bionic DT_NEEDED，不能在 glibc guest 内加载"——**
  这件事不是不能做，是要一个 bionic 链接器垫片。
- `sailfishos/droidmedia`（19★，Droidian 也有 port）：在 glibc 上包出 MediaCodec /
  Camera / AudioRecorder 的 C 接口。它是"glibc 侧用 Android 硬件"的完整样板，
  连**相机和录音**的用法都在里面。
- 对我们的取舍：**我们本身就跑在 Android app 里，能直接调 NDK（AMediaCodec、
  ACameraManager、AAudio），不需要 hybris**；所以这两者对我们不是"拿来用"，
  而是"接口形状参考"——尤其是相机（Camera2 NDK + 一个预览回调 → 缓冲队列）
  和录音（AAudio input + 环形缓冲）该怎么设计。

## 3. 因此本轮之后的计划调整（已写进 docs/STATUS.md）

1. 视频解码的验收客户端：**FFmpeg 系优先**（gst-vaapi 保留为"能解码"的证据，不再当吞吐基准）。
2. HEVC：按 `tva_bridge.c` 的 VPS/SPS/PPS 合成移植到 Rust（`h264.rs` 旁边新增 `hevc.rs`）。
3. 编码：照 1.4 的握手/帧格式设计我们自己的编码通道（`c2.mtk.avc.encoder` 已实测存在）。
4. Chromium 硬解：标为"MTK 上已知失败"，不作为必达项；不为此改 Chromium。
5. 30 fps：要么走 FFmpeg 的流水线，要么走 evidence 79/80 的 AHB 上屏路径；
   VA 系统层 1 深取帧不再指望跑满。
