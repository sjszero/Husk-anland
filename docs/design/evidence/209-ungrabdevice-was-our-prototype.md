# 209 · `XUngrabDevice` 的 SIGSEGV 是我们自己的原型写错了：把 `XDevice*` 当成了 `int`

日期：2026-11-25 19:30–19:45（CST）。设备 RMX3357，`:0`，0.0.7 会话
（`CHECK wm=yes / ding=yes / gpu=zink / input=primed`）。这一份给 208 那个悬案结案。

## 1 · 最小复现（FACT）

`tools/device-tests/input/ungrabmin.c`，四种模式，同一棵树同一会话：

| 模式 | 调用 | 输出 |
|---|---|---|
| `xi` | `XIUngrabDevice(dpy, 2, 0)` | `rc=0`，`XSync`、`XCloseDisplay` 都过 |
| `core` | `XUngrabDevice(dpy, 0)`（按 2 参数原型调用） | **`segv\|sig=11`，退出码 139** |
| `core-pt` | `XUngrabPointer(dpy, 0)` | `rc=1`（GrabNotActive），不崩 |
| `xi-grab` | `XIGrabDevice(root)` → `XIUngrabDevice` | `rc=0`，六个步骤全过 |

`core` 崩溃发生在第一次请求上：`XOpenDisplay` 之后、还没有任何抓取，直接 `XUngrabDevice`。

## 2 · 为什么崩（FACT）

X.Org 的 `X11/extensions/XInput.h`（XInput-1 的 API，实现在 libXi）：

```c
extern int XUngrabDevice(Display* /* display */, XDevice* /* device */, Time /* time */);
extern int XIUngrabDevice(Display* /* display */, int /* deviceid */, Time /* time */);  /* XI2 */
```

`XUngrabDevice` 的第二个参数是 **`XDevice` 指针**，第三个是 `Time`。`xi2probe.c` 原先写的是
`extern int XUngrabDevice(Display *, int, Time);`，再传 `XUngrabDevice(d, 2, 0)`——libXi 把 `2`
当地址去解引用 `dev->device_id`，于是 SIGSEGV。

对照：`XUngrabPointer(Display*, Time)` 是核心协议、2 个参数，同一台 server 上正常返回 1
（`GrabNotActive`），说明核心协议的解除抓取没有坏。

## 3 · 修法（FACT）

- `tools/device-tests/input/xi2probe.c`：改为 `XIUngrabDevice(d, 2, 0)`。修改后 `grab 模式`
  完整跑完：`XIGrabDevice ret=0` → `RESULT mode=grab core_motion=0 …` → `rc=0`；修改前是同一条
  命令在 ungrab 处 `segv`、`rc=139`。
- 新增 `tools/device-tests/input/ungrabmin.c`：只做 `XOpenDisplay` → 一次解除抓取 → `XSync`，
  四种模式可分别复跑，用来区分「核心协议 / XInput-1 / XI2」三条路。

## 4 · 更正 208

208 §2 列的三种可能（server 侧、探针 ABI、path-shim 副作用）里，答案是第二种，而且比
「调用约定不一致」更具体：**调的不是同一个函数**。server、libXi、path-shim 都没有问题。

208 §1 表里的「崩点在 `XIUngrabDevice` 这一步」作废：那个程序从来没调用过 `XIUngrabDevice`。
208 §3 计划的最小程序已经写完并跑过，见本文 §1。

## 5 · 与 0.0.7 的拖动结论无关

拖动修复（evidence 206）走的是 `xdotool` 的 XTEST 通路，不经过这两个函数；这一条改的只是
探针收尾那一步。修前修后 `xi2probe grab` 收到的 motion 数都是 0（本次没注入），差别只在
退出码与是否打印 `RESULT`。

## 6 · 复现

```sh
# 在设备上（xe-plain.sh 是 guest 运行器）
sh $F/xe-plain.sh usr/bin/env DISPLAY=:0 HOME=/root XDG_RUNTIME_DIR=/tmp/xdg-husk \
  usr/bin/gcc -O0 -o tmp/ungrabmin tmp/ungrabmin.c \
  usr/lib/aarch64-linux-gnu/libXi.so.6 usr/lib/aarch64-linux-gnu/libX11.so.6
sh $F/xe-plain.sh usr/bin/env DISPLAY=:0 HOME=/root XDG_RUNTIME_DIR=/tmp/xdg-husk \
  tmp/ungrabmin xi        # 期望 rc=0
```
