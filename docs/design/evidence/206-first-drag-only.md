# 206 · 「拖不动窗」的真相：只有会话里的**第一次**指针拖动失效，第二次起就好；已在运行时层预热修掉

日期：2026-11-25 17:20–17:40（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`，zink 会话。

**这一份推翻 197 / 198 / 203 / 205 的结论**：那四份都在测「新起会话里的第一次拖动」，
于是把「第一次失效」当成了「拖动完全失效」。下面是 A/B 实测。

## 1 · 关键对照（FACT）

`husk-desktop.sh` 起会话 → 立刻 `drag7.sh`（每次开一个新的 xev 窗口，`Super`+按住拖 300×200）：

| 会话 | 第 1 次拖动 | 第 2 次拖动 | 第 3 次拖动 |
|---|---|---|---|
| fresh.log（起会话后立刻） | `236,188` **不动** | 45 s 后 `536,388`（+300,+200） | 90 s 后 `536,388` |
| fresh2.log（两次连着做） | `236,188` **不动** | **紧接着** `536,388` | — |

⇒ 失效的是**会话内的第一次 grab op**，不是"拖动"这个功能；也不是时间问题
（第二次紧接着做就好了），更不是设备问题（同一个 XTEST 指针，第二次就能拖）。

## 2 · 第一次拖动时 mutter 自己在干嘛（FACT）

`MUTTER_DEBUG=window-ops` 起会话，第一次拖动的 shell 日志：

```
17:25:19.172  WINDOW_OPS: Doing grab op 1281 on window 0x1200001 pointer pos 550,468
17:25:19.186  WINDOW_OPS: Grab op 1281 on window 0x1200001 successful
17:25:20.448  WINDOW_OPS: Computing edges to resist-movement or snap-to for 0x1200001.
17:25:21.486  WINDOW_OPS: Ending grab op 1281
```

读法：抓取**成功**、指针位置**正确**、motion **到了**（否则不会去算边缘），但位置一次都没更新。
⇒ 不是"收不到事件"，是这一次 grab op 里的位置更新没有被应用。

## 3 · 事件层已经全部排除（FACT，本轮重测）

| 项 | 结果 |
|---|---|
| 核心 `XGrabPointer`（窗口级 / root 级）+ XTEST motion | 抓取 `Success`，抓取期间两次 motion **全收到**（`motion=2`） |
| `XISelectEvents`（窗口 / root） | 返回 **0** |
| `XIGrabDevice(root)` + XI2 事件 | 抓到 `ret=0`，收到 **10 个 XI_Motion + 1 个 button** |
| XI2 motion 里的坐标 | payload 里 `root_x/root_y` = 注入值本身（400,300 → 520,370 → 640,440），`event_x/event_y` 也合理 |

（顺带：205 初版说"XI2 坏了"是**错的**，那是我们临时探针把 `XISelectEvents` 的请求构造错了；
仓库里已有的 `xi2probe.c` 重测是 rc=0、事件到手。205 已改。）

## 4 · 修法：进会话后做一次空点击 + 8 px 空拖动（FACT）

`husk-desktop.sh` 加第 5b 步「输入预热」：屏幕中下部（`W/2, H*3/4`，避开左侧 dock 和 DING 图标列）
一次 `click`、一次 `mousedown → +8px → mouseup`，然后打 `CHECK input=primed`。

实测（fresh3.log，起会话后**立刻**拖，不做任何手工预热）：

```
CHECK wm=yes / CHECK ding=yes / CHECK gpu=zink / CHECK input=primed
-- drag#1 right after session --
geo_before: 236 188 628 566
geo_after_super_drag: 536 388 628 566     ← 第一次就动了
```

对照组（prime.log，手工做同样的空点击+空拖动再拖）：同样第一次就动。

## 5 · 顺带把「对齐 / 吸附」也验了（FACT）

拖到屏幕左边缘松手（`snap.sh`）：

```
before: 286 188 628 566
after_drag_to_left_edge: 46 12 1207 1088       ← 左半屏平铺（含阴影边距）
```

⇒ 边缘平铺（edge-tiling）是好的，`46,12 / 1207x1088` 就是 2400×1080 的左半屏。

## 6 · 还没解释的部分（UNKNOWN，不猜）

为什么**只有第一次** grab op 的位置更新不生效——是 mutter 第一次算边缘阻力时用了还没建好的
work area，还是 X server 在第一次 ConfigureWindow 前还没准备好——没有证据，先按"预热可绕过"处理。
预热不是修根因，是把一次已知会失效的操作提前消耗掉；根因留给之后要碰 mutter 的时候再定。

## 7 · 复现

```sh
sh $R/usr/local/share/linuxhub/husk-desktop.sh $R          # 起会话（带预热）
sh tools/device-tests/input/drag-window.sh $R $LD          # 第一次就应位移
sh files/linux/snap.sh                                     # 拖到左边缘应平铺成 1207x1088
```
