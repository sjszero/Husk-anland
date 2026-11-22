# evidence 28 — 黑屏根因：`ANativeActivity.callbacks` 指针被平台覆盖

日期：2026-11-22 21:00 CST
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 1080x2400 / 4 KiB
构建：`huskapp` = 纯 Rust `libhuskapp.so`，`android:hasCode="false"` + `android.app.NativeActivity`

## 症状

`dumpsys window` 里窗口一切正常：

```
mHasSurface=true isReadyForDisplay()=true
mSurface=Surface(name=com.linuxhub.husk/android.app.NativeActivity)
mDrawState=HAS_DRAWN  isOnScreen=true  isVisible=true
```

但屏幕全黑，logcat 里 **一条 `onNativeWindow` 都没有**。
窗口的 `HAS_DRAWN` 是 WindowManager 自身画的背景，不是我们画的。

## 根因（FACT）

`ANativeActivity_onCreate` 里原来的写法是**替换指针**：

```rust
(*activity).callbacks = &CALLBACKS as *const _ as *mut ANativeActivityCallbacks;
```

平台在我们返回之后又把 `activity->callbacks` 指回它自己的静态表，
于是我们整张回调表一条都收不到——`onStart` / `onResume` / `onInputQueueCreated` /
`onNativeWindowCreated` / `onDestroy` 全部丢失。只有 `onCreate`（平台主动调我们）会有反应。

修复：**原地覆盖平台给的那张表**，不动指针。

```rust
let fw_cb = (*activity).callbacks;          // 平台给的表
if !fw_cb.is_null() {
    core::ptr::copy_nonoverlapping(&CALLBACKS as *const _ as *const u8,
                                   fw_cb as *mut u8,
                                   core::mem::size_of::<ANativeActivityCallbacks>());
} else {
    (*activity).callbacks = &CALLBACKS as *const _ as *mut _;   // 兜底
}
```

两种模型下都成立：平台若每次事件都重新解引用 `activity->callbacks`，原地写当然有效；
平台若只认自己那张静态表，原地写也有效。指针替换只在第一种下成立。

## 验证（实测）

```
I Husk : onCreate callbacks=0xb4000078c4923790 -> in-place copy (128 bytes)
I Husk : cb: onStart
I Husk : cb: onResume
I Husk : cb: onInputQueueCreated q=0xb4000078a9570c80
I Husk : cb: onNativeWindow win=0xb4000078a94ff010 1080x2400
```

截图：终端首屏可见 `bash -i` 的提示符与光标；`groups` 的告警也照常刷出来。
GL 初始化一次成功（`EGL surface failed` 未出现），1080x2400 → 132x60 字符格。

## 顺带的诊断增强（保留）

每个回调都打一行 `cb: ...`；`sync_surface` 在还没拿到窗口时打一次
`sync: waiting for onNativeWindowCreated`；`EGL surface failed` 带上窗口地址。
这三条让「黑屏」以后能一次分清是「没窗口」「窗口 0x0」还是「EGL 建面失败」。
