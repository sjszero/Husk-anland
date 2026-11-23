# evidence 95 — 输入桥：App 手势/按键 → guest evdev 记录（头一次跑通）

设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB；guest = `untrusted_app`（uid u0_a302）。
日期 2026-11-23。

## 结论（FACT）

`adb shell input tap / keyevent / swipe` 三类事件全部按契约到达 guest：

```
input|header|INPUT w=1080 h=2400 max=32767        ← 连接时的 surface 尺寸
input|ev|ABS|ABS_X|0        ← 连接瞬间的状态同步（指针/按键）
input|ev|ABS|ABS_Y|0
input|ev|ABS|ABS_X|16398    ← tap 540,1200 → 归一化 0..32767
input|ev|ABS|ABS_Y|16390
input|ev|KEY|BTN_LEFT|1
input|ev|KEY|BTN_LEFT|0
input|ev|KEY|KEY_A|1        ← keyevent KEYCODE_A → evdev 30
input|ev|KEY|KEY_A|0
input|ev|ABS|ABS_X|15183    ← swipe 500..900,1600 → DOWN 带 BTN_LEFT=1
input|ev|ABS|ABS_Y|21853
input|ev|KEY|BTN_LEFT|1
input|ev|ABS|ABS_X|15609 …  ← 拖动期间连续 ABS 移动
input|done|24
```

语义与先人一致（`input-prior-art.md`）：点 = 左键按下+抬起、拖 = 左键按住移动、
双指上下 = `REL_WHEEL`、长按 500 ms = 右键。手势在 app 侧识别，跨进程只传结果。

## 修掉的三个真 bug（都是"看起来像没事件"的根因）

1. **guest 工具绕过 `BufReader`**：`read_line()` 读头之后用 `r.get_mut()` 直接读裸
   socket，把 `BufReader` 已缓冲的头几条记录丢掉；没有缓冲数据时就永久阻塞。
   改成 `read_full(&mut r, …)`。这一条解释了"连上了但一条事件都没有"。
2. **app 不知道客户端已经退出**：事件被写进死 socket，永远不到新客户端。
   加 `poll(fd, POLLIN|POLLERR|POLLHUP, 0)` 在每个写之前判活；已取出的那条事件
   `carry` 给下一个客户端，不丢。
3. **socket 不是 evdev**：客户端不在时产生的记录会消失。改为**连接即同步**：
   头之后先发当前指针位置与按键状态（`state_sync()`），客户端不需要靠"从第一条
   事件开始"来推断状态。

## 为什么不是 `/dev/input` + libinput

见 `anland-deep-read.md`：anland（GNOME/KDE 在 Android 上的成熟实现）与 DroidNative
都把输入注入到**合成器**，没有一家走 evdev。我们下一步的注入端是 XTEST（X11 桌面）
或 RemoteDesktop/EIS（mutter Wayland），所以这层 evdev 记录只是**传输格式**，
不假装成内核设备。

## 未完成

- 注入端未接：guest 里还没有 X server / mutter 会话，所以记录目前只到"解码"这一步。
- 触摸多指、平板笔、滚轮离散量已实现但未逐一上机验证。
- 终端自身的键盘输入仍走 PTY（与这条桌面输入链路并行，互不影响）。
