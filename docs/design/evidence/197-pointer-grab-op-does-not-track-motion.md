# 197 · 拖窗根因定到 grab op 类型：指针发起的移动抓取不跟 motion，键盘发起的跟

日期：2026-11-25 15:43–16:00（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4.19.191 / 4 KiB），应用 `com.linuxhub.myth.husk`，X server `:0`，`sess-force.sh` 会话
（发布版加载器 `libld.so` + `/etc/husk/gpu-env` zink，`MUTTER_DEBUG=window-ops,focus`）。
X root：`2400x1080`（`xdpyinfo`）。

196 只压到"抓取成立、键盘能动、指针 motion 不生效"。这一份把剩下的两半分开：
**谁在抓、抓的时候 motion 到底能不能到、以及哪一条抓取路径能让窗口真的动。**

## 1 · 先证明生产输入通路本身是通的（FACT）

`husk-rd`（`runtime/crates/husk-rd`，本次新加 `seq` / 绝对 motion 支持）在 guest 里跑，
用 `org.gnome.Mutter.RemoteDesktop` 注入，X 客户端侧 `xev` 自己记到：

```
MotionNotify=3  ButtonPress=1  ButtonRelease=1
```

⇒ RemoteDesktop 注入的按钮和 motion 都能穿过 mutter 落到 X 客户端上，不是"注入没用"。

## 2 · 但相对 motion 只动一个轴（FACT）

每条都先 `xdotool mousemove 500 400` 归零，再注入一次，再读 `getmouselocation`：

| 注入 | 指针落点 |
|---|---|
| `NotifyPointerMotionRelative(100, 0)` | `x:500 y:500` |
| `NotifyPointerMotionRelative(0, 100)` | `x:500 y:400`（没动） |
| `NotifyPointerMotionRelative(-100, 0)` | `x:500 y:300` |
| `NotifyPointerMotionRelative(0, -100)` | `x:500 y:400`（没动） |

⇒ 只有第一个参数生效，而且作用在 **y** 上；第二个参数被丢掉。也就是说走这条路
**指针只能上下动，横向永远动不了**。这是一条独立的缺陷，和窗口无关。

## 3 · 绝对 motion 是空操作（FACT）

`NotifyPointerMotionAbsolute(x, y)` 调用成功（没有 D-Bus 错误），但 `p:600:200`、
`p:1800:800`、`p:200:1000` 三次之后指针仍在 `0,0`。触摸屏语义要的正是绝对坐标，
这条暂时用不了。

## 4 · 按住标题栏时，指针确实被别人抓着（FACT）

`tmp/grabstatus`（`XIGrabDevice(dev=2, root)`，返回 0=Success 1=AlreadyGrabbed）：

| 时机 | 返回 |
|---|---|
| 空闲（指针在窗口内） | 0（抓得到，随后探针自己 segv） |
| 按住标题栏期间（两次采样） | 1 = AlreadyGrabbed |

⇒ 拖拽期间 mutter 是**真的**发起了指针抓取，不是"没开始搬窗"。

## 5 · 关键对照：alt+F7 之后动指针，窗口真的会走（FACT）

| 操作 | 窗口位置 |
|---|---|
| 起点 | `120,150` |
| `alt+F7` → 只动指针（800,500 → 1100,740）→ `Return` | `689,425` |
| 再 `alt+F7` → 8 次 Right+Down → `Return` | `759,495` |

而同样的起点、同样按住标题栏，三种注入方式全部纹丝不动：

| 注入方式 | 指针 | 窗口 |
|---|---|---|
| XTEST 相对（`mousedown` + `mousemove_relative`） | `531,170 → 531,330` | `120,150` 不变 |
| `XWarpPointer` 绝对（`mousedown` + `mousemove`） | `531,170 → 771,430` | `120,150` 不变 |
| RemoteDesktop 相对 | `531,170 → 531,330` | `120,150` 不变 |

