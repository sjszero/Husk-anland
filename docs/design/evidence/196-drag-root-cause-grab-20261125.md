# 196 · 窗口拖不动：抓取成立、键盘能动、指针 motion 进不了 mutter

日期：2026-11-25 15:24–15:37（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4.19.191 / 4 KiB），应用 `com.linuxhub.myth.husk`，X server `:0`，zink 会话，
`gnome-shell`（`--x11 --replace --mode=linuxhub-ubuntu-min`）。

这一份只做一件事：把"窗口拖不动"压到**一层**。全部靠命令行取值，不靠看图。

## 1 · 先确认 WM 是活的（FACT）

| 探针 | 结果 |
|---|---|
| `xprop -root _NET_SUPPORTING_WM_CHECK` → `_NET_WM_NAME` | `GNOME Shell` |
| `_NET_SUPPORTED` 里有没有 `_NET_WM_MOVERESIZE` | 有 |
| `alt+Tab` | `_NET_ACTIVE_WINDOW` 从 `0x2400001` 变成 `0x1e00001`（真的切换了） |
| 带框（reparent） | 客户窗口 `0x2400001` 在 frame `0x1600011` 里：frame 1059,415 348x286 / client 1073,464 320x220 |

⇒ 会话里有一个**活着且在干活**的 mutter，不是"没人当 WM"。

## 2 · 客户端 input 是通的（FACT）

`xev` 窗口里用 XTEST 点一下、再动一下，`xev` 自己记到：

```
ButtonPress 1   ButtonRelease 1   EnterNotify 1   MotionNotify 2
```

⇒ 指针事件能正常投递到客户端窗口。不是"点不进去"。

## 3 · 三条发起路径，全部不动（FACT）

| 路径 | 怎么发起的 | 结果 |
|---|---|---|
| 客户端请求 | 自己写的 `movetest`：按下时向 root 发 `_NET_WM_MOVERESIZE`（MOVE / button 1 / source=1），stdout 打出 `sent _NET_WM_MOVERESIZE at 1233,558` | 窗口不动（1073,464 原地） |
| WM 自己的标题栏 | 按住 frame 的标题栏区域再拖 | 不动 |
| 键盘 move op | `alt+F7` 之后动指针 | 不动 |

## 4 · 但抓取是成功的，键盘还能推着窗口走（FACT，这次是决定性一条）

`LD_PRELOAD` 探针（钩 `XIGrabDevice` / `XGrabPointer` / `XGrabKeyboard`，并把 mutter 自己装的
X error handler 包一层）在 shell 进程里记到：

```
PID=12499 XIGrabDevice dev=2 win=0x1200005 own=0 mask_len=5
PID=12499 XIGrabDevice ret=0 dev=2        ← dev 2 = Virtual core pointer
PID=12499 XIGrabDevice dev=3 win=0x1200005 own=0 mask_len=5
PID=12499 XIGrabDevice ret=0 dev=3        ← dev 3 = Virtual core keyboard
```

也就是说 `alt+F7` 之后 mutter **已经拿到了指针+键盘的设备抓取**。紧接着：

| 动作 | 窗口位置 |
|---|---|
| 起始 | 1073,464 |
| `alt+F7` 后指针移到 1400,700 → 1600,820 | 1073,464（一点没动） |
| `alt+F7` 后连按 20 次 Right + 10 次 Down + Return | **1250,551**（真的动了） |

⇒ **FACT：grab op 起来了、抓取拿到了、键盘事件进得去、窗口移动的执行也是好的；
只有"抓取期间的指针 motion"没有生效。**

顺带把库层也钉了：`libmutter-14.so.0` 的动态符号里只有 `XIGrabDevice`
（没有 `XGrabPointer` / `XGrabKeyboard`），所以 mutter 在 X11 上抓指针就是走 XI2。

## 5 · 现在的判断（一个 FACT，一个 UNKNOWN）

- FACT：故障点在"grab 期间的指针 motion 传递/处理"，不在发起、不在抓取、不在几何计算。
- UNKNOWN：是 **X server 侧**（应用自带的那个 Xorg 在"设备被抓取"时没有把 master pointer 的
  XI2 motion 投给抓取者），还是 **mutter 侧**（收到了但按 deviceid 过滤掉了）。

下一刀（已经定好，不需要看图）：写一个自己的 XI2 客户端 —— `XISelectEvents` 选
`XI_Motion` → `XIGrabDevice(dev=2)` → 由 XTEST 产生 motion —— 看它**自己**能不能收到
`XI_Motion`。收到就是 mutter 侧的问题；收不到就是 X server 的 XI2 合成问题（那就要找
应用侧的 X server，不是我们这层能补的，得给 2.0 侧一个明确的缺陷报告）。

## 6 · 顺带查清的两件事（FACT）

1. **生产输入链现在根本没跑**：evidence 98 记的链路是
   `app → run/husk/input.sock → husk-rd rd → org.gnome.Mutter.RemoteDesktop → Clutter 虚拟设备`。
   当前这台设备上：`husk-rd` 二进制不存在、`/run/husk/input.sock` 不存在、进程里没有
   `husk-rd`。所以这一轮我量的全是 XTEST→X server 这条路；生产的
   RemoteDesktop 那条路要先把 `husk-rd` 装进 rootfs 才能量。
2. X server 里有 7 个输入设备：`Virtual core pointer/keyboard`、`Virtual core XTEST
   pointer/keyboard`、`Lorie mouse` / `Lorie touch` / `Lorie keyboard`。
   即"真实触摸"在 X 里是以 Lorie 这几个 slave 出现的，和 XTEST 不是同一个 slave。

## 7 · 这一轮把自己弄脏的地方（记下来，别再犯）

- 诊断壁纸时反复改 `picture-uri`（纯色绿 → 白底网格 → 0..255 渐变 → 预缩放图），
  用户看到的就是"壁纸老是变样"。收尾已清：临时图全删，
  `picture-uri` 只剩 `~/.cache/husk/wallpaper-2400x1080.png`（贴合版），
  `primary-color` 从测试用的 `#00ff00` 复位成 `#023c88`。
- 会话是我手工用 `sess-host.sh` 反复起停的，中途还留下过重复的
  `colord / accounts-daemon / upowerd / dbus-daemon`。0.0.6 要把"起会话"收敛成一个
  确定入口（先清残留、再起、起来后自检），不能靠人手。
