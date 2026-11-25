# 205 · 拖动缺陷再压一层：事件投递都是好的，缺口收窄到 mutter 的 XIGrabDevice 路径

日期：2026-11-25 17:25–18:05（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`。
**本文件在 17:40 修订过一次**：初版把原因写成"这台 server 的 XI2 选择/枚举是坏的"，
那是错判——错在我们自己的探针。下面是重测之后的版本，初版结论已在 §5 写明作废理由。

197 说"指针抓取期间 motion 不生效"，但没区分**是谁的 motion 不生效**。这一轮把三件事分开测了。

## 1 · 核心抓取 + XTEST motion：正常（FACT）

`tmp/grabmot.c`（只 core API）：`XGrabPointer` + 两次 `XTestFakeMotionEvent` + 数事件，
四轮交替，`PointerMotionMask|ButtonMotionMask|ButtonPressMask`：

```
round=1 grab=0 grab_rc=-2(?)          motion=0 other=0
round=2 grab=1 grab_rc=0(Success)     motion=2 other=0
round=3 grab=0 grab_rc=-2(?)          motion=0 other=0
round=4 grab=1 grab_rc=0(Success)     motion=2 other=0
```

抓在**窗口**上、以及抓在**root** 上（`owner_events=1`）都是这个结果：
抓取返回 `Success`，抓取期间两次 motion **全部收到**。不抓时为 0 是正常的
（指针不在该窗口上）。⇒ **核心抓取与 motion 投递没有缺陷。**

## 2 · XI2 选择与投递：也正常（FACT）

`tools/device-tests/input/xi2probe.c`（按 64 位 ABI 手写结构体）：

```
win=0xe00001 mode=nograb
XISelectEvents rc=0
RESULT mode=nograb core_motion=0 core_button=0 xi_motion=9 xi_button=10 xi_other=0
RESULT mode=nograb core_motion=0 core_button=0 xi_motion=3  xi_button=4  xi_other=0
```

`XISelectEvents` 返回 0，XTEST 的 motion 与 button 都以 **XI2 事件**送到了客户端
（9/10 与 3/4 两次是重叠运行的两份计数）。⇒ **XI2 的 select 与投递都是好的**，
198 说的"`XIQueryDevice(XIAllDevices)` 只回键盘"是**枚举**的缺陷，不等于投递坏。

## 3 · `XIGrabDevice`：探针必崩（FACT，缺陷位置待定）

同一探针切到 `grab` 模式（`XIGrabDevice(dev=2, root, ...)`）：

```
win=0xe00001 mode=grab
XISelectEvents rc=0
XIGrabDevice ret=0 (0=Success 1=AlreadyGrabbed 3=NotViewable)
segv|sig=11|fa=2|pc=... lr=35 x0=0 x1=2 x8=10
```

抓取本身**成功**（`ret=0`），但**收到第一个事件就 SIGSEGV**（`segv` 那行是加载器自己的信号处理器打的）。
对照组：同样的探针在 `nograb` 模式跑满 12 秒、收 9 个 XI2 事件、正常退出；
核心抓取探针（§1）跑完四轮也不崩。⇒ 崩溃点在 `XIGrabDevice` 之后的事件路径上。

补充：早先给 `XIGrabDevice` 传窗口（而不是 root）时返回 `3 = NotViewable`，
本次已把"映射后等 400 ms"补进探针，所以 `ret=0` 是干净的结果，不是时序假象。

## 4 · 结论（INFERENCE，缺口写清楚）

| 已确认正常 | 依据 |
|---|---|
| 核心抓取成立、抓取期间 motion 到手 | §1 |
| XI2 选择成立、XI2 motion/button 到手 | §2 |
| 抓在 root 上同样成立 | §1 的 root 变体 |

于是"`Super`+拖标题栏不动、`alt+F7` 能动"（197/198/203）只剩一个解释：
**mutter 的指针 grab op 这条路径**（它用 `XIGrabDevice` + XI2 事件）——而这条路径恰好是
§3 里让探针崩溃的那条。`alt+F7` 之后的移动由**键盘**驱动，不走 Pointer motion，所以能搬窗。

**这条还不能算定案**：探针崩了，等于"mutter 那条路径上有东西不对"有了强旁证，
但"到底崩在 Xlib/服务端/我们的 shim"没有分开。要分开只有一步：把 §3 的协议交互退化成最小重现
（grab 后只收 button、只收 motion、mask 长度逐个改），看哪一档开始崩。列为 0.0.7 第一项。

## 5 · 初版错在哪（必须留痕）

初版写的是：`XISelectEvents(root, XI_Motion)` 直接 `BadValue (minor 46)`，因此 XI2 坏了。
错在两点：

1. 那次调用用的是我临时写的结构体与 mask（`mask_len=4`、全 0xff），**请求本身就构造错了**，
   `BadValue` 是我们的 bug，不是 server 的；
2. 用仓库里已有的 `xi2probe.c`（正确 ABI）重测，`XISelectEvents` 返回 **0**，事件也真的到手（§2）。

这条错判已经在 LIMITATIONS §10 同步改掉。留在这里的原因是：**探针写错会造出看起来很硬的假结论**，
而"硬结论"是这个仓库最贵的东西。
