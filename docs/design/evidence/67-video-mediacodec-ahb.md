# Evidence 67 — MediaCodec 硬解与 AHardwareBuffer dma-buf 输出（2026-11-23）

设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB 页。
运行域：Husk NativeActivity 的普通 app 进程（bionic，`untrusted_app`）。
目的：在写 guest 通道之前，证明本机 MediaCodec 真能选中 MTK Codec2、解出 1080p30，
并把输出作为 AHardwareBuffer 交给 app，且能从 native handle 取得 dma-buf fd。

## 结论

- **FACT**：H.264 1080p30 使用 `c2.mtk.avc.decoder`，150/150 帧，EOS，654 ms。
- **FACT**：HEVC 1080p30 使用 `c2.mtk.hevc.decoder`，150/150 帧，EOS，1441 ms。
- **FACT**：两条路径都通过 `AImageReader_newWithUsage(..., AIMAGE_FORMAT_PRIVATE,
  AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE, ...)` 得到 AHardwareBuffer。
- **FACT**：每个采样到的 AHB native handle 有 3 个 fd；app 不需要读像素即可取得 fd。
- **FACT**：AHB 描述为 1920x1080、layers=1、format=842094169（YUV420_888）、stride=1920。
- **FACT**：codec stop/delete、ImageReader delete、extractor delete 均返回成功。
- **UNKNOWN**：fd 尚未经 kbase `MEM_IMPORT` 导入 guest/Mesa；不能把本证据写成
  guest 零拷贝已完成。
- **UNKNOWN**：当前探针在 app 内用 `AMediaExtractor` 读 MP4；正式路径仍是 guest 解复用，
  独立通道把压缩帧和 CSD 送进来。此探针只验证硬解和 AHB 输出。

## 构建与安装

```sh
cd app-rs
NDK=$ANDROID_HOME/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android
cp target/aarch64-linux-android/release/libhuskapp.so \
  ../app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
cd ../app && ./gradlew :app:assembleDebug
adb -s UKHMMB8PYTGECMNV install -r app/build/outputs/apk/debug/app-debug.apk
```

产物哈希：

```text
APK       623bb5a38f7b28b3e9eb71ca3212cbbd9789207ad17a62c362411fb08b110094
libhuskapp.so 68025b0351f902632cf4355fcc7652d14b01083002f854e0474c75b294de0a05
```

## 测试素材

宿主用 ffmpeg 生成，只作为探针输入：

```sh
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 5 -an \
  -c:v libx264 -preset ultrafast -tune zerolatency -pix_fmt yuv420p \
  -movflags +faststart avc-1080p30.mp4
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 5 -an \
  -c:v libx265 -preset ultrafast -x265-params log-level=error -pix_fmt yuv420p \
  -tag:v hvc1 -movflags +faststart hevc-1080p30.mp4
```

marker：

```sh
adb shell run-as com.linuxhub.husk sh -c \
  'printf %s surface:avc-1080p30.mp4 > files/video-probe'
```

## AVC 关键日志

```text
video|probe|BEGIN|path=/data/user/0/com.linuxhub.husk/files/avc-1080p30.mp4|bytes=9823326|mode=image-reader-ahb
video|probe|track|index=0|mime=video/avc
video|probe|codec|requested=c2.mtk.avc.decoder|actual=c2.mtk.avc.decoder
video|probe|image-reader|size=1920x1080|usage=0x100|window=0xb400007aa2345810|rc=0
video|probe|configure|rc=0
video|probe|start|rc=0
video|probe|ahb|n=1|timestamp_ns=0|ts_rc=0|size=1920x1080|layers=1|format=842094169|usage=0x103|stride=1920|handle_version=12|num_fds=3|first_fd=233
video|probe|done|pass=true|samples=150|input_bytes=9821765|frames=151|output_bytes=150|images=150|eos=true|elapsed_ms=654
video|probe|stop|rc=0
video|probe|delete-codec|rc=0
video|probe|delete-image-reader
video|probe|delete-extractor|rc=0
```

完整日志：`67-video-mediacodec-ahb-avc.log`。

## HEVC 关键日志

```text
video|probe|BEGIN|path=/data/user/0/com.linuxhub.husk/files/hevc-1080p30.mp4|bytes=2154642|mode=image-reader-ahb
video|probe|track|index=0|mime=video/hevc
video|probe|codec|requested=c2.mtk.hevc.decoder|actual=c2.mtk.hevc.decoder
video|probe|image-reader|size=1920x1080|usage=0x100|window=0xb400007aa2345810|rc=0
video|probe|configure|rc=0
video|probe|start|rc=0
video|probe|ahb|n=1|timestamp_ns=0|ts_rc=0|size=1920x1080|layers=1|format=842094169|usage=0x103|stride=1920|handle_version=12|num_fds=3|first_fd=207
video|probe|done|pass=true|samples=150|input_bytes=2149375|frames=151|output_bytes=150|images=150|eos=true|elapsed_ms=1441
video|probe|stop|rc=0
video|probe|delete-codec|rc=0
video|probe|delete-image-reader
video|probe|delete-extractor|rc=0
```

完整日志：`67-video-mediacodec-ahb-hevc.log`。

## 失败记录与边界

第一次 AVC 运行发生在设备已经息屏时。视频线程已输出到第 76 帧，但 app 的 UI 线程在
`eglCreateWindowSurface` 内部触发：

```text
FORTIFY: pthread_mutex_lock called on a destroyed mutex
```

这是 Surface 生命周期竞争，不是 MediaCodec 输出错误；屏幕点亮后重跑完整通过。该问题单独按
生命周期缺陷处理，不能因此把视频证据失败化，也不能把“生命周期已完成”继续当成无条件成立。

探针进程 PID：11882。
