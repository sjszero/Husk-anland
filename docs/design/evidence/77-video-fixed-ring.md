# Evidence 77 — 固定三槽 AHB ring 的 150 帧门禁

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
运行域：`untrusted_app`（`com.linuxhub.husk`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT**：真实 MTK `c2.mtk.avc.decoder` 的 1920×1080@30 视频，已经通过
**3 个应用预分配 YV12 AHardwareBuffer** 组成的固定 ring 完成 **150/150 帧**交接。
app 每帧把 MediaCodec 的 I420 输出拷贝到固定的 YV12 AHB；guest 只导入 3 个稳定
dma-buf 身份，命中后复用同一组 Vulkan image/memory，不再逐帧创建/销毁。

**FACT**：app 计数为：

```text
video|fixed-ring|READY|slots=3
video|ahb-ring|SUMMARY|PASS|sent=150|completed=150|handles=3|reuses=147
video|bridge|CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 frames=151 output_bytes=466560000 images_last=151 stop_rc=0 delete_rc=0
```

guest 计数为：

```text
vk|video-ahb|ring_summary|slots=3|imports=3|reuses=147
vk|video-ahb|STREAM|CLOSED|completed=150/150
vk|video-ahb|SUMMARY|PASS|completed=150/150
```

guest 侧 `frame_summary`、`gpu_luma`、`gpu_plane1`、`gpu_plane2` 和 `ring_frame`
均为 150/150 PASS；三平面 CPU/GPU FNV-1a 校验一致。解码端
`RESULT PASS`，150 samples / 9,821,765 bytes。

## 组件哈希

```text
eabc2457e25abe119a312e13c5322ce4811d6f3b4690b2370f507a347a698c11  app-debug.apk
022dc136cda1cff2f6dfbdf2888a4263334f96ec75347ef9c5c17bdc22e7a765  libhuskapp.so
5e79c4171e18b7d60350fa579e9d968b87d74fd0cb7b62af270b0e729a0ac6f3  vulkan-probe
```

## 关键实现

- `app-rs/src/video/fixed_ring.rs`：预分配 3 个 `1920×1080` YV12 AHB；从
  MediaCodec I420 ByteBuffer 拷贝时显式交换 U/V 平面；每帧计算 Y/plane1/plane2 FNV-1a。
- `app-rs/src/video/ebridge.rs`：固定 ring 帧没有 `AImage` 需要释放；guest ACK 后只归还 slot。
- `app-rs/src/video.rs`：优先用 `AHardwareBuffer_getId()` 的稳定 AHB id 作为 slot key；
  dma-buf key 仍保留用于交叉校验。
- guest `vulkan-probe`：按稳定 key 首次创建 Vulkan image/memory；后续帧只验证内容和 GPU 回读，
  不重建资源。

## 边界（不能误写成完成）

**FACT**：这仍是 **CPU I420 → YV12** 拷贝。它解决了 ImageReader 每帧新 dma-buf 导致的
import ring 不稳定问题，但不是 decoder→GPU 的零拷贝，也不是 YUV→RGB 渲染。

**仍未完成**：YUV→RGB GPU sampling/render、guest WSI/swapchain、VA-API/GStreamer/Chromium、
HEVC/VP9/AV1 bridge、AV 同步、10 分钟实时播放、丢帧率和硬软解对比、2.0 proot 替换。

## 复现前提

测试脚本必须在 app 已启动并出现下面这行后再运行；否则 guest 会等待
`/run/husk/video-ahb.sock`，这是测试启动顺序问题，不是 ring 本身：

```text
video|bridge|LISTEN|/data/user/0/com.linuxhub.husk/files/rootfs-full/run/husk/video.sock
```

本轮先启动 NativeActivity，等待 `LISTEN`，再分别在两个 adb shell 中启动 guest reader
和 guest video-bridge，最终两边均返回 0。

## 原始证据

- [`77-video-fixed-ring-app.log`](77-video-fixed-ring-app.log) — app 过滤日志（logcat 保留的区间）
- [`77-video-fixed-ring-guest.log`](77-video-fixed-ring-guest.log) — 完整 guest 150 帧日志
- [`77-video-fixed-ring-bridge.log`](77-video-fixed-ring-bridge.log) — guest 解码/控制通道日志
- [`77-video-fixed-ring-result.log`](77-video-fixed-ring-result.log) — app 端会话 result 文件

日志 SHA-256：

```text
1819057c2084f5a4d061d00b5351290527a2ac24aac7a3b5fd5851ae34024c65  77-video-fixed-ring-app.log
d70ea9bc4166e4e133207e036486417fb67c6f4fd5a54a8385dc2ddcee9b1a57  77-video-fixed-ring-guest.log
2dea22aa6d875f036349e30de02ae0a629b845b6fa5642a937be9d969642171b  77-video-fixed-ring-bridge.log
0b66fdc3fdaf23deb20fc828415b61326c185ec3cb303c428bf6f4a3c33907b9  77-video-fixed-ring-result.log
```
