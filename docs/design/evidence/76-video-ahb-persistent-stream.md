# Evidence 76 — 单连接持久 AHB 交接，150/150 帧完整送达

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通 `untrusted_app`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT**：在 evidence 75 的逐帧连接路径上，app 与 guest 已改成**整个解码会话只建立一条
Unix socket 连接**。app 最多保留 3 个已解码 `AImage` / AHB 在途；guest 完成 PanVK 三平面
校验后回 ACK，app 收到 ACK 才释放对应 `AImage`，形成有界背压。

**FACT**：真实 `c2.mtk.avc.decoder` 的 150 个输出帧全部完成交接：
app `FRAME=150`、`ACK=150`、`RELEASE=150`、`FAIL=0`；guest
`frame_summary PASS=150`、`gpu_luma PASS=150`、`gpu_plane1 PASS=150`、
`gpu_plane2 PASS=150`，最终 `SUMMARY PASS|completed=150/150`。

**FACT**：上一版在 `CLOSE` 时只送达 147/150。原因是 app 最多压着 3 个未 ACK 的 AImage，
`shutdown()` 立即停止 worker 并删掉这 3 帧。本版在停止 worker 前最多等待 30 秒，让在途
帧完成 ACK/RELEASE；本次实测尾部 3 帧（index 147/148/149）均送达，没有再丢失。

**FACT（边界）**：这还不是最终的预分配 3-slot AHB import ring。guest 仍对每帧创建、
导入并销毁一次 Vulkan image/memory；本次只消除了每帧重新建连接和尾部丢帧。YUV→RGB、
guest WSI/swapchain、VA-API、GStreamer/Chromium、HEVC/VP9/AV1、AV 同步和 10 分钟门禁
仍未完成。

## 组件哈希

```text
56addd9dad897c9674134f0087bcc0eb74c05a71f1c08a8d36780a260eac543b  app-debug.apk
4e1017b661c1045de5eae643f2e9aa2bdf7b7241bdd601ccfe154555354d93c1  libhuskapp.so
f005d26e038130cd6e9f75d177eaae141e81004d57a65e7267fb818611a9042f  vulkan-probe
```

## 真机关键输出

### app：单连接、150 帧 ACK 后释放

```text
video|ahb-ring|SESSION|START
video|ahb-ring|LISTEN|.../rootfs-full/run/husk/video-ahb.sock
video|ahb-ring|CONNECT|fd=148|.../video-ahb.sock
video|ahb-ring|FRAME|index=1|bytes=549|send_rc=0|socket=148
video|ahb-ring|ACK|index=1|byte=0x44|ok=true|elapsed_ms=112
video|ahb-ring|RELEASE|index=1
...
video|ahb-ring|FRAME|index=150|bytes=557|send_rc=0|socket=148
video|ahb-ring|ACK|index=150|byte=0x44|ok=true|elapsed_ms=106
video|ahb-ring|RELEASE|index=150
video|ahb-ring|SUMMARY|PASS|sent=150|completed=150
video|ahb-ring|SESSION|STOP
```

计数：`FRAME=150`、`ACK=150`、`RELEASE=150`、`SESSION START/STOP 各 1`、
`ahb-ring FAIL=0`、`video|bridge FAIL=0`。完整日志见
[`76-video-ahb-app.log`](76-video-ahb-app.log)。

### guest：一条流完成 150 帧三平面 GPU 校验

```text
vk|video-ahb|STREAM|CONNECTED|path=/run/husk/video-ahb.sock|frames=150
vk|video-ahb|frame_summary|PASS|index=0|elapsed_ms=130
...
vk|video-ahb|frame_summary|PASS|index=149|elapsed_ms=107
vk|video-ahb|STREAM|CLOSED|completed=150/150
vk|video-ahb|SUMMARY|PASS|completed=150/150
```

计数：`STREAM CONNECTED=1`、`STREAM CLOSED=1`、`frame_summary PASS=150`、
`gpu_luma/gpu_plane1/gpu_plane2 PASS=150`、`FAIL=0`。完整日志见
[`76-video-ahb-guest.log`](76-video-ahb-guest.log)。

### 解码端：真实 MTK 硬解仍为 150 帧

```text
OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=18530
RESULT PASS
```

完整日志见 [`76-video-ahb-bridge.log`](76-video-ahb-bridge.log)。测试使用
`HUSK_VIDEO_REALTIME=0`，这不是实时播放门禁。

## 实现要点

- `ebridge` 每进程维护一个持久 `Session`，一条 `UnixStream` 贯穿整场解码。
- `MAX_INFLIGHT = 3`，app 在 `inflight == 3` 时等待 ACK，不再继续持有更多 `AImage`。
- guest 对每帧完成三平面 GPU 回读后发送 `D`；app 收到后才调用 `AImage_delete` 并发出
  `RELEASE` 日志。
- `shutdown()` 先 `drain_before_stop(30s)`，确认在途帧全部 ACK/RELEASE 后才停止 worker，
  修复了尾部三帧被提前删除的 147/150 缺陷。
- 测试结束后没有 `video-bridge` / `vulkan-probe` / `proot` / `virgl` 残留进程；
  guest 控制流 `RESULT PASS`。app 取样时 fd/RSS 尚未做前后快照，资源数字只覆盖协议计数，
  不能写成无泄漏证明。

## 复现命令

构建 app：

```sh
cd $REPO/app-rs
NDK=$NDK
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android
cp target/aarch64-linux-android/release/libhuskapp.so \
  ../app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
cd ../app && ./gradlew :app:assembleDebug
```

运行时探针沿用 evidence 75 的 release 构建；设备侧测试脚本先启动 guest
`vulkan-probe-video --video-ahb-frames 150`，再启动 `video-bridge ... 150`。注意：
marker、socket 清理和 app 启动必须按 evidence 75 的顺序执行；不能在 app 已经 `LISTEN`
之后删除 `rootfs-full/run/husk/video.sock`，否则 listener 仍绑定但路径已被 unlink。

## 哈希

```text
687d17929190abd2a4ba9ea880dc33049a40c71f9f787ab0e539ea7983740061  76-video-ahb-guest.log
74d042f89875d67eca4386da51c4b65a0eef8f2fe0bdabbdbe899943e2fac5cf  76-video-ahb-bridge.log
1abf51b47e3e3682f7f5fae2b01a2c246934aac08c69677a7f86a962e87ff7af  76-video-ahb-app.log
```
