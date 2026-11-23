# 输入这块的先人工作：Lorie / DroidNative / libevdev

日期：2026-11-23（按用户要求先读再做）

## 一句话

两个做得最深的实现（我们要替换的 LinuxHub 2.0、以及 DroidNative）都**不往桌面里
塞原始多点触控**：它们把手指手势翻译成"鼠标指针 + 按钮 + 滚轮"，再交给桌面。原因很
实际——在 6 寸屏上跑桌面，指针模型比触摸模型好用得多，而桌面（X11/GNOME）本来就把
指针当第一公民。

## 1. LinuxHub 2.0（Lorie，我们要替换的东西）

`termux_x11_source/lorie/src/main/java/com/termux/x11/input/`：

| 文件 | 行 | 作用 |
|---|---:|---|
| `TouchInputHandler.java` | 1184 | 手势状态机：点击/拖动/长按/双指/滚轮 |
| `InputEventSender.java` | 265 | 把结果转成 X11 事件（XTEST 风格） |
| `TapGestureDetector.java` | 214 | 单击/双击/三指 |
| `SwipeDetector.java` | 126 | 滑动阈值与速度 |
| `EmbeddedInputController.java` | 72 | 把 App 的 View 接到手势管线 |

两种模式（`MouseMode`）：**MOUSE**（手指即指针：点=左键、拖=左键拖、长按=右键）与
**TOUCHPAD**（相对移动的虚拟光标：点=左键、点多再拖=按住左键、双指=右键、三指=中键；
双指移动=滚轮）。这是这一代 Android 桌面容器的标准交互契约。

## 2. DroidNativeApp

`app/src/main/kotlin/app/droidnative/display/TouchInput.kt`（262 行）+ `ToplevelInput.kt`。
同样是 MOUSE/TOUCHPAD 两模式、同样的长按右键与双指滚轮，只是把它送进**它自己的
Wayland 合成器**（`app/src/main/cpp/compositor/`，带 text-input-unstable-v3），
因为它的 App 自己就是合成器。我们不是这个架构：我们让 guest 里的 mutter/libinput
当合成器，所以必须走内核的输入接口（evdev）。

## 3. 内核 ABI

`struct input_event`（64 位）= `timeval(8+8) + type(u16) + code(u16) + value(i32)` = **24 字节**；
`EV_SYN=0 / SYN_REPORT=0`、`EV_KEY=1`、`EV_REL=2`、`EV_ABS=3`；
`ABS_X=0 / ABS_Y=1`、`ABS_MT_*`、`BTN_LEFT=0x110`、`REL_WHEEL=8`；
键盘用 Linux keycode（`KEY_A=30`、`KEY_ENTER=28`、`KEY_BACKSPACE=14`…），与 Android
`AKEYCODE_*` 是两套编号，必须查表。

## 3.5 Anland（`SuperTurtleDev/anland`）——第三个先人，也是结论最强的

详见 [`anland-deep-read.md`](anland-deep-read.md)。要点：

- GNOME/KDE 桌面在 Android 上跑通的现成实现；输入从 App 直接注入**合成器的 seat**
  （`clutter_virtual_input_device_notify_button`、
  `meta_virtual_input_device_native_notify_absolute_relative_motion`），
  **不经过 `/dev/input`、不经过 libinput、不经过 udev**。
- 协议里输入是 20 字节定长记录（`common/protocol.h:98`）：`TOUCH / KEY / POINTER_MOTION /
  POINTER_BUTTON / POINTER_AXIS / TOUCH_FRAME`，坐标 float，按键用 Linux evdev 码。
- 手势语义与我们 §1、§2 记的 MOUSE/TOUCHPAD 契约一致，说明这是这一代产品的共识。

⇒ 结论：内核 evdev 那条路只在"要给读 `/dev/input` 的老程序"时才需要；主路径是
**合成器级注入**。X11 桌面用 XTEST，Wayland/mutter 用 EIS（libei）。

## 4. Husk 的决定（2026-11-23 二次修订，看过 anland 后）

1. **手势识别在 app 侧**（点=左键、拖=左键拖、长按=右键、双指=滚轮、双指移动=滚轮），
   与 Lorie / DroidNative / anland 三家一致，跨进程只传结果。
2. **注入点在合成器，不在内核**：
   - X11 桌面 → `XTEST FakeInput`（纯 Rust 实现 X11 协议子集）；
   - Wayland/mutter → `org.gnome.Mutter.RemoteDesktop` 拿 ei fd，走 libei 协议。
   原计划的"FIFO 假装 /dev/input/eventN 让 libinput 消费"**撤销**：libinput 还要 udev
   枚举、`/sys/class/input` 目录和 `EVIOCG*` 查询，代价高且三家先人都绕开了它。
3. 只有确认为某个 guest 程序必须直读 `/dev/input/event*` 时，才补一层最小 evdev 接口，
   且用 FIFO（保证 `poll/epoll` 语义），不用 memfd。
4. 传输仍是 `run/husk/input.sock`；记录格式换成 anland 风格的定长 20 字节 + 类型枚举，
   按键用 evdev 码（与注入端一致，省一层翻译）。
