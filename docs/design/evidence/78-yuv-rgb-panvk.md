# Evidence 78 — PanVK 将真实 YV12 AHB 转成 RGBA8

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
运行域：`untrusted_app`（`com.linuxhub.husk`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT**：真实 `c2.mtk.avc.decoder` 输出的 1920x1080 YV12 AHardwareBuffer，已经作为
`VK_EXT_external_memory_dma_buf` storage buffer 被 guest 内 PanVK 直接导入。Rust 探针用
Vulkan compute shader 按 BT.601 limited-range 整数公式转换为 packed RGBA8，GPU fence 后
回读 9 个点，与同一 dma-buf 上 CPU 计算出的 9 个值 **逐位一致**：

```text
vk|yuv_rgb|external_props|features=0x6 export=0x201 compatible=0x201
vk|yuv_rgb|input_import|VkResult=0|size=3110400|required=3110400|fd_size=3137536|type=2
vk|yuv_rgb|samples|expected=[ffff017a, fffd0000, ffffff00, ff0000ff, ffff0000, ff0003fd, ff0000ff, ffff0000, ffffff00]|gpu=[ffff017a, fffd0000, ffffff00, ff0000ff, ffff0000, ff0003fd, ff0000ff, ffff0000, ffffff00]
vk|yuv_rgb|PASS|pixels=9/9|frame=1920x1080|offsets=[0, 2592000, 2073600]|strides=[1920, 960, 960]
```

同一轮固定 3-slot AHB ring 回归也完成 150/150 帧：

```text
video|ahb-ring|SUMMARY|PASS|sent=150|completed=150|handles=3|reuses=147
vk|video-ahb|ring_summary|slots=3|imports=3|reuses=147
vk|video-ahb|SUMMARY|PASS|completed=150/150
RESULT PASS
```

## 关键实现

- `runtime/crates/vulkan-probe/src/yuv.rs`：把 dma-buf 作为外部 storage buffer 导入，
  创建 host-visible RGBA8 输出 buffer，执行 compute pipeline、fence、map/readback。
- `runtime/crates/vulkan-probe/shaders/yuv_to_rgba.comp`：YV12 的 `Y/plane1/plane2`
  通过 push constants 提供的 offset/stride 读取；`plane1` 作为 V，`plane2` 作为 U。
- `runtime/crates/vulkan-probe/shaders/yuv_to_rgba.spv`：SPIR-V 2 进制，运行时通过
  `include_bytes!` 嵌入，不依赖 guest 内的 shader 编译器。
- `runtime/crates/vulkan-probe/src/dmabuf.rs`：固定 ring 第一帧增加 CPU 参考值 + GPU
  YUV→RGBA 门禁；后续帧继续执行原有三平面 CPU/GPU FNV-1a 校验。

YV12→RGBA8 的整数公式：

```text
c = max(Y - 16, 0)
d = U - 128
e = V - 128
R = clamp((298*c + 409*e + 128) >> 8, 0, 255)
G = clamp((298*c - 100*d - 208*e + 128) >> 8, 0, 255)
B = clamp((298*c + 516*d + 128) >> 8, 0, 255)
```

## 边界（不能误写成完成）

**FACT**：这次只验证了**第一帧**的 GPU YUV→RGBA 转换，后续 149 帧仍只做原有的三平面
GPU 哈希校验。当前每帧仍会重建转换资源，不是持久 converter。

**FACT**：MediaCodec 输出仍是 ByteBuffer → CPU I420→YV12 拷贝，再进固定 AHB；
不是 decoder→GPU 零拷贝。

**FACT**：转换结果只写入探针自己的 host-visible RGBA8 buffer 并回读，没有写回 app AHB，
没有接 guest WSI/swapchain，也没有经过 SurfaceFlinger 显示。

**仍未完成**：持久 YUV converter、per-frame GPU 转换、RGBA 输出写回 AHB、真实播放器出画、
guest WSI、VA-API/GStreamer/Chromium、HEVC/VP9/AV1 bridge、AV 同步、10 分钟实时播放、
丢帧率与硬软解对比、2.0 proot 替换。

## 组件哈希

```text
040a41d6efc94d434dfec4a1a0c391d4aaddbf335a3d513e9c58fd6f3d5b5df5  vulkan-probe
e579bbe5f046722df51424581b0e3d784494df2fe4071fd9a24c79bdd1bfb519  yuv_to_rgba.spv
eabc2457e25abe119a312e13c5322ce4811d6f3b4690b2370f507a347a698c11  app-debug.apk
```

## 复现

生成 SPIR-V：

```sh
GLSLC=/Users/l/Library/Android/sdk/ndk/29.0.14206865/shader-tools/darwin-x86_64/glslc
$GLSLC -O -o runtime/crates/vulkan-probe/shaders/yuv_to_rgba.spv \
  runtime/crates/vulkan-probe/shaders/yuv_to_rgba.comp
```

构建 guest probe：

```sh
cd runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu
```

运行顺序与 evidence 77 相同：先启动 NativeActivity 并等待 `video|bridge|LISTEN`，再启动
guest `vulkan-probe-yuv --video-ahb-frames 150`，最后启动 guest `video-bridge`。

## 原始证据

- [`78-yuv-rgb-app.log`](78-yuv-rgb-app.log) — app 侧固定 ring 与解码会话日志
- [`78-yuv-rgb-guest.log`](78-yuv-rgb-guest.log) — guest 侧 YUV→RGBA 与 150 帧完整日志
- [`78-yuv-rgb-bridge.log`](78-yuv-rgb-bridge.log) — guest 解码/控制通道日志

日志 SHA-256：

```text
aed3d99cddf638c586271ea605d763425c2d34812b73f42449ae4beffd98bf25  78-yuv-rgb-app.log
c55e3e698c4872c0c94de5d8b07dc674784a315e425b01e0070d861ef7f996fd  78-yuv-rgb-guest.log
5e6075c065d5c1ffc6b73efe50a2b7ba95378bf20e3af534f300a2ac8d069562  78-yuv-rgb-bridge.log
```
