# 208 · `XIUngrabDevice` 之后客户端 SIGSEGV（跳过它就好），责任未定

日期：2026-11-25 17:44–17:52（CST）。设备 RMX3357，`:0`，0.0.7 会话。

## 1 · 二分结果（FACT）

`tools/device-tests/input/xi2probe.c`（`grab` 模式：`XISelectEvents` → `XIGrabDevice(dev=2, root)`
→ 收 8 个注入的 XI_Motion → 打印 RESULT → `XIUngrabDevice`）：

| 变体 | 输出 |
|---|---|
| A 正常 | `XIGrabDevice ret=0` 之后 **segv\|sig=11**，看不到 RESULT |
| B `NO_UNGRAB=1`（跳过 `XIUngrabDevice`） | `RESULT mode=grab ...` 正常打印，退出码 0 |

⇒ 崩点在 `XIUngrabDevice` 这一步；抓取、事件投递、事件计数都正常。
（A 看不到 RESULT 只是 stdout 块缓冲没刷出去——RESULT 在崩溃点之前就 printf 了。）

## 2 · 谁的责任（UNKNOWN）

三种可能，都没排掉：

1. 这台 server 对 `XIUngrabDevice` 的回复/事件顺序有问题，libXi 拿到已释放的东西；
2. 我们手写 ABI 的调用约定与 libXi 的实际实现不一致（探针自己的问题）；
3. path-shim 在解除抓取这条路径上有副作用。

反证只到这一步：**mutter 每个 grab op 结束都会 ungrab，而 shell 不崩**（evidence 206 §2 的日志里
`Ending grab op 1281` 之后 shell 继续跑）——所以"凡是 ungrab 就崩"不成立，更像探针侧的问题。

## 3 · 下一步（0.0.8 第一项）

写一个只做 `XOpenDisplay` + `XIUngrabDevice(dev=2, 0)` + `XSync` 的最小程序：
- 若它也崩 → 责任在 server/libXi，与抓取无关，报给 2.0 侧；
- 若不崩 → 二分抓取参数（mask 长度、owner_events、grab 窗口），定位到具体请求。

## 4 · 这一条不影响 0.0.7 的结论

拖动已经在 0.0.7 修掉并验证（evidence 206），本文只是把"探针为什么不收尾"这件事记清楚，
避免下次有人看到 segv 又以为拖动没修好。
