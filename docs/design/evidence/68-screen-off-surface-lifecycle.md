# Evidence 68 — 息屏冷启动 Surface 生命周期崩溃修复（2026-11-23）

设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB 页。
组件：纯 Rust NativeActivity 壳（`libhuskapp.so`）。
目的：修复 evidence 67 暴露的息屏重启动崩溃，并确认修复没有破坏会话、切后台或旋转。

## 原始失败

AVC 视频探针第一次运行发生在设备已经息屏时。视频线程输出到第 76 帧后，app 的 UI 线程在
`eglCreateWindowSurface` 内部 abort：

```text
FORTIFY: pthread_mutex_lock called on a destroyed mutex
```

backtrace 顶点为 `android::Surface::hook_query`，调用来自 `eglCreateWindowSurface`。
这是 Windows/Surface 生命周期竞争，不是 MediaCodec 错误；点亮后同一视频路径完整通过。

## 根因判断

- **FACT**：原实现用 `WINDOW` 原子指针判断是否可建 EGL surface，没有记录
  `onNativeWindowCreated`/`onNativeWindowDestroyed` 之间的有效状态，也没有要求 Activity 已获得焦点。
- **INFERENCE**：息屏启动时 UI 循环可以在旧的/已失效窗口状态上尝试
  `eglCreateWindowSurface`；平台窗口已被销毁，故 Surface 内部 mutex 失效。
- **修复**：新增 `WINDOW_ALIVE` 状态，只有窗口回调有效且 `onWindowFocusChanged(true)` 后才允许
  `Gfx::new` / 窗口复用；窗口销毁或失焦时丢弃 GL 资源，PTY 会话保持运行。

## 代码

- `app-rs/src/lib.rs`
  - `WINDOW_ALIVE: AtomicBool`
  - `on_window`：窗口指针有效时置 true。
  - `on_window_gone`：先置 false，再清 `WINDOW`。
  - `ANativeActivity_onCreate`：重置为 false。
  - `App::sync_surface`：`WINDOW_ALIVE && FOCUSED` 不成立时不创建/复用 EGL surface，
    已有 `gfx` 释放，`win_ptr` 清零。

## 最终产物

```text
libhuskapp.so 978b75e8b4eeef1f29c6a18765860bb54b79c44cd54a4f42e75e772b0e07ce28
APK           80a31f590a2df0e7d1876362263c0b719de3c94b4826ca558c23c80519c16cd0
```

## 最终产物实机验证

命令：

```sh
adb shell 'am force-stop com.linuxhub.husk; input keyevent KEYCODE_SLEEP'
sleep 2
adb shell 'am start -n com.linuxhub.husk/android.app.NativeActivity'
sleep 5
adb shell 'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; input swipe 540 1800 540 400 250'
sleep 4
adb shell 'wm user-rotation lock 1'
sleep 4
adb shell 'wm user-rotation lock 0'
sleep 4
adb shell 'input keyevent KEYCODE_HOME'
sleep 3
adb shell 'am start -n com.linuxhub.husk/android.app.NativeActivity'
```

结果：

```text
P_OFF=16123 P_ON=16123 P_R1=16123 P_R0=16123
HOME_P0=16123 HOME_P1=16123 HOME_P2=16123
```

关键日志：

```text
cb: onStart
session: running pid=17011 (pgid=sid=pid by setsid) 80x24 fds=98
cb: onResume
cb: onPause
cb: onStop (session running, fds=120)
cb: onStart
cb: onResume
cb: onNativeWindow win=0xb400007aa2282010 1080x2400
cb: onWindowFocusChanged focused=1
session: resize 80x24 -> 67x58 (TIOCSWINSZ, pid=17011)
cb: onNativeWindow win=0xb400007aa2282010 2280x1080
session: resize 67x58 -> 142x26 (TIOCSWINSZ, pid=17011)
cb: onNativeWindow win=0xb400007aa2282010 1080x2400
session: resize 142x26 -> 67x58 (TIOCSWINSZ, pid=17011)
cb: onPause
cb: onNativeWindowDestroyed win=0xb400007aa2282010
cb: onStop (session running, fds=138)
cb: onStart
cb: onResume
cb: onNativeWindow win=0xb400007aa2282010 1080x2400
cb: onWindowFocusChanged focused=1
```

- 息屏冷启动后 app PID / guest PID 都存活。
- 点亮后同一 app PID 创建窗口并 resize，无重启。
- 运行中息屏/唤醒、HOME/回前台、横竖旋转均保持同一 PID。
- 本轮最终产物日志中 `FORTIFY` / `Fatal signal` / `am_crash` 计数为 0。
- 为排除偶发，同一修复逻辑在重建前的构建上另做过 3 次连续息屏冷启动：
  `13885/13885`、`14589/14589`、`15690/15690`（息屏 PID / 点亮后 PID）。

## 边界

本证据只关闭“息屏窗口状态导致 EGL 创建崩溃”这一条。LMK 直接杀进程、真实内存压力下的
`onLowMemory` 和长时 fd 趋势仍按 `docs/design/lifecycle.md` 的 G2/G3/G5 保持未验证。
