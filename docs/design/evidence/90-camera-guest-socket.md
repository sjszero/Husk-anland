# Evidence 90 — 相机下半场第一步：guest 进程真的收到了相机帧

日期：2026-11-23
设备：RMX3357 / MT6893 / Android 13 / arm64；域 `untrusted_app`

## 结论

**FACT：跨进程链路打通。** app（纯 Rust Camera2 NDK）把 NV12 帧通过
`rootfs-full/run/husk/camera.sock` 交给 **guest 侧的 Rust 客户端**
（`runtime/crates/camera-client`，走 Husk 自己的 loader/shim 运行）。

- `OPEN w=1280 h=720 id=1` → `OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=1`
- 20 帧连续取回：`PASS frames=20 elapsed_ms=928` → **21.5 fps 端到端**
  （含 socket 拷贝，1280x720 NV12 = 1.38 MB/帧 ≈ 29 MB/s）。
- 帧时间戳间隔稳定 33.3 ms（`...22107000 → ...55429000` = 33,322 µs）= 传感器 30 fps 节奏。
- guest 收到的第 20 帧重建为 PNG，是真实画面（天花板/灯罩/风扇/门框）：
  `90-camera-guest-frame.png`，Y `mean=16.73 max=190`。

## 协议（当前版本，先验证传输，后面 V4L2 层复用同一服务）

```text
-> OPEN w=1280 h=720 [id=1]
<- OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=1
-> FRAME
<- FRAME seq=N ts=US w=1280 h=720 bytes=1382400  + 原始 NV12
<- NOFRAME            (1 秒内没有新帧)
-> STOP
<- BYE
```

一次只服务一个客户端（打开相机是独占资源），`OPEN` 时把请求尺寸吸附到设备真实支持的
尺寸列表（真实枚举结果来自 `ACameraMetadata SCALER_AVAILABLE_STREAM_CONFIGURATIONS`）。

## 端口/路径

- socket：`files/rootfs-full/run/husk/camera.sock`（必须在 rootfs 内，evidence 88 的教训）
- marker：`files/camera` 或 `files/rootfs-full/tmp/husk-camera/enabled`

## 复现

```sh
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "touch files/camera;
  mkdir -p files/rootfs-full/tmp/husk-camera; echo 1 > files/rootfs-full/tmp/husk-camera/enabled"'
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/data/com.linuxhub.husk/files/rootfs-full
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files &&
  NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full ./camera-client $R/run/husk/camera.sock \
  $R/tmp/husk-cam-frames frames=20 w=1280 h=720 id=1'"
```

## 边界（FACT）

- 不是 V4L2：guest 现在还得用我们自己的 `camera-client`，`/dev/videoN` 还没出现。
- 只有 NV12；YUYV 没做。没有 YUYV/NV12 之外任何格式，没有 controls，没有 crop。
- 单客户端、无握手超时重试；相机被系统收回（`onDisconnected`）时只写日志、不自动恢复。
- 预览带宽 29 MB/s 是 socket 拷贝的成本，未做共享内存/零拷贝。
