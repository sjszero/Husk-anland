# Evidence 89 — 相机上半场：纯 Rust Camera2 NDK 抓到真实画面

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / arm64
域：`untrusted_app`（`com.linuxhub.husk`），无 root

## 结论

**FACT：app 侧相机链路已经在纯 Rust 里跑通并抓到真实画面**（不是假帧、不是静态图）：

- 枚举到 2 个相机：`id=0` BACK，sensor orientation 90；`id=1` FRONT，270；
  两者都报 16 档 `YUV_420_888` 尺寸（1920x1080 … 1560x720）。
- 前置相机 1280x720 连续 4 帧，`YUV_420_888` → NV12 转换后落盘，每帧 1,382,400 字节。
- 帧内容是真的：Y 平面 `mean=15.60 stddev=22.89 max=188`，直方图跨 6 个 32 宽桶，
  重建 PNG 能看到天花板、灯罩、风扇叶片、门框（`89-camera-front-frame.png`）。
- 后置相机同一路径也出帧，但手机背面朝下（桌面），所以是黑画面：
  Y 平面 `mean≈0.3 max=2`，帧间有 5.29% 像素不同（传感器噪声），**说明是活的传感器而不是
  未填充的缓冲区**。这一条要写清楚，不能拿黑帧当"相机可用"。

## 这个 ROM 上的 4 个 ABI 坑（都是实测，不是猜）

1. **`ACameraManager_create` 是"返回指针"变体**。按 NDK 头文件是
   `camera_status_t f(ACameraManager**)`，但这台机器的 `libcamera2ndk.so` 里
   该符号直接 `mov x0, x19` 返回新建对象，完全不写 out 参数（反汇编
   `0xf1f0`：`new(0x30)` → `pthread_mutex_init` → `mov x0, x19` → `ret`）。
   实测：`raw_create(&mut manager)` 返回 `0xb400007aa5426350`、`manager` 仍为 null。
   处理：两种 ABI 都试，`manager` 为空时把返回值当管理器。
2. **`ACaptureRequest_setEntry_u8` 的参数顺序是 `(request, tag, count, data)`**。
   我按 `(request, tag, data, count)` 写，直接 SIGSEGV：
   `memcpy_opt ← update_camera_metadata_entry ← ACameraMetadata::updateImpl ←
   ACaptureRequest_setEntry_u8.cfi`，`fault addr 0x1`。改成正确顺序后 `rc=0`。
3. **`ALooper_pollOnce` 没有 looper 参数**：`int ALooper_pollOnce(int timeout, int* fd,
   int* events, void** data)`，永远轮询"当前线程"的 looper。多传一个 `ALooper*` 会把
   指针低 32 位当超时（≈17 天），线程看起来"卡死"。
4. **导出符号拼写和头文件不完全一致**：这台机器上是 `ACaptureSessionOutputContainer_add`
   （不是 `_addOutput`）、`ACaptureRequest_setEntry_u8`（不是 `_uint8`）。
   做法已改为：先把每个符号用 `nm -D` 对着真机 `.so` 核一遍，再写绑定。

## 本次没有依赖 session 回调

`ACameraCaptureSession_stateCallbacks.onReady` 在这台机器上始终没有投递到我们的 looper
（`ready_flag=false` 一直为 false），但 `ACameraCaptureSession_setRepeatingRequest`
在创建 session 后直接调用就是 `rc=0`，并且立刻出帧。所以实现改成：
**不阻塞等 onReady，直接下发 repeating request，靠返回码判断**。

## 复现

```sh
# marker 里可以指定相机：frames=4 camera=1
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c \
  "echo \"frames=4 camera=1\" > files/camera-selftest"'
adb -s UKHMMB8PYTGECMNV install -r -g app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV shell 'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard;
  am start -n com.linuxhub.husk/android.app.NativeActivity'
adb -s UKHMMB8PYTGECMNV logcat -s Husk | grep 'camera|'
# 帧在 files/camera-frames/frame-NN.nv12（1280x720 NV12）
```

## 边界（FACT / UNKNOWN）

- 只做了**抓帧**：没有 guest 侧 `/dev/videoN`，没有预览窗口，没有拍照（JPEG），没有录像。
- 只测了 1280x720 一个尺寸、前置/后置各一台，没测 1920x1080 与切换。
- 帧率没有意义：selftest 只取 10 帧，`fps` 统计用的是图像时间戳差值，因为帧是排队出来的，
  这一版不能当吞吐证据。
- 后置相机拍到的黑画面**不能**当成"相机可用"的证据；可用证据只有前置那张真实场景图。
