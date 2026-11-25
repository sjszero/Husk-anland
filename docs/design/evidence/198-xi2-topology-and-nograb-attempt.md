# 198 · 输入栈的 XI2 拓扑，以及"让 mutter 别抓指针"这条路为什么走不通

日期：2026-11-25 15:59–16:10（CST）。同一台 RMX3357、同一个 `:0`、同一套 `sess-force.sh` /
`sess-nograb.sh` 会话。197 把拖窗压到"指针发起的 grab op 不跟 motion"，这一份回答两件事：
**这台 X server 的输入设备树长什么样**，以及**能不能在 guest 侧把 mutter 的指针抓取变成空操作**。

## 1 · XI2 设备树（FACT）

`tmp/devlist2`（`XIQueryVersion` + `XIQueryDevice`）：

```
XI 2.0
query(XIAllDevices=3)        -> n=1   id=3  use=2 attach=2 enabled=1  Virtual core keyboard
query(XIAllMasterDevices=1)  -> n=2   id=2  use=1 attach=3 enabled=1  Virtual core pointer
                                     id=3  use=2 attach=2 enabled=1  Virtual core keyboard
query(id2=2)                 -> id=2  Virtual core pointer（classes=3）
query(id4=4)                 -> id=4  use=3 attach=2 enabled=1  Virtual core XTEST pointer
```

三点值得记：

1. **只到 XI 2.0**，没有 2.1，所以 `XIChangeHierarchy`（动态加设备）不可用，任何"给容器加一个虚拟触摸设备"的方案在这台 server 上直接没戏。
2. `XIAllDevices` 查询**只返回 1 个设备**（键盘），而 `XIAllMasterDevices` 返回 2 个。这是服务器侧枚举的缺陷：依赖 `XIAllDevices` 的客户端（很多输入工具、部分 mutter 代码路径）会看不到指针。
3. 只有一个 master pointer（id=2）加一个 XTEST pointer slave（id=4）。也就是说**所有真实输入都以 XTEST 事件的形式出现**，没有第二类设备。

## 2 · 不是客户端请求的问题（FACT）

`Super`+拖是 mutter 自己发起的搬窗，完全不经过客户端的 `_NET_WM_MOVERESIZE`：

| 操作 | 窗口位置 |
|---|---|
| 起点（`xdotool windowmove` 放好） | `300,200` |
| `keydown super` → `mousedown 1` → 两次 `mousemove` → `mouseup` → `keyup super` | `300,200`（没动） |

⇒ 客户端那条请求路径可以排除：mutter 自己发起的搬窗同样不动。

## 3 · 被动抓取在这台 server 上永远拿不到（FACT）

`tmp/pbgrab`（`XGrabButton(Button1, AnyModifier, root, owner_events=0, ...)`）：

```
XGrabButton status=1     # 1 = AlreadyGrabbed
```

在完全空闲、没有任何拖拽的时候也是 1。结合 197 里 `XIGrabDevice` 空闲态返回 0、拖拽期返回 1
的读数，这台 server 的抓取状态机本身就不自洽——**不能用"抓到没抓到"来继续二分**。

## 4 · 把 `XIGrabDevice` 换成空操作：预载进 mutter 失败了（FACT，附原文）

思路：如果 mutter 不抓指针，motion 就会按正常路径进它自己的 stage（热角证明这条路是通的），
grab op 的位移计算也就能拿到坐标。为此写了 `tmp/nograb.c`（hook `XIGrabDevice` / `XUngrabDevice`，
只跳过指针设备，键盘照常抓），并尝试三种注入方式：

| 方式 | 结果 |
|---|---|
| `LD_PRELOAD=<rootfs>/tmp/nograb.so` 起 loader | `CANNOT LINK EXECUTABLE "sh": library "libc.so.6" not found: needed by .../nograb.so in namespace (default)` —— 被 bionic 链接器先吃掉 |
| `LD_PRELOAD=/tmp/nograb.so`（guest 路径） | `CANNOT LINK EXECUTABLE "sh": library "/tmp/nograb.so" not found` —— guest 加载器不按 rootfs 翻译 preload 路径 |
| `libld.so --preload tmp/nograb.so`（相对 cwd） | **能加载**：`/proc/19221/maps` 里 4 处 `nograb`；但**完全没有拦截**：`alt+F7`（196 已证明这条路径会调 `XIGrabDevice dev=2/3`）之后 `xigrab.log` 仍是 **0 字节** |

