# Evidence 79 — 持久 YUV converter 写回 RGBA AHB 并实际上屏

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
运行域：`untrusted_app`（`com.linuxhub.husk`）
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT**：真实 `c2.mtk.avc.decoder` 的 1920x1080 YV12 三平面 AHB 已经逐帧进入持久
PanVK converter；三个固定输入 slot 各自只创建一次 pipeline/descriptor/command pool，
后续帧只 reset command pool/fence、写 push constants、dispatch 并等待 fence。
转换结果写入同一 slot 对应的 RGBA8 AHB，再由 app 的 SurfaceControl 路径提交给
SurfaceFlinger。150 帧实测结果：

```text
app:   video|ahb-ring|SUMMARY|PASS|sent=150|completed=150|handles=3|reuses=147
app:   150 次 video|ahb-ring|PRESENT
guest: video-ahb|ring_summary|slots=3|imports=3|reuses=147
guest: yuv_conv|SUMMARY slot=0|frames=50|passes=50|failures=0
guest: yuv_conv|SUMMARY slot=1|frames=50|passes=50|failures=0
guest: yuv_conv|SUMMARY slot=2|frames=50|passes=50|failures=0
guest: video-ahb|SUMMARY|PASS|completed=150/150
bridge: EOS_OK frames=151 images=151 output_bytes=466560000
bridge: RESULT PASS
```

截图 [`79-yuv-rgba-present.png`](79-yuv-rgba-present.png) 证明不是“只 ACK 未显示”：
设备画面上可见 AVC 测试源的彩条，SurfaceFlinger 层列表同时存在 `husk-video`。

## 关键实现

- `runtime/crates/vulkan-probe/src/yuv.rs`：每个固定 AHB slot 持有持久 `Converter`；
  input/output dma-buf、pipeline、descriptor set、command pool、fence 均只初始化一次。
- `runtime/crates/vulkan-probe/shaders/yuv_to_rgba.comp`：push constants 支持 output
  stride/offset，按 RGBA8 AHB 的真实 row stride 写入。
- `runtime/crates/vulkan-probe/src/dmabuf.rs`：协议升级为 `HUSK-VIDEO-AHB-3`，一帧携带
  YV12 输入 handle 和 RGBA8 输出 handle；沿用 v2/v3 共有的 `NEW/REUSE` framing。
- `app-rs/src/video/fixed_ring.rs`：每个 YV12 slot 同时分配一个 RGBA8 AHB；复用前等待
  SurfaceControl 不再持有该输出 buffer。
- `app-rs/src/video/ebridge.rs`：同一条持久 socket 发送两个 native handle；guest ACK 后
  立即把输出 AHB 交给 SurfaceControl。
- `app-rs/src/video/present.rs`：用一个 `busy` 集合跟踪 SurfaceFlinger 仍持有的 buffer，
  通过 completion callback 释放，避免输入 ring 和显示 ring 互相踩写。

## 本轮修掉的真 bug

**FACT**：v3 metadata 已扩展为两个 handle，但 guest 的 framing 判断仍只接受 v2 magic，
因此没有读取 app 在每帧发送的 `NEW/REUSE` 字节，并在 reuse 帧继续等待不存在的 handle。
修复为 v2 与 v3 都读取同一个 framing 字节，并保持 `handle_count=2` 的 slot 导入语义。
修复前固定 ring 无法持续；修复后 150/150 帧通过。

## 资源

同一次会话：

```text
before: VmRSS=174368 KiB  threads=46  fds=140
after:  VmRSS=205172 KiB  threads=46  fds=140
after+5s: fds=140
```

**FACT**：fd 和线程回到会话前基线，没有累积；RSS 会话后保留约 31 MiB，当前只能标记为
allocator/driver cache，尚不能声称 RSS 完全持平。

## 边界（不能误写成完成）

**FACT**：MediaCodec 仍是 ByteBuffer → CPU I420/YV12 拷贝，不是 decoder→GPU 零拷贝。
本证据打通的是“固定 YV12 AHB → 持久 GPU 转换 → RGBA AHB → SurfaceControl 上屏”这一段。

**仍未完成**：guest WSI/swapchain、VA-API/GStreamer/Chromium、HEVC/VP9/AV1 bridge、
AV 同步、10 分钟实时播放、丢帧率与硬软解 CPU 对比、跨设备验证、2.0 proot 替换。
本测试使用 `HUSK_VIDEO_REALTIME=0`，因此 150 帧证明数据路和显示路正确，不证明 30 fps
实时播放已达标。

## 组件哈希

```text
b77e8d361eacb3131aad011bdefea0c2d3f5d881e53b4e32953d81cd77553bc3  vulkan-probe
60b0bcfaa4b491fd050d28d7204237cfe8640eb3f235510fe14e7c8672aba121  yuv_to_rgba.spv
ea173f48b982c2fedd10f6d40450da13f5a79b8177c5021c7f04a4234ba072e6  libhuskapp.so
401ca489299381fee7fc4f6c4ece4d51d49c82cc8efd7da6e5bbee12124b1737  app-debug.apk
```

## 复现

构建 guest probe：

```sh
cd runtime
NDK=$NDK
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu
```

运行顺序：点亮并解锁设备，启动 NativeActivity，等待 `video|bridge|LISTEN`；启动
`vulkan-probe-yuv-new --video-ahb-frames 150`；再启动 `video-bridge ... 150`。
全部跑完后 `force-stop` app 并息屏。

## 原始证据

- [`79-yuv-rgba-present-app.log`](79-yuv-rgba-present-app.log)
- [`79-yuv-rgba-present-guest.log`](79-yuv-rgba-present-guest.log)
- [`79-yuv-rgba-present-bridge.log`](79-yuv-rgba-present-bridge.log)
- [`79-yuv-rgba-present-resources.log`](79-yuv-rgba-present-resources.log)
- [`79-yuv-rgba-present.png`](79-yuv-rgba-present.png)
- [`79-yuv-rgba-present.sha256`](79-yuv-rgba-present.sha256)