另外单独验证 mutter **不是**看不到指针：指针移到左上角 `2,2` 时热角触发，
截屏 `a` 与 `b` 哈希不同，移回 `1200,600` 后与 `a` 完全一致（`sha1` 相同）。
⇒ 全局 motion 有到 mutter，只是"抓取期间的 motion"没有作用到窗口上。

## 6 · 结论（INFERENCE，FACT 支撑见上）

- **指针发起的移动抓取（标题栏拖、`_NET_WM_MOVERESIZE`）不跟随 motion。**
- **键盘发起的移动抓取（`alt+F7`）跟随 motion，并且能落位。**
- 两者唯一的区别：alt+F7 只抓键盘，指针仍是"未抓取"的自由指针；标题栏拖会把
  指针本身抓走，之后 motion 要么没到 mutter、要么到了没被用。

`MUTTER_DEBUG=window-ops,focus` 的日志里，整个拖拽过程只有 `WINDOW_OPS: ... has_move = 1`
这类属性行，没有任何 grab op 记录（46 的这个调试域只打装饰属性），所以日志不能用来
继续二分。

## 7 · 还没定死的一步（UNKNOWN）

motion 丢在哪一侧，还差一个实验：

1. 抓取方是 mutter，第三方客户端在抓取期间抓不到指针（`AlreadyGrabbed`），所以只能
   在"我方持有抓取"的条件下量投递——`tmp/grabrecv`（抓 root 后数 `XI_Motion`）目前
   读到 `grab-status=1`、`motion=0`，但**空闲态同样读到 1**，与 `grabstatus` 的空闲态
   返回 0 矛盾，怀疑 `XISelectEvents` 与 `XIGrabDevice` 用同一份 mask 时被服务器判成
   占用了自己，探针需要改写（先 grab、后 select），这一条**不作数**。
2. 待做：改成"先 grab 再 select"，若抓取期间仍收不到 `XI_Motion`，则问题在 X server 侧
   （应用内 bionic Xorg 的 XI2 设备抓取不投递 motion）；若收得到，则问题在 mutter 侧
   （grab op 里的 motion 被丢）。

## 8 · 可落地的一条替代通路（已在真机验证的机制，未实现）

既然"alt+F7 + 指针 motion"是通的，拖窗可以在运行时输入层自己实现，不动应用、不动合成器：

1. 输入层（`husk-rd`）判定按下点落在窗口标题栏区域（`XQueryPointer` 命中客户窗口且
   局部 y < 标题栏高度）；
2. **吞掉这次按下**，不转发给 mutter（避免它发起不跟随 motion 的指针抓取）；
3. 注入 `alt+F7`，把后续 motion 照常喂给 mutter，松手时注入 `Return` 落位；
4. 落位后补一次 `_NET_ACTIVE_WINDOW`，把"点标题栏顺带激活窗口"的语义补回来。

代价：标题栏上的双击最大化、右键菜单要单独处理（或按位置放行给 mutter）。

## 9 · 本次动过的东西（可回退）

- `runtime/crates/husk-rd`：新增 `seq` 模式（一次会话内跑完整手势）和 `p:x:y` 绝对
  motion 操作；`m:x:y` 仍然只走相对注入。没有改动任何既有分支行为。
- 设备侧脚本（`files/linux/`）：`drag-rd.sh`、`drag2.sh`、`drag3.sh`、`ptr.sh`、
  `axis.sh`、`abs.sh`、`grabtest.sh`、`warptest.sh`、`f7test.sh`、`wkaround.sh`、
  `hotcorner.sh`、`grabchk.sh`、`grabrecv.sh`、`rdname.sh`、`probe2.sh`、`dbgrun.sh`。
- 探针源码：`tmp/grabstatus.c`、`tmp/grabrecv.c`（已收进 `tools/device-tests/input/`）。
- 浏览器/桌面状态未改：壁纸、主题、`gsettings` 全程未动。