⇒ 我们的加载器的 `--preload` 只把对象映进地址空间，没有把它放进全局符号查找域，
所以对 mutter 里已经绑定的 `XIGrabDevice` 调用不产生 interposition。这条路要成立，
必须先改加载器（我们自己的代码），不是改一句脚本能解决的。

## 5 · 结论

- 拖窗在这台 X server 上不动的直接原因：**mutter 的指针 grab op 拿不到可用的 motion**（197 + 本节第 2 点），
  而抓取状态机本身不自洽（本节第 3 点），所以既不能靠"抓取成功"判断，也不能靠"谁来抓"绕过。
- 从 guest 侧强行让 mutter 不抓指针：**当前加载器做不到**；要么改加载器给 `--preload` 正确的全局作用域，
  要么等 X server 侧修 XI2 设备抓取（应用侧，`com.linuxhub.myth` 的 Xorg）。
- 因此 0.0.6 的输入项按两档交付：**`alt+F7` + 指针 motion 是已验证可用的搬窗操作**（197 第 5 节，
  `120,150 → 689,425`），标题栏拖动列为已知缺陷，并把"加载器 `--preload` 作用域"写成下一步的
  运行时层任务（这条完全在我们自己手里，1–2 小时量级）。

## 6 · 探针与脚本

- 新增源码：`tools/device-tests/input/devlist2.c`（设备树）、`pbgrab.c`（被动抓取状态）、
  `nograb.c`（抓取空操作 shim）。
- 设备侧一次性脚本：`devlist.sh`、`pbgrab.sh`、`nograb-run.sh`、`nograb-run2.sh`、`nr3.sh`、
  `t4.sh`、`t5.sh`、`superdrag.sh`；会话变体 `sess-nograb.sh`、`xe-plain-p.sh`（后者就是
  `xe-plain.sh` 多一个 `--preload`，用来验证加载器行为，不是发布脚本）。
- 桌面状态未改：没有动壁纸、主题、`gsettings`。

## 7 · 追加：客户端级的"按住拖"是正常的（FACT）

既然要判断是服务器丢事件还是 mutter 不用事件，就看一个普通客户端按住拖动能不能收到 motion。
用 RemoteDesktop 注入在 `xev` 窗口上做一次"按下 → 两次移动 → 松开"：

```
MotionNotify=3  ButtonPress=1  ButtonRelease=1
MotionNotify event, serial 35, synthetic NO, window 0x1800001
ButtonPress  event, serial 35, synthetic NO, window 0x1800001
MotionNotify event, serial 35, synthetic NO, window 0x1800001
```

（同一轮还有 `getmouselocation` 的落点变化可对照。）

⇒ **按住按钮期间的 motion，服务器是照常投递给客户端窗口的**。所以"按住拖动"这件事在
协议层没有坏，坏的是 mutter 的 grab op 那一条：它既没拿到、也没用上 motion
（本节 + 第 2 点 `Super`+拖同样不动）。归因从"服务器丢事件"收窄到 **mutter 的指针 grab op**，
而要把它变成空操作，卡在我们加载器 `--preload` 的作用域上（第 4 点）。

后续两件事，按这个顺序：

1. 给加载器 `--preload` 补上全局符号作用域（我们自己的代码，1–2 小时量级），再跑一次
   `nograb` 实验：若拖窗立刻能动，则拖窗在 0.0.6 就能修在运行时层。
2. 若加载器修好后仍然不动，则问题在 mutter 的 X11 grab op 与新设备拓扑的组合上，
   0.0.6 按"缺陷 + `alt+F7` 替代操作"发布，并给 2.0 侧一份 X server 侧的复现步骤。

