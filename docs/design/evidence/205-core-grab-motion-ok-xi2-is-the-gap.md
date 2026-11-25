# 205 · 拖动缺陷再压一层：抓取和核心 motion 投递都正常，缺口在 XI2 事件路径

日期：2026-11-25 17:25–17:35（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`。

这一份把 197/198/203 的结论再往下压一层。197 说"指针抓取期间 motion 不生效"，
但没区分**是谁的 motion 不生效**。这次用一个纯 core X11 的探针把它分开。

## 1 · 结果（FACT）

`tmp/grabmot.c`：`XGrabPointer` + `XTestFakeMotionEvent` ×2 + 数事件。
四轮交替（不抓 / 抓 / 不抓 / 抓），`PointerMotionMask | ButtonMotionMask | ButtonPressMask`：

```
round=1 grab=0 grab_rc=-2(?)          motion=0 other=0
round=2 grab=1 grab_rc=0(Success)     motion=2 other=0
round=3 grab=0 grab_rc=-2(?)          motion=0 other=0
round=4 grab=1 grab_rc=0(Success)     motion=2 other=0
```

读法：

- **抓取成功**（`Success`，不是 `AlreadyGrabbed`）；
- **抓取期间两次 XTEST motion 全部收到**（`motion=2`）；
- 不抓的时候 `motion=0` 是正常的：指针在窗口外，core motion 只投给指针下的窗口，
  本来就不该给这个客户端。

⇒ **X server 的核心抓取与 motion 投递都是好的**。197 说的"抓取期间 motion 不生效"，
不是核心事件通路的问题。

## 2 · 那缺口在哪（INFERENCE，有旁证）

mutter 46 的 X11 后端读输入走 **XI2 设备事件**（`XISelectEvents` + `XIGrabDevice`），
不走核心 motion。而这台 server 的 XI2 是残的——198 已经实测：
`XIQueryDevice(XIAllDevices=3)` 只返回 1 个设备（键盘），`XIAllMasterDevices` 才返回 2 个；
只有 XI 2.0，没有 2.1。

再叠上这一条：`XISelectEvents(root, XI_Motion 掩码)` 在这台 server 上直接回
`BadValue (XInputExtension minor 46)`，连"选择事件"这一步都过不去。

⇒ 最简解释：**mutter 用 XI2 读指针，而 XI2 的枚举/选择在这台 server 上是坏的**，
所以它自己的 grab op 期间永远收不到 motion；核心抓取的客户端反而一切正常。
这解释了为什么"客户端按住拖正常、mutter 发起的搬窗不动、`alt+F7` 却能搬"——
`alt+F7` 之后的移动是 mutter 用**键盘**事件驱动的，不依赖指针 motion。

## 3 · 对生产输入的含义（UNKNOWN，必须说清）

生产手指走的是应用侧创建的 `Lorie mouse/touch/keyboard`（198：设备 6/7/8），
和自动化的 XTEST pointer 不是同一条设备。若 XI2 对**真实设备**的投递是好的、只有
XTEST/合成设备那条坏，那"真手指拖窗"可能其实是可用的——**这一点没有验证**，
需要人手上机拖一次。反过来，如果 XI2 对所有设备都坏，那它就必然修在 X server 侧。

## 4 · 给 2.0 侧的最小复现（三步）

```sh
# 1) XI2 枚举与选择
xinput list                      # 期望看到 pointer/touch 设备齐全
XISelectEvents(root, XI_Motion)  # 这条现在直接 BadValue（minor 46）
# 2) 核心对照（应正常）
sh files/linux/.../grabmot        # 打印 grab=1 motion=2 即为"核心通路正常"
# 3) 现象：Super+拖标题栏不动、alt+F7 能动
```

要修的是 **XI2 的 `XISelectEvents`/`XIQueryDevice` 语义**，不是抓取，也不是核心事件。
