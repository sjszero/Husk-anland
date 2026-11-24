# evidence 53 — guest AHB 经 SurfaceControl 零拷贝上屏

- 日期：2026-11-23 03:58 CST
- 设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
- 状态：**PASS（静态单帧零拷贝展示）**
- 产物：
  - APK `e7e5da9ab3dcf7f0fee84afef48fc4c04b94074cecec181af7a950e1ca9e827c`
  - `libhuskapp.so` `8644002c82f50317553e72f514b35c8b7327d56fe8fe76f2441566c99d1f42be`
  - guest `vulkan-probe` `ccb176858a5f83ed4f7bac9d75a9b2bb54544b99efbd7204acf9e7318df15e26`
  - 截图 `b627e91b2d3cf09eb5f03e1ddb036f3329e7905e6cc71a5de544e84d6d7a8a96`

截图：[`53-surfacecontrol-zero-copy.png`](53-surfacecontrol-zero-copy.png)

## 结论

这次走的是真正的 AHardwareBuffer → SurfaceControl → SurfaceFlinger 路径，不是 evidence 52
的 EGLImage 采样回退：

```text
guest PanVK GPU clear 同一 AHB
  -> app CPU readback 9/9 红
  -> 纯 Rust JNI 找到当前 Activity
  -> attach 平台 SurfaceView
  -> 读取 SurfaceView.mBlastSurfaceControl.mNativeObject
  -> ASurfaceControl_create(child)
  -> ASurfaceTransaction_setBuffer(同一 AHB)
  -> ASurfaceTransaction_apply
  -> SurfaceFlinger DEVICE 合成
  -> 截图正文区域全红
```

截图正文采样 24,192 / 24,192 个采样点为纯红 `(255,0,0)`；状态栏与导航栏保持系统绘制。

## 关键突破

`ASurfaceControl_createFromWindow` 对 NativeActivity 的 BLAST window 返回 null：

```text
E SurfaceComposerClient: SurfaceComposerClient::createWithSurfaceParent error Invalid argument
I Husk: ahb|present|control=0x0
```

解决方式不是改 Java，也不是退回 EGL 合成，而是：

1. 用纯 Rust JNI 从 `ActivityThread.mActivities` 找到当前 Activity；
2. 创建平台 `SurfaceView(Context)` 并 `setContentView`；
3. `SurfaceView` 在 Android 13 上创建 `mSurfaceControl` 和 `mBlastSurfaceControl`；
4. 读取 `mBlastSurfaceControl.mNativeObject`，得到真实 BLAST `SurfaceControl*`；
5. 把它作为 parent 调 NDK `ASurfaceControl_create`，再提交同一 AHB。

业务代码仍为零 Java、零 dex、零 C；JNI 只调用 Android 平台已有类，不携带任何自有 Java 类。

最初使用 `setZOrderMediaOverlay(true)` 时层已存在且 SF 显示为可见，但被 NativeActivity 主窗口
覆盖，截图仍是终端。改为 `setZOrderOnTop(true)` 后实际出现在系统栏之间的正文区域。
这是窗口层级事实，不是 buffer 内容问题。

## 原始日志（关键行）

```text
I/Husk: surface: activity|PASS|ActivityThread.mActivities
I/Husk: surface: install|PASS|SurfaceView on Activity 0x195
I/Husk: ahb|present|readback|red=9/9|unlock=0
I/Husk: surface: parent|PASS|native=0xb400007a45cba2a0
I/Husk: ahb|present|surface_parent=0xb400007a45cba2a0
I/Husk: ahb|present|control=0xb400007aa54708c0
I/Husk: ahb|present|PASS|SurfaceControl 1080x2400 from 64x64
```

SurfaceFlinger 层树（同一轮）：

```text
+ BufferStateLayer (husk-ahb#1846) uid=10302
  Region VisibleRegion (this=0 count=1)
    [  0, 120, 1080, 2359]
      z=100, pos=(0,120), size=(64,64),
      defaultPixelFormat=RGBA_8888,
      tr=[16.88, 0.00][0.00, 37.50],
      activeBuffer=[64x64: 802,RGBA_8888]
      parent=SurfaceView[com.linuxhub.husk/android.app.NativeActivity](BLAST)#1841
```

输出合成：

```text
- Output Layer (husk-ahb#1846)
  visibleRegion [0, 120, 1080, 2359]
  displayFrame=[0 120 1080 2359]
  sourceCrop=[0 0 64 60]
  dataspace=V0_SRGB
  hwc: composition=DEVICE (2)
```

## 复现命令

```sh
cd $REPO
NDK=$NDK

CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
(cd app && ./gradlew :app:assembleDebug -q)

adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  [ ! -e files/ahb-bridge.sock ] || unlink files/ahb-bridge.sock
  : > files/ahb-bridge
'"
adb -s UKHMMB8PYTGECMNV logcat -c
adb -s UKHMMB8PYTGECMNV shell am start \
  -n com.linuxhub.husk/android.app.NativeActivity

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
adb -s UKHMMB8PYTGECMNV exec-out screencap -p \
  > docs/design/evidence/53-surfacecontrol-zero-copy.png
```

## 边界（不能写成整个显示目标完成）

- 已验证：同一 AHB 由 guest GPU 写入、app 通过 `ASurfaceControl` 提交、SurfaceFlinger
  `DEVICE` 合成上屏；静态单帧。
- 未验证：acquire/release fence、连续帧 / 背压、buffer 复用、GBM / Wayland / X11、Vulkan WSI /
  swapchain、旋转、DPI、多窗口、SurfaceView 生命周期重建和 10 分钟泄漏。
- 当前 probe 在提交后保留 5 秒再释放，只为取证；产品路径必须改为按 frame release 回调复用。
- `mBlastSurfaceControl.mNativeObject` 是平台实现字段，必须保留运行时探测和 EGLImage 回退；
  不能在字段缺失时崩溃。
