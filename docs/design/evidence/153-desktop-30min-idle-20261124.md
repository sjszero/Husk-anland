# 153 · 桌面空载 30 分钟：RSS / 线程 / fd / 僵尸（FACT）

日期 2026-11-24 14:47–15:17 · RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4 KiB / arm64）
包 `com.linuxhub.myth.husk`（debug），`untrusted_app`，LinuxHub 2.0 rev 21
会话：GNOME Shell 46 X11 桌面已起来并保持在前台，**全程不做任何交互**

## 0 · 结论

30 个采样点（每分钟一次），桌面空载：

| 指标 | 首 | 末 | 最小 | 最大 | 首末差 |
|---|---|---|---|---|---|
| app PSS (KiB) | 232 661 | 232 138 | 231 567 | 233 104 | **−523** |
| app 线程数 | 47 | 47 | 47 | 47 | **0** |
| `:x11` 进程 RSS (KiB) | 157 152 | 156 648 | 156 316 | 157 320 | **−504** |
| `:x11` fd 数 | 118 | 118 | 118 | 118 | **0** |
| guest 僵尸 | 0 | 0 | 0 | 0 | **0** |

没有增长趋势：RSS 波动 ±1.5 MB 属于系统页回收的噪声，末值比首值低；
线程与 fd 完全恒定；guest 侧 30 分钟零僵尸。

## 1 · 采样方法

`/tmp/husk-stability.sh`，每分钟一行，写 `/tmp/husk-stability.log`：

```sh
rss_app=$(adb shell "dumpsys meminfo <pkg> | awk '/TOTAL PSS:/{print \$3; exit}'")
thr_app=$(run-as <pkg> sh -c 'ls /proc/$(pidof <pkg>)/task | wc -l')
fd_x11=$(run-as <pkg> sh -c 'ls /proc/$(pidof <pkg>:x11)/fd | wc -l')
rss_x11=$(run-as <pkg> sh -c 'awk "/VmRSS/{print \$2}" /proc/$(pidof <pkg>:x11)/status')
z_guest=$(run-as <pkg> sh files/linux/xe.sh bin/sh -c 'ps -e -o stat= | grep -c Z')
z_android=$(adb shell "ps -A -o STAT= | grep -c '^Z'")
```

完整原始序列（30 行）保留在 `/tmp/husk-stability.log`，节选：

```
14:47:45 min=1  rss_app_kb=232661 thr_app=47 rss_x11_kb=157152 fd_x11=118 zombies_guest=0 zombies_android=1
15:02:56 min=16 rss_app_kb=232376 thr_app=47 rss_x11_kb=156324 fd_x11=118 zombies_guest=0 zombies_android=1
15:17:06 min=30 rss_app_kb=232138 thr_app=47 rss_x11_kb=156648 fd_x11=118 zombies_guest=0 zombies_android=1
DONE 15:18:07
```

## 2 · Android 侧那一个僵尸不是我们的（FACT）

30 分钟里 `zombies_android` 恒为 1，查了它是谁：

```
$ adb shell "ps -A -o PID,STAT,NAME | awk '$2 ~ /^Z/'"
 5885 Z     [settings]
```

系统 `settings` 进程的历史僵尸，与 `com.linuxhub.myth.husk` 无关；本包的进程树在
`ps -A` 里没有 Z 状态。

## 3 · 未覆盖（UNKNOWN）

- 这 30 分钟是**空载**。带负载（滚动、视频、编译）的 RSS 曲线尚未采样。
- 静止 5 分钟平均 CPU、10 分钟耗电、温度曲线：未测。
