# Evidence 39 — 视频硬解在 Husk 里能走哪条路（2026-11-22）

设备：RMX3357 / MT6893 / Android 13（SDK 33）/ kernel 4.19.191 / 4 KiB 页。
目的：把「guest 里能不能直连硬解」和「app 侧有哪些硬解器」一次钉死，供 GOALS 第 4 条（视频）使用。

## 1. guest 直连硬解：不可行（FACT）

```
$ adb shell 'ls -l /dev/video*'
crw-rw---- 1 media  system u:object_r:video_device:s0  81, 0  /dev/video0
crw-rw---- 1 media  system u:object_r:video_device:s0  81, 1  /dev/video1
crw-rw---- 1 camera system u:object_r:video_device:s0  81, 2  /dev/video2
crw-rw---- 1 camera system u:object_r:video_device:s0  81, 3  /dev/video3
crw-rw---- 1 camera system u:object_r:video_device:s0  81, 4  /dev/video4
```

- 权限是 `0660 media:system`（**不是**旧结论里写的 `root:media`，更正）。
  guest 进程跑在 `untrusted_app` 域/uid，既不是 owner 也不是 group → 打不开，和 `/dev/snd` 同一类。
- 即使能打开也没有意义：MTK 平台没有 V4L2 状态型解码节点，`video0-4` 是相机/ISP 的采集通道。
  编解码器在 MTK 平台走的是 Codec2/OMX（bionic 进程 + binder），不在 `/dev/video*`。
- 因此「guest 里 dlopen 厂商解码 .so」也走不通，理由与 GPU 那条一致：厂商 .so 带 bionic DT_NEEDED，
  glibc guest 内加载不了（evidence 31 已排除同型路线）。

**结论**：硬解只能在 bionic 侧（我们 app 进程自己的 Rust 线程）调 NDK `AMediaCodec`。

## 2. 本机有哪些硬解器（FACT，读 /vendor/etc）

```
$ adb shell 'ls /vendor/etc/media_codecs*.xml'
/vendor/etc/media_codecs.xml  /vendor/etc/media_codecs_c2.xml  /vendor/etc/media_codecs_performance.xml
（另有 media_codecs_google_*.xml / media_codecs_mediatek_audio.xml）

$ adb pull /vendor/etc/media_codecs_c2.xml
$ grep -o '<MediaCodec name="[^"]*"[^>]*type="[^"]*"' media_codecs_c2.xml
```

视频解码器（Codec2，MTK 厂商实现）：

| name | MIME |
|---|---|
| `c2.mtk.avc.decoder` | video/avc (H.264) |
| `c2.mtk.hevc.decoder` | video/hevc (H.265) |
| `c2.mtk.vp9.decoder` | video/x-vnd.on2.vp9 |
| `c2.mtk.av1.decoder` | video/av01 |
| `c2.mtk.mpeg4.decoder` | video/mp4v-es |
| `c2.mtk.h263.decoder` | video/3gpp |
| `c2.mtk.mpeg2.decoder` | video/mpeg2 |
| `c2.mtk.vc1.decoder` | video/x-ms-wmv |
| `c2.mtk.heif.decoder` | image/vnd.android.heic |

同一文件里仍保留 `OMX.MTK.VIDEO.DECODER.*`（AVC/HEVC/VP9/AV1/MPEG2/MPEG4/VC1/H263）以及
Google 软解兜底 `c2.android.{avc,hevc,vp8,vp9,av1}.decoder`。

## 3. 厂商自报性能（FACT 取自 /vendor/etc/media_codecs_performance.xml；**未实测**）

| 解码器 | 1920x1080 实测帧率区间（厂商表） |
|---|---|
| `c2.mtk.avc.decoder` | 113 – 248 fps |
| `c2.mtk.hevc.decoder` | 125 – 276 fps |
| `c2.mtk.vp9.decoder` | 114 – 251 fps |

INFERENCE：1080p30 硬解在本机容量上绰绰有余（余量 ≥3.7x，取区间下界），
真正的不确定性不在解码能力，而在「压缩帧怎么从 guest 递进去、解码帧怎么零拷贝递出来」这两段。

## 4. 由此定下的实现形状（写进 GOALS 第 4 条）

```
guest (glibc)                                   app 进程 (bionic, 同一 uid)
 ffmpeg/GStreamer                                  ┌─ Rust 视频桥线程 ────────────┐
  ├ demux  + 音视频同步（保留在 guest）             │ AMediaCodec (libmediandk)    │
  └ 压缩帧 ── AF_UNIX socketpair + 共享内存 ring ──▶│ → 硬解 → AHardwareBuffer    │
                                                   └── dma-buf（或共享内存）─────┘
                                                          │
                                       MEM_IMPORT 0xc0188016（第 2 条 DRM 语义层）
                                                          ▼
                                              guest EGL / B3 的零拷贝 Surface 路径
```

- 通道独立：不复用输入/剪贴板/音频的通道，单独一条 socketpair + 一个共享内存环。
- 不做独立进程：解码线程活在我们 app 进程里；不留 `virgl`/`gst` 那类中转服务进程。
- 硬解器用 NDK `AMediaCodec`（`dlopen libmediandk.so`），纯 Rust 调用，不写 C、不写 Java。

## 5. 还没测的（UNKNOWN，别当已知）

- `AMediaCodec` 输出 AHardwareBuffer 能否被 `AHardwareBuffer_getNativeHandle` 拿到 dma-buf fd，
  以及该 fd 能否过 `MEM_IMPORT`（取决于 display 条目里 kbase BO 导出能力的结论）。
- guest 侧 Chromium（自带 ffmpeg，走 GStreamer 或自绘）能否在无 VA-API/V4L2 硬解的情况下
  接上这条桥；退路是 Chromium 走软解、独立播放器走硬解。
- 软解 vs 硬解的 CPU 数字（要上机，写进验收）。

## 6. 复现命令

```sh
adb -s UKHMMB8PYTGECMNV pull /vendor/etc/media_codecs_c2.xml /tmp/mc2.xml
adb -s UKHMMB8PYTGECMNV pull /vendor/etc/media_codecs_performance.xml /tmp/mcperf.xml
grep -o '<MediaCodec name="[^"]*"[^>]*type="[^"]*"' /tmp/mc2.xml
grep -A6 'c2.mtk.avc.decoder' /tmp/mcperf.xml
```
