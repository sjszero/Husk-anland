# Evidence 49 — Android AHardwareBuffer 导出 dma-buf 并通过 Unix socket 传递

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通应用域，UID `u0_a302`）  
APK sha256：`dfe3dccee9851b4561ff60b043d391f09bf6f427ddb79d87d9cf0f2d23690899`  
`libhuskapp.so` sha256：`661fc92572d64d4fe78c00a082ff42e51036a2e8c45f4ffb050d16aae8cb2747`

## 结论

- **FACT**：纯 Rust NativeActivity 进程内成功 `dlopen("libandroid.so")`，并解析到
  `AHardwareBuffer_allocate`、`_describe`、`_release`、`_getNativeHandle`、
  `_sendHandleToUnixSocket`、`_recvHandleFromUnixSocket` 六个符号。
- **FACT**：用 usage `GPU_SAMPLED_IMAGE | GPU_FRAMEBUFFER` 分配
  `64x64x1 RGBA8` 的 AHardwareBuffer 成功，驱动返回 stride 128。
- **FACT**：`AHardwareBuffer_getNativeHandle()` 返回 version 12、3 个 fd、76 个 int；
  其中 fd 1 的 `/proc/self/fd` 目标是 `/dmabuf:G:pid [21007]`。
- **FACT**：`AHardwareBuffer_sendHandleToUnixSocket()` 在 `AF_UNIX/SOCK_SEQPACKET`
  上返回 0，`AHardwareBuffer_recvHandleFromUnixSocket()` 返回 0；接收端得到 3 个新的有效 fd，
  其中 fd 1 仍是 dmabuf。
- **INFERENCE**：本机 app/bionic 侧存在可用的 AHardwareBuffer → dma-buf fd 路线，
  不需要经过 Java，也不需要额外 C 代码。
- **UNKNOWN / 未验证（本条取证时）**：guest（glibc）进程跨进程接收该 fd 后，PanVK 是否能用
  `VK_EXT_external_memory_dma_buf` + `VK_KHR_external_memory_fd` 成功创建 image；
  AHardwareBuffer 的 native-handle fd 顺序也不是公开稳定 ABI，产品代码必须扫描 fd 并用
  `readlink(/proc/self/fd/N)` 找 dmabuf，不能把 fd 1 写死。
  其中跨进程 fd 传递与 **buffer** 导入/GPU 访问已由 **evidence 50** 补齐；image 导入仍未完成。
- **UNKNOWN / 未验证**：把同一块 AHB 通过 `ANativeWindow_queueBuffer` 交给 SurfaceFlinger、
  渲染完成 fence 和实际窗口显示尚未做。

## 复现方法

探针在 `ANativeActivity_onCreate` 中一次性启动，源码：
`app-rs/src/ahb.rs`。构建命令：

```sh
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
cd app
JAVA_HOME=${JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home} \
  ./gradlew :app:assembleDebug -q
adb -s UKHMMB8PYTGECMNV install -r app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV logcat -c
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start \
  -n com.linuxhub.husk/android.app.NativeActivity
adb -s UKHMMB8PYTGECMNV logcat -d -s Husk:V '*:S'
```

## 原始日志（关键段）

```text
I Husk    : ahb|probe|BEGIN
I Husk    : ahb|loader|PASS|libandroid.so
I Husk    : ahb|symbol|PASS|AHardwareBuffer_allocate
I Husk    : ahb|symbol|PASS|AHardwareBuffer_describe
I Husk    : ahb|symbol|PASS|AHardwareBuffer_release
I Husk    : ahb|symbol|PASS|AHardwareBuffer_getNativeHandle
I Husk    : ahb|symbol|PASS|AHardwareBuffer_sendHandleToUnixSocket
I Husk    : ahb|symbol|PASS|AHardwareBuffer_recvHandleFromUnixSocket
I Husk    : ahb|allocate|rc=0 ptr=0xb400007a45cbec00
I Husk    : ahb|describe|64x64x1 format=1 usage=0x300 stride=128
I Husk    : ahb|allocated|handle version=12 fds=3 ints=76
I Husk    : ahb|allocated|fd[0]=109 valid=true target=anon_inode:gralloc_extra
I Husk    : ahb|allocated|fd[1]=110 valid=true target=/dmabuf:G:pid [21007]
I Husk    : ahb|allocated|fd[2]=111 valid=true target=/dev/ashmemc9cae759-c33d-400e-8b08-d93f8623edaa
I Husk    : ahb|allocated|first_fd=109
I Husk    : ahb|socketpair|rc=0 tx=102 rx=103
I Husk    : ahb|send|rc=0
I Husk    : ahb|recv|rc=0 ptr=0xb400007a96d65900
I Husk    : ahb|received|handle version=12 fds=3 ints=76
I Husk    : ahb|received|fd[0]=114 valid=true target=anon_inode:gralloc_extra
I Husk    : ahb|received|fd[1]=115 valid=true target=/dmabuf:G:pid [21007]
I Husk    : ahb|received|fd[2]=116 valid=true target=/dev/ashmemc9cae759-c33d-400e-8b08-d93f8623edaa
I Husk    : ahb|received|first_fd=114
I Husk    : ahb|received|released
I Husk    : ahb|allocated|released
I Husk    : ahb|probe|DONE
```

同一轮启动还确认会话与 Surface 正常：`spawned pid=22330`、`session: running`、
`resize 80x24 -> 67x58`、`onNativeWindow 1080x2400`。

## 设计含义

当前显示零拷贝路线可以继续为：

1. app 侧分配 AHardwareBuffer；
2. 从 native handle 扫描出 dmabuf fd（不要硬编码索引）；
3. 通过独立 Unix socket 用 `SCM_RIGHTS` 发给 guest；
4. guest 用 PanVK raw dma-buf 导入；
5. 渲染后 app 用同一 AHB `queueBuffer`。

第 2 步已在本机实测；第 3 步的本机同进程 socket 收发已实测；跨进程到 glibc guest 与第 4/5 步
仍未验证。
