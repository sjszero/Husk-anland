# 207 · Nautilus 46：内容区**不是**空白，出窗约 15 秒；「点了没反应」是旧会话残留实例

日期：2026-11-25 17:36–17:42（CST）。设备 RMX3357，`:0`，0.0.7 会话（`CHECK wm=yes ding=yes
gpu=zink input=primed`）。这一份给 0.0.6 目标里的第 ③ 项收尾。

## 1 · 内容区到底空不空（FACT）

清掉所有旧实例 → `nautilus --new-window /root/vtest`（目录里放了 `aaa.txt` / `bbb.txt`）：

```
win=0x1200004 after=15s
  Absolute upper-left X: 281   Y: 142   W: 1012   H: 672
```

抓像素（`cap.sh` → `root.ppm`，2400×1080）后按窗口矩形裁剪，逐像素统计：

| 指标 | 值 |
|---|---|
| 窗口内不同颜色数 | 26 286 |
| 内容区（去掉顶栏 48 px、侧栏 240 px）不同颜色数 | 17 223 |
| 内容区主色占比（白底） | 0.757 |
| 有文字边的行 | 44 / 672 |

渲染结果（`docs/assets/nau-46-content-20261125.png`）：侧栏 Files/Recent/Starred/Home/Desktop/Trash/
Other Locations 齐全，标题栏是 `Home / vtest`，内容区两个文件图标 + `aaa.txt` / `bbb.txt` 都在，
窗口带最小化/最大化/关闭，主题是 Yaru。⇒ **内容区不空白，"cell 通路"那条候选可以排除。**

## 2 · 为什么以前看起来是「空」或「点了没反应」（FACT）

1. **旧会话残留实例劫持单实例**。Nautilus 是单实例：进程仍在（上一轮会话留下的），
   `--new-window` 只把请求转给它，而那个实例的 X 连接已经死了 → **一个新窗口都不出现**。
   实测：残留实例在时连等 60 秒，`big_windows` 一直为空；先 `kill -9` 掉实例再起，15 秒出窗。
   `husk-desktop.sh` 的 `stop_leftovers` 里本来就有 `usr/bin/nautilus`，所以走单一入口起会话不会有这个问题。
2. **出窗慢**：从进程起来到窗口映射约 **15 秒**（等 tracker 初始化）。这段时间点第二次不会更快，
   表现就像"点了没反应"。
3. **以 root 跑**：它自己打 `This app cannot work correctly if run as root`，某些功能降级，
   但列表/导航/选择都正常（上面的渲染）。

## 3 · 和 `/proc/self/mountinfo` 那条 EPERM 的关系（FACT）

同一进程日志里仍有：

```
GLib-GIO-WARNING: Error creating IO channel for /proc/self/mountinfo: Operation not permitted (g-io-error-quark, 14)
```

按 evidence 202 的定根因，这条只让"挂载变化通知"失效（虚拟 proc 文件是 memfd，`epoll_ctl` 对
普通文件 EPERM），**不影响目录枚举与渲染**——本节的渲染结果就是证据。

## 4 · 对文档的更正

`docs/LIMITATIONS.md` §8 里 "文件管理器（Nautilus 46）……**内容区空白**" 一行改掉：
改成"能出窗、能枚举、能渲染；首次出窗约 15 秒；旧会话残留实例会让 `--new-window` 不出窗
（会话入口已清）"。

## 5 · 复现

```sh
ps -eo pid,args | awk '/org\.gnome\.Nautilus/ && !/awk/ {print $1}' | xargs -r kill -9
sh xe-plain.sh usr/bin/env DISPLAY=:0 usr/bin/nautilus --new-window /root/vtest &
# 约 15 秒后应出现 ~1012x672 的窗口；sh files/linux/cap.sh 取像素核对
```
