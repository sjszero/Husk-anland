# evidence 98 — 输入注入到 guest 合成器（mutter 的 RemoteDesktop API），全链路

设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 链路

```
Android MotionEvent/KeyEvent
  → app-rs/src/input.rs（手势识别，24 字节 evdev 记录）
  → run/husk/input.sock
  → guest: husk-rd rd /run/husk/input.sock
  → org.gnome.Mutter.RemoteDesktop（D-Bus，mutter 46 自带，无需补丁）
  → Clutter 虚拟输入设备 → 合成器
```

不使用 `/dev/input`、libinput、udev、XTEST，也不改 mutter：走的是 mutter 官方 D-Bus 接口。

## 真实按键的 mutter 侧日志（FACT）

```
$ adb shell input tap 540 1200
INPUT: Emitting virtual button-press  of button 0x110 (device 0x555af260a0)
INPUT: Emitting virtual button-release of button 0x110
$ adb shell input swipe 500 1400 800 1400 300
INPUT: Emitting virtual button-press  of button 0x110
INPUT: Emitting virtual button-release of button 0x110        ← 按住 ~300ms 的拖动
$ adb shell input keyevent KEYCODE_B
INPUT: Emitting virtual key-press  of key 0x1f (device 0x555af1c1b0)   ← 0x1f = KEY_B
INPUT: Emitting virtual key-release of key 0x1f
```

## 两个必须记住的 ABI 事实

1. **按钮用 evdev 码，不是 X11 按钮号**：传 1 时 mutter 报
   `meta_evdev_button_to_clutter: assertion 'evdev_button > BTN_LEFT' failed`；
   必须传 `0x110`(BTN_LEFT)/`0x111`/`0x112`。
2. **会检查 press/release 配对，且状态按 D-Bus 连接计**：另开一个连接只发 release 会得到
   `Invalid key event`。所以注入端必须是**一条长连接 + 一个会话**（`husk-rd rd` 就是这样做的）。

## 实现要点（`runtime/crates/husk-rd`）

- GIO 通过 `dlopen("/usr/lib/aarch64-linux-gnu/libgio-2.0.so.0")` 取符号，**不链接**
  libgio：交叉编译时 sysroot 里没有 rootfs 的 .so，dlopen 走 guest 自己的 loader/shim 路径，
  和 MediaCodec 那层用的是同一手法。
- 命令：`selftest`（连通性）、`probe`（一个连接里跑完 motion/button/key/axis 全套）、
  `rd <socket>`（生产路径：消费 app 的 evdev 记录并注入）。
- 相对运动由 ABS_X/ABS_Y（0..32767）换算成像素增量后调
  `NotifyPointerMotionRelative`；滚轮 → `NotifyPointerAxisDiscrete`。

## 未完成

- 绝对定位（点哪指哪）需要 ScreenCast 流做坐标变换，`NotifyPointerMotionAbsolute`
  在 `session->screen_cast_session` 为空时直接报 "No screen cast active"（源码
  `meta-remote-desktop-session.c:961`）。当前用相对运动等价实现，体验稍差。
- 触摸（`CLUTTER_TOUCHSCREEN_DEVICE`）未接：桌面用指针模型即可，但"全硬件"验收里
  触摸要有明确结论。