## 8 · 追加：设备树其实有 7 个，而且"生产输入"和我们测的不是同一个设备（FACT）

`XIQueryDevice(dpy, 0, &n)`（`tmp/devlist3`，在 `:0` 上）：

```
query(0) -> n=7
   id=2  use=1 attach=3  Virtual core pointer
   id=3  use=2 attach=2  Virtual core keyboard
   id=4  use=3 attach=2  Virtual core XTEST pointer
   id=5  use=4 attach=3  Virtual core XTEST keyboard
   id=6  use=3 attach=2  Lorie mouse      classes=7
   id=7  use=3 attach=2  Lorie touch      classes=4
   id=8  use=4 attach=3  Lorie keyboard   classes=1
```

两件事必须更正 198 前面的说法：

1. **设备树不是"只有 2 个"**：`XIQueryDevice(0)` 能拿到完整 7 个；坏掉的是 `XIAllDevices(=3)`
   这个枚举值（只回 1 个）。`XIAllMasterDevices(=1)` 回 2 个是对的。这是服务器枚举的**取值差异**，
   不是能力缺失。
2. **`Lorie mouse` / `Lorie touch` / `Lorie keyboard` 是应用侧（`com.linuxhub.myth` 的
   Xorg）创建的真实 XI2 设备**，也就是手指和触控板的实际来源。我们前面所有拖动实验用的是
   `XTEST pointer(id=4)`（`xdotool`），**和生产的设备不是同一个**。所以"XTEST 拖不动"不能
   直接推出"手指拖不动"，反过来也一样——这一条之前没有分清，现在分清。

## 9 · 真机手指输入没法从 adb 驱动（FACT）

试过用 `adb shell input tap` 打点，再读 `xdotool getmouselocation`：

```
初始： x:2160 y:972
input tap 540 1200  -> x:2160 y:972（没动）
input tap 900 600   -> x:2160 y:972（没动）
```

⇒ 应用是直接读 `/dev/input` 的，`input` 注入的事件不进那条路。**手指/触控板路径只能人工在
机器上验**，命令行能覆盖的只有 XTEST 这一条。

## 10 · 预载实验的正确结论（修正本节第 4、5 点）

- `--preload` 的**符号拦截是成立的**：`tmp/nograb2.c` 预载进 gnome-shell 后，启动期日志里
  拿到 `XIGrabButton SKIP a=2 b=1/2/3` 各 11–18 次、`XIGrabTouchBegin SKIP` 1 次、`XIUngrabButton SKIP`。
  之前"完全没拦截"的判断是错的，原因是**mutter 对指针根本不用 `XIGrabDevice`**，
  它用的是 `XIGrabButton`（被动抓取）+ Clutter 自己的 `clutter_stage_grab`。
- 把三类指针抓取全部跳过（`HUSK_NOGRAB=1`）之后，标题栏拖动**依然不动**，而且拖动期间
  日志里没有任何新的抓取调用 ⇒ 拖窗不动**不是 X 层抓取造成的**，问题在 mutter 的 grab op
  本身拿不到/不用 motion。
- `tmp/mutfix.c`（把 `XIQueryDevice(3)` 改成 `(1)` 并补上 XTEST slave）从未被调用：
  实测 mutter 只用 `XIQueryDevice(0)`（一次，7 个设备）和 `XIQueryDevice(7)`（一次），
  所以"枚举值 3 坏掉"这件事对 mutter 没有影响，这个补丁没有价值，**不再尝试**。

## 11 · 结论（更新）

- 拖窗不动：**mutter 的 grab op 不跟 motion**，与 X 层抓取、与设备枚举无关。
- 生产输入（Lorie mouse/touch）与命令行可测输入（XTEST）是两条设备，最终验收必须人工用手指。
- 下一档可做的运行时层工作不是继续猜 mutter，而是**把"搬窗"做成我们自己的操作**：
  输入层（`husk-rd`，吃应用给的 evdev 流）判定标题栏手势后直接 `_NET_MOVERESIZE_WINDOW`，
  完全不依赖 mutter 的交互式搬窗。这条在 0.0.6 里按增量做，不影响其它三项。
