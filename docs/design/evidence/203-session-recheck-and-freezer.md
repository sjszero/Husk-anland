# 203 · 0.0.6 会话复核：单一入口一次起成、拖动仍然失效、以及「桌面挂死」到底是什么

日期：2026-11-25 16:50–17:05（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`。
这一份是新提交（`24c4fcf`）之后的**复核**，三件事：
会话能否一次起成、窗口拖动到底修没修、以及开发过程中反复出现的「所有 X 命令都卡住」是不是我们的 bug。

## 1 · 单一入口一次起成（FACT）

`husk-desktop.sh` 从空环境（上一次会话的进程全清掉）跑一次，输出：

```
CHECK wm=yes
CHECK ding=yes
CHECK gpu=zink
CHECK shell_log=.../rootfs/tmp/husk-shell.log
husk-desktop: 会话已起（wm=yes ding=yes gpu=zink）
EXIT=0  17:02:48
```

`wm=yes` 来自 `xprop -root _NET_SUPPORTING_WM_CHECK`，`ding=yes` 来自桌面图标窗口，
`gpu=zink` 来自 shell 日志里的渲染器串。⇒ 会话形态这一项在 0.0.6 上**可复现地起得来**
（evidence 200 是首次达成，这一条是重复验证）。

注意一处**只影响开发布局、不影响安装版**的差异：`NR_LD` 默认取
`dirname(rootfs)/libld.so`，而本机是「loader 在 `files/linux/husk/`、rootfs 在
`files/linux/distributions/ubuntu-24.04/`」的开发布局，所以要显式
`NR_LD=$F/husk/libld.so`。安装器布局里两者同级，默认值是对的。

## 2 · 窗口拖动：仍然失效，alt+F7 可用（FACT）

`drag7.sh`：`xev` 开一个 600×500 窗口，先 `windowactivate`，然后两段动作、每段后读
`xwininfo -id`。

| 动作 | (x, y, w, h) |
|---|---|
| 起始 | `236 188 628 566` |
| `Super` 按下 → 按住 button1 → 两次 motion → 松开 → `Super` 抬起 | `236 188 628 566`（**一格没动**） |
| `alt+F7` → 指针移到 `500,400` → 单击落位 | `186 120 628 566`（**动了**） |

和 evidence 198 的结论一致：**客户端请求、X 层被动抓取、`XIChangeHierarchy` 全部不是原因**，
mutter 自己发起的指针搬窗同样不跟 motion；只有键盘搬窗（`alt+F7`）这条不依赖 grab op 的路能用。
⇒ 0.0.6 把「拖动/吸附」列为**已知缺陷**，替代操作是 `alt+F7`（见 LIMITATIONS §10）。

## 3 · 「所有 X 命令都卡住」不是我们的 bug：应用被 Android 冻结（FACT）

开发中多次出现这种状态：`gnome-shell` 不在、`xdpyinfo`/`xwininfo`/`xprop` 全部 **卡到超时**、
`echo` 和 `id` 却秒回。定位过程：

1. `ps` 里 **X server 进程还活着**（`com.linuxhub.myth.husk:x11`，CPU 0%）。
2. 它的**每一个线程**都停在 `get_signal`：
   ```
   for t in /proc/<x11 pid>/task/*; do cat $t/wchan; done  ->  get_signal (所有线程)
   ```
   `get_signal` 不是等 I/O 的姿态，是 **cgroup freezer 的冻结姿态**。
3. `dumpsys power` → `mWakefulness=Asleep`（屏幕灭了）。
4. `input keyevent KEYCODE_WAKEUP` + `am start` 之后，`xdpyinfo` 立刻恢复正常：
   ```
   name of display: :0 / version number: 11.0 / vendor: The X.Org Foundation
   ```

⇒ 链条：屏幕熄灭 → 应用转 cached → Android 的 Cached Apps Freezer 把它 SIGSTOP 冻结 →
应用进程里的 X server 一起停 → guest 侧新起的 X 客户端连上去就等 → 表现为「桌面挂死」。
**不是崩溃、不是锁死、不需要重启应用**，唤醒即恢复（FACT）。

两条直接后果，都记进 0.0.6 的发布说明：

- 用户视角：「锁屏久了再回来，桌面像死了一样」= 冻结，不是 hang。解锁就回来。
- 开发视角：这台机器 `settings put system.screen_off_timeout` 没有 `WRITE_SETTINGS` 权限
  改不了、`svc power stayon` 被 ROM 直接 `Killed`，所以自动化期间必须每 15–20 秒
  `input keyevent KEYCODE_WAKEUP` 保活，否则任何长任务都会被冻结打断
  （`husk-desktop.sh` 会被冻在中途，唤醒后继续，最终仍然 `EXIT=0`）。

另外一个**自己造成的假 bug**要记下来：同时跑多个 `husk-desktop.sh` 时，各自的
`stop_leftovers` 会互相 `pkill -9 gnome-shell`，看起来像「shell 起来了又自己死」。
复现时必须确认只有一份实例在跑（用 `ps -eo pid,args | grep husk-desktop` 核）。

## 4 · 复现

```sh
# 起会话（开发布局）
NR_LD=$F/husk/libld.so sh $R/usr/local/share/linuxhub/husk-desktop.sh $R
# 拖动对照（见 §2）
sh files/linux/drag7.sh
# 冻结判定
dumpsys power | grep mWakefulness
cat /proc/<x11 pid>/task/<tid>/wchan
```
