# evidence 52 — guest AHB → EGLImage → Android 窗口（GPU 合成回退）

- 日期：2026-11-23 03:47 CST
- 设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
- 状态：**PASS（展示回退）**；**不是零拷贝完成**
- 产物：
  - APK `95b668bf4f3381c23517457798dd3292305cb59c9d81b54500de15320d594772`
  - `libhuskapp.so` `7dfe12b7f494205fd8c24e4cb6febd7e73e24886c66fa1ba9dec0a7bfbf9eeda`
  - guest `vulkan-probe` `ccb176858a5f83ed4f7bac9d75a9b2bb54544b99efbd7204acf9e7318df15e26`
  - screenshot `ed564edb62f72a5df0422dffc7310a000ca8303610ce0cae3086ed6e2d5dc8eb`

## 结论

这次真实走通了：

```text
guest PanVK GPU image clear
  -> 同一 AHB dma-buf 红色 9/9
  -> app 读取 AHB 红色 9/9
  -> app EGLImage 导入同一 AHB
  -> GPU 采样并 eglSwapBuffers
  -> Android 窗口全屏红色
```

截图：[`52-husk-red-surface.png`](52-husk-red-surface.png)

这是 **app 侧 EGL 合成回退**：guest 写的是 app 分配的同一块 AHardwareBuffer，app 用
`eglGetNativeClientBufferANDROID` + `eglCreateImageKHR` + `glEGLImageTargetTexture2DOES`
（三者运行时解析，避免最低 SDK 24 在加载期绑定不存在的符号）做一次 GPU 采样后上屏。它证明了跨进程 AHB 和 Android 展示链路可用，但**不是**
`ASurfaceControl_setBuffer` / `ANativeWindow_queueBuffer` 的零拷贝提交。

## `ASurfaceControl` 失败点（FACT）

第一次尝试 `ASurfaceControl_createFromWindow(window, "husk-ahb")` 返回 null。设备日志：

```text
E SurfaceComposerClient: SurfaceComposerClient::createWithSurfaceParent error Invalid argument
I Husk: ahb|present|control=0x0
```

AOSP Android 13 的 `native/android/surface_control.cpp` 在该窗口没有
`Surface::getSurfaceControlHandle()` 时走 `createWithSurfaceParent`；SurfaceFlinger 的
`Client::createWithSurfaceParent` 明确对该路径返回 `BAD_VALUE`（BLAST 下该 API 不再成立）。
因此 NativeActivity 当前拿到的窗口不能作为 `ASurfaceControl` 的 parent。这个失败是窗口类型/
平台 API 约束，不是 dma-buf 导入失败。

## 复现命令

```sh
cd $REPO

NDK=$NDK
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
(cd app && ./gradlew :app:assembleDebug -q)

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu \
  --manifest-path runtime/Cargo.toml

adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-v19
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cat /data/local/tmp/vulkan-probe-v19 > files/vulkan-probe-v19
  chmod 755 files/vulkan-probe-v19
  [ ! -e files/ahb-bridge.sock ] || unlink files/ahb-bridge.sock
  : > files/ahb-bridge
'"
adb -s UKHMMB8PYTGECMNV logcat -c
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start -n com.linuxhub.husk/android.app.NativeActivity

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/user/0/com.linuxhub.husk/files
  R=\$F/rootfs-full
  ICD=\$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
  export VK_ICD_FILENAMES=\$ICD VK_DRIVER_FILES=\$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink
  cd \$F
  timeout 60 sh gexec2.sh rootfs-full \$F/vulkan-probe-v19 \
    --ahb-socket \$F/ahb-bridge.sock 2>&1
'"
adb -s UKHMMB8PYTGECMNV exec-out screencap -p > /tmp/husk-red-surface.png
```

## 原始日志（关键行）

guest：

```text
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|dmabuf|linear|PASS|match=9/9|64x64 stride=64 pitch=256 len=16384
vk|dmabuf|image_readback|PASS|red=9/9 first=0xff0000ff
vk|dmabuf|image_dma_buf_view|PASS|red=9/9 first=0xff0000ff
vk|dmabuf|image_summary|PASS|LINEAR 2D image imported, cleared by GPU, read back 9/9 red
vk|dmabuf|display_handoff|PASS|guest GPU image ready; app may present
```

app：

```text
ahb|present|readback|red=9/9|unlock=0
ahb|present|control=0x0
ahb|present|surface_control_unavailable|egl_fallback
ahb|present|egl_client=0xb400007aa5510010 size=64x64
ahb|present|egl_image=0xb400007a2eb7c040
ahb|present|egl_swap=1
ahb|present|EGL_FALLBACK|true
ahb|bridge|PRESENT|true
```

截图全屏红色，状态栏和导航栏正常，说明不是只读内存成功而是真的进了 Android 合成器。

## 边界（不能写成完成的部分）

- 未验证 `ASurfaceControl` 零拷贝路径（当前返回 null）。
- 未验证 `ANativeWindow_queueBuffer` / present fence / acquire fence。
- 未验证旋转、DPI、多窗口、Surface 重建和连续 10 分钟泄漏。
- EGLImage 回退多了一次 GPU 采样/合成，开销尚未测量；不能作为最终性能结论。
- 下一步应换成带 SurfaceControl handle 的 `SurfaceView`/等效窗口路径，或建立可验证的
  AHB → SurfaceFlinger 零拷贝提交；在此之前显示验收仍是 **部分通过**。
