# 353 · 桌面冷启动定量：14.7 s 花在哪

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
宿主：`com.linuxhub.myth.husk`（黑盒，未改其源码）
运行时：工作区 `c924d62` + 本轮改动，`loader_sha256=67f543b9…`

## 0 · 结论

| 指标 | 中位数 | 范围 | 判定 |
|---|---:|---:|---|
| App 冷起到“打开桌面”按钮可见 | 3.23 s | 3.17–3.47 s | FACT（5 次） |
| 点按钮 → gnome-shell 进程真实创建 | ≈ 4.95 s | 4.9–5.0 s | FACT（由 `/proc/<pid>/stat` 第 22 字段算） |
| 点按钮 → 桌面 SurfaceView 出现（首帧可用） | **14.73 s** | 14.52–15.37 s | FACT（5 次） |
| gnome-shell 创建 → 首帧 | 9.70 s | 9.38–9.89 s | FACT（5 次） |

**关键发现**：gnome-shell 起来后有 **约 4.1 s 完全空转**（utime/stime 增量都是 0），
之后才进入约 3.4 s 的满核 CPU 阶段，然后才出首帧。慢的主体是**阻塞等待**，不是算力。

## 1 · 5 次冷启原始数据

```text
RUN 1 app_to_button_ms=3238 tap_to_shellpid_ms=6918 tap_to_surface_ms=14647
RUN 2 app_to_button_ms=3167 tap_to_shellpid_ms=6989 tap_to_surface_ms=14730
RUN 3 app_to_button_ms=3467 tap_to_shellpid_ms=7942 tap_to_surface_ms=15370
RUN 4 app_to_button_ms=3325 tap_to_shellpid_ms=6973 tap_to_surface_ms=14523
RUN 5 app_to_button_ms=3187 tap_to_shellpid_ms=7029 tap_to_surface_ms=14901
```

- `tap_to_shellpid_ms` 是“轮询发现进程”的时间，含最多 150 ms 轮询粒度；
  用 `stat` 的 starttime 反算的真实创建时刻是 tap + 4.9~5.0 s。
- 首帧判据：`dumpsys SurfaceFlinger --list` 出现 `SurfaceView[com.linuxhub.myth.husk…]`，
  单次查询 24 ms，不抓屏、不抢设备。前几轮用全屏 `screencap`（每帧 10 MB）轮询，
  那个方法本身会拖慢合成，已废弃。

## 2 · 进程启动序列（锚定宿主 X server 启动 = t+0）

```text
 +0.00s  com.linuxhub.myth.husk:x11        ← 宿主起 X server
 +4.81s  gnome-shell
 +5.03s  libld.so（子进程，dbus-daemon 一线）
 +6.44s  gsd-color / gsd-media-keys / gsd-xsettings
 +6.83s  gvfsd
 +6.83s ────────── 3.9 s 无任何新子进程 ──────────
+10.72s  at-spi-bus-launcher
+10.77s  dbus-daemon
+10.86s  at-spi2-registryd
+11.24s  mutter-x11-frames
+12.09s  xdg-permission-store / gnome-shell-calendar-server
+12.16s  evolution-source-registry
+12.31s  gjs
+12.39s  evolution-calendar-factory
+12.58s  evolution-addressbook-factory
+12.65s  gvfs-udisks2-volume-monitor
+12.78s  gvfsd-trash / gvfs-afc / gvfs-mtp
+12.86s  gvfs-gphoto2 / gvfs-goa
+13.20s  dconf-service
+14.57s  ding.js                          ← 桌面图标扩展
+14.85s  xdg-desktop-portal
+14.98s  xdg-desktop-portal-gnome
+14.7s   ← 首帧
```

## 3 · CPU 采样：4.1 s 纯空转

同一次启动，每 250 ms 读一次 `/proc/<gnome-shell>/stat` 的 utime/stime 增量（单位 tick=10 ms）：

```text
t=+10ms    utime=8  stime=6      ← 起步
t=+299ms   utime=18 stime=5
t=+591ms   utime=0  stime=0   ┐
t=+885ms   ...                     │
 ...                               │ 约 4.1 s 内 utime/stime 增量全为 0
t=+4163ms  utime=0  stime=0   ┘
t=+4459ms  utime=8  stime=5      ← 开始满核跑
t=+4749ms  utime=16 stime=8
 ...
t=+7633ms  utime=13 stime=12
t=+7926ms  utime=2  stime=6   SURFACE（首帧）
```

即：**等待 4.1 s（0% CPU）→ 计算 3.4 s（近 100% 单核）→ 首帧**。

## 4 · 已排除的假设

| 假设 | 测法 | 结果 |
|---|---|---|
| 冷启动每帧 CPU 回读 | `xdpyinfo` + `glxinfo` | **不成立**：DRI3/Present 都在，`direct rendering: Yes`，`glxgears` 141–155 FPS |
| DNS 超时（`resolv.conf` 是 `timeout:2 attempts:3`） | guest 内 `getent hosts foo.invalid` | **不成立**：125 ms；localhost 40 ms；IP 22 ms |
| zink/mesa 着色器缓存没写盘、每次重编 | 统计 `~/.cache/mesa_shader_cache` 今日改动 | **不成立**：今日新增 81 个条目，缓存正常写 |
| 每次 exec 的 loader 固定成本叠成 8–11 s | 用子进程启动序列反推 | **不成立**：40 个子进程 × 7 ms ≈ 0.3 s。**修正 evidence 350 之后我的口头推断**，loader 成本不是主因 |

## 4b · 那 4.1 s 在等什么：主循环 poll 阻塞

趁启动窗口每 200 ms 采一次 `/proc/<gnome-shell>/wchan` 与 `syscall`：

```text
wchan=do_sys_poll      syscall=73 (ppoll)  state=S
wchan=futex_wait_queue_me syscall=98        state=S
wchan=0                syscall=running     state=R
```

即那段等待期间 shell 处在主循环的 `ppoll`/`futex` 上：**在等外部事件，不是卡在代码里**。
配合 §3 的 0% CPU，可以排除“算得慢”，锁定为“等得久”。

`dbus-monitor` 抓不到这段：这个会话总线是私有配置
（`unix:abstract=linuxhub-session-bus-husk`，`EXTERNAL` + `ANONYMOUS`），
没有 eavesdrop 权限，26 s 里只收到 7 条（都是监控进程自己的）消息。
要定位具体等待对象，需要在总线侧临时开 `deny_own`/eavesdrop，或用
`gdb`/`ftrace` 看 poll 的 fd 指向哪个 socket。

顺带发现：`$XDG_RUNTIME_DIR/gvfsd/` 里积了 **146 个历史 socket**，
会话退出时没有清理，每启动一次再加一个。见 H42。

## 5 · 下一步（按信息量排序）

1. 定位 `ppoll` 等的那个 fd 是谁：临时在会话总线配置里开 eavesdrop 跑一次，
   或对 shell 进程做 `ftrace`/`gdb` 抓 poll 的 fd 目标。
2. 若确认是某个可延迟的服务（evolution/tracker/portal 一类），把它的激活改成异步或延后。
3. 3.4 s 的 CPU 段用 `perf` 或 `MESA_DEBUG` 确认是不是首帧管线编译；若是，改成启动后预编译。

## 6 · 未验证

- 4.1 s 等待的确切对象（本轮只证明“在等、不在算”）。
- 强退重进、等 30 s 重进两组数字：本轮只做了冷启 5 次。
- 与 PRoot 的同机同负载对照：本轮未测（上一轮起了 PRoot 但未取数，已停止）。
