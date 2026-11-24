# 147 · 第一帧出屏：两条私有总线活过来、信号通了、GNOME 46 真的画在手机上

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`，MT6893 / Android 13 / kernel 4.19.191 /
4 KiB / `untrusted_app`），包 `com.linuxhub.myth.husk`（debug），rootfs =
`files/linux/distributions/ubuntu-24.04/rootfs`（App 自己装的 814 包树，无 proot）。

## 0 · 结论（FACT）

在不计 ptrace、不 root、不加 seccomp 拦截面的前提下，Husk 现在能把 **GNOME 46 的完整桌面
画到手机屏幕上**：顶栏时钟、Ubuntu 壁纸、DING 桌面图标（主目录/回收站）、左侧 dock、
通知气泡全都在屏上（11:14 的截屏）。会话全程由 App 自己的进程树拉起：
`com.linuxhub.myth.husk` → `libld.so`（我们的 loader）→ `gnome-shell 9229` +
DING/Nautilus/gvfs/tracker 一整套。

在它出屏之前，这一轮修掉 **四个各自都足以让桌面起不来** 的独立故障，全部有上机实测：

| # | 故障 | 证据 | 修在哪 |
|---|---|---|---|
| 1 | `dbus-daemon` 1.14 要把 `RLIMIT_NOFILE` 抬到 65536，Android 硬上限 32768 → 它**拒绝每一条连接** | 日志 `org.freedesktop.DBus.Error.AccessDenied: Failed to set fd limit to 65536`；客户端 11 ms 就 `Did not receive a reply`，daemon 仍在 epoll_wait | shim 新增 `setrlimit()` interpose（钳到硬上限） |
| 2 | 私有系统总线用 `EXTERNAL` 认证，而容器对外只有"假身份" | 假 root：uid 0 vs `SO_PEERCRED` 10265 不匹配；关掉假 root：`Could not get password database information for UID of current process` | 系统总线配置改 `ANONYMOUS`（会话总线本来就是） |
| 3 | shim 的 `kill()` 策略**拒绝一切非自身目标** | `kill -TERM $$` 之后 shell 仍打印 `survived`；`pkill -9 -f dbus-daemon` 报 EPERM 却留下 7 个 daemon | 改成直接透传内核（+ SIGSYS 兜底 EPERM） |
| 4 | GDBus 按"假 uid"填 `SCM_CREDENTIALS`，内核只认真实 uid/gid | gnome-shell 起来后又退出：`Failed to connect to session bus: 发送凭据时出错：不允许的操作` | shim 新增 `sendmsg()` 里改写 `SCM_CREDENTIALS` 的 pid/uid/gid |

外加一个 2.0 侧的路径错位（§4）：X socket 建在 App 私有 tmp，guest 却去 `rootfs/tmp` 找。

## 1 · 故障 1：dbus-daemon 被 `RLIMIT_NOFILE` 挡住（FACT）

现象：点"启动桌面"后 `tmp/doro-startup-stage.log` 只到 `stage=outer-root`，
`tmp/doro-system-dbus.log` 是 `system_bus=unreachable attempts=50`，
`stage=before-su` 永远不出现（系统总线脚本 `exit 1`）。现场还有 6–7 个 `dbus-daemon`
挂在 `do_epoll_wait`，`/run/dbus/system_bus_socket` 在，任何客户端连上去都立刻失败。

复现与定位（`--nofork --nosyslog`，stderr 直接落在文件里）：

```text
dbus-daemon[32187]: [session uid=0 pid=32187] org.freedesktop.DBus.Error.AccessDenied:
    Failed to set fd limit to 65536: Operation not permitted
```

```text
soft=32768 hard=32768          # ulimit -n / -Hn（untrusted_app 与 runas_app 都一样）
$ ulimit -Hn 65536
sh: ulimit: error setting limit (Operation not permitted)   # 无 CAP_SYS_RESOURCE，抬不动
```

INFERENCE（证据充分）：dbus-daemon 1.14 在**每条新连接**的 setup 里调
`setrlimit(RLIMIT_NOFILE, 65536)`，失败即把该连接判成 AccessDenied 并挂断，所以
socket 在、daemon 活、客户端永远收不到 AUTH 回复。这个 65536 是常量，与
`<limit name="max_completed_connections">` 无关（实测把三个 limit 调小仍是 65536）。

修法（`path-shim`）：interpose `setrlimit()`——先 `prlimit64(...NULL, &old)` 读天花板，
把请求的 `cur/max` 钳到 `old.max` 再下发。容器本来就不可能持有比内核允许更多的 fd；
请求落在天花板内的照旧原样下发。

## 2 · 故障 2：系统总线的 `EXTERNAL` 认证在容器里无解（FACT）

钳掉 fd limit 之后 daemon 不再拒绝连接，但系统总线仍连不上。两种身份各测一次：

```text
# 假 root（默认）：uid=0，SO_PEERCRED=10265 → 不匹配
Failed to open connection to "system" message bus: Did not receive a reply.

# NR_FAKE_ROOT=0：uid 对上了，但没有 passwd 条目
dbus-daemon[32707]: Could not get password database information for UID of current process:
    User "???" unknown or no memory to allocate password entry
```

同一条 socket 上换 `ANONYMOUS` 立刻正常（`method return ... ListNames`，连两个客户端都通）。
会话总线配置一直是 `ANONYMOUS`（2.0 里写明了原因），系统总线漏了，现在补上：
`linuxhub-system-bus.conf` 从 `<auth>EXTERNAL</auth>` 改成 `<auth>ANONYMOUS</auth>` +
`<allow_anonymous/>`。这条总线只活在本 App 自己的 uid 里，没有更大的信任边界。

## 3 · 故障 3：容器不能给自己人发信号（FACT，改前实测）

`path-shim` 原来的 `kill/tkill/tgkill` 策略是"目标不是自己 → 直接 EPERM"，注释写的理由是
"app 本来就发不出信号"。实测是错的：

```text
# 同一个 uid、同一个 run-as 域，绕开 shim（/system/bin/sh 的 toybox kill）
started=32504  kill0=0  kill9=0  →  No such process      # 内核允许同 uid 互发

# 走 shim（guest 里的 dash）
kill -9 32393        → rc=0，目标仍活          # 假成功
kill -9 999999       → "Operation not permitted"（应为 ESRCH）
kill -TERM $$        → shell 继续跑
```

后果是容器**管不了自己的进程**：`timeout` 杀不掉超时的命令、`pkill -9 -x gnome-shell`
杀不掉上一轮、6–7 个 daemon 永远不退（这才是"`--fork` 会在同一 address 堆出多个 daemon"
的真实原因：不是 `--fork` 的问题，是没人能杀）。

修法：三个入口直接透传内核（同 uid 才能成功，跨 uid 内核自己给 EPERM——这正是
proot 下容器用户看到的行为）；另外 `on_sigsys` 对 129/130/131 兜底：万一这台 ROM 把
kill 家族塞进 seccomp TRAP，就把该次调用以 `-EPERM` 结束，而不是让进程 159 死掉。

改后实测（runas_app 域，同一颗树）：

```text
/bin/sh -c 'kill -TERM $$'      → Terminated（outer_rc=143）
kill -9 <sibling>               → rc=0，/proc/<pid> 消失
kill -0 <live sibling>          → 0；kill -TERM <sibling> → 目标消失
pkill -9 -f dbus-daemon         → daemons_before=7 → daemons_after=0
```

补一条域边界（FACT）：`runas_app` 域**杀不掉** `untrusted_app` 域里的进程
（`pkill: killing pid 6826 failed: Permission denied`，SELinux 拦的），只有 App 自己的
进程树里才管得动——这也是为什么"桌面会话只能由 App 域控制"。

## 4 · 故障 4：X socket 建在 App 私有 tmp，guest 却在 `rootfs/tmp` 找（FACT，2.0 侧）

第一次把两条总线修好之后，mutter 起来了，但：

```text
Running GNOME Shell (using mutter 46.2) as a X11 window and compositing manager
Failed to setup: Unable to open display ':0'
```

对照两边目录：

```text
files/linux/tmp/.X11-unix/X0                  ← Lorie 建的（Android 侧进程，TMPDIR 指这里）
rootfs/tmp/.X11-unix/                         ← 空的（guest 的 /tmp 是这里）
```

proot 时代靠 `-b <app>/tmp:/tmp` 把两者并成一个；bind 删掉之后必须有一侧让步。
现在让 Android 侧让步：`X11Environment.prepare()` 的 `tmpDir` 改成
`<rootfs>/tmp`（`HuskRuntime.guestDirs` 已经保证它存在），于是服务器和 guest 都叫
`/tmp/.X11-unix/X0`。

## 5 · 故障 5：GDBus 发不出 `SCM_CREDENTIALS`（FACT）

X 通之后桌面开始搭 UI，`LH_STAGE` 一路走到 `panel`、`keyboard`，然后：

```text
Failed to connect to session bus: 发送凭据时出错： 发送信息时出错：不允许的操作
```

机制（INFERENCE，有内核语义支撑）：GDBus 用 `g_credentials_new()` 的结果填
`SCM_CREDENTIALS`，而我们的假身份让它填 `uid=0`；内核只接受 uid/gid 与**真实**值一致的
control message（否则要 CAP_SYS_ADMIN），于是 `sendmsg` 返回 EPERM。proot 时代同样的问题
由 proot 自己改写报文解决，我们现在在自己的 shim 里做同一件事。

修法：interpose `sendmsg()`，扫 `msg_control` 里的 `SOL_SOCKET/SCM_CREDENTIALS`，把
pid/uid/gid 换成真实值（接收端从 `SO_PEERCRED` 读到的本来就是这三个数，所以没有伪造任何
内核不肯给的东西）。修完 `gnome-shell` 连上会话总线，DING、Nautilus、gvfs、tracker 全部起来。

## 6 · 出屏的那一次（FACT）

```
stage=outer-root
stage=before-su                    ← 第一次出现
stage=session-user
stage=exec-shell display=:0 runtime=/tmp/runtime-husk gpu=software
system_bus=ready pid=9238
session_bus=ready pid=9256 address=unix:abstract=linuxhub-session-bus-husk
logind=unavailable stale_seats=removed logind_features=disabled
LH_STAGE start → parental-controls → before-initialize-ui → tracker → layout → overview
        → window-manager → message-tray → panel → keyboard → notification-daemon
        → component-manager → layout-init → overview-init → end-session
        → after-initialize-ui → access-dialog
LH_LAYOUT monitors=1 stage_visible=false stage_size=2400x1080
窗口管理器警告：Impossible to set scaling on crtc 63（Lorie 没实现 xrandr 的 scaling）
```

11:14 的截屏：顶栏（时钟/输入法/音量/电源）、Ubuntu 壁纸、桌面图标、左侧 dock、
"屏幕锁定已禁用"通知气泡。`gnome-shell`(9229) 与其 40+ 个子进程全部存活。

## 7 · 还没收口（诚实清单，全部实测过现状）

| 项 | 现状（FACT） | 影响 |
|---|---|---|
| ~~僵尸堆积~~ **已修** | 修之前那个会话：uid 10265 的 142 个进程里 **105 个 Z**，GLib 报 `waitid(pid:10017, pidfd=11) failed: 无效的参数 (22)`。修之后同一套流程跑 100 s：37 个进程、**0 个 Z**、`waitid` 警告 0 条 | 见 §7.1 |
| ~~`polkitd` 起不来~~ **已修** | 根因是 `Error switching to user polkitd: Error initializing groups: Operation not permitted`——glibc 的 `initgroups()` 在 libc 内部发 `setgroups`，GOT 改写看不见它（和 evidence 139 里 `su` 是同一条）。修：shim interpose `initgroups()` | 修后 `polkitd` 常驻并持有 `org.freedesktop.PolicyKit1`（`GetNameOwner` → `:1.31`），只剩 `Error getting login monitor: -2`（没有 logind，符合设计） |
| gvfs 点对点总线 | `用尽了所有可用的认证机制（已尝试：EXTERNAL）`，随后回退到会话总线 | 功能上能用（已回退），只是日志噪音 |
| accessibility bus | `Couldn't register with accessibility bus` | at-spi 不可用 |
| Lorie 的 FPS 计数 | 桌面在屏上，但 `LorieNative` 仍打 `0 frames in 5.0 seconds = 0.0 FPS` | 计数口径问题（`choreographer=302 redraw=302` 一直在涨），不是没画 |
| `landlock` trap 噪音 | `nr-sigsys.log` 832 KB，几乎全是 `nr=444 mode=2` | 无害但淹没真记录，要按 nr 去重 |
| 无 xrandr scaling | `Impossible to set scaling on crtc 63` | 分辨率不能改 |

### 7.1 · 僵尸的根因与修法（FACT）

这台 kernel 有 pidfd（所以 GLib 用得上），但 `waitid` 不认 `P_PIDFD`：

```text
GLib-WARNING **: waitid(pid:10017, pidfd=11) failed: 无效的参数 (22)
```

`P_PIDFD` 是 5.4 才进 `waitid` 的，这里是 4.19.191，于是内核直接 EINVAL；而
GLib 2.76+ 的 child watch **优先** 走 pidfd，失败之后那个 child 就没人回收——一晚上
105 个僵尸。修法（`path-shim` interpose `waitid`）：遇到 `idtype == P_PIDFD` 时，先读
`/proc/self/fdinfo/<fd>` 里内核自己打的 `Pid:` 行（`NSpid:` 不算），拿到 pid 后按
`P_PID` 下发同一个调用；读不出来就原样交给内核，行为与修之前一致。实测：同一套
桌面流程 100 s 后僵尸 0、`waitid` 警告 0 条，进程数从 142 降到 37（之前的差值就是僵尸）。

## 8 · 还没做的：桌面收不到输入（FACT）

点屏幕上任何位置，guest 里的光标不动（一直停在右下角），桌面也不响应。
代码层面同一条结论：`X11Runtime` 接口只有 `attachSurface/resize/detachSurface`，
**没有任何注入指针或键盘的入口**——termux-x11 的 `LorieView` 那套触摸转发随它的
Activity 一起删掉了，2.0 这条路径还没有替代品。这是"桌面能不能用"的第一块砖，
排在 §7 剩下的几条之前。

## 8.1 · 复现

```sh
# 1) 三个 shim 改动打进 loader
NDK=<ndk>/toolchains/llvm/prebuilt/darwin-x86_64/bin OUT=/tmp/loader sh tools/loader/build-embedded-loader.sh
# 2) 换到设备上正在用的那份（rename，别 cp 覆盖：在跑的进程还映着它）
adb push /tmp/loader/libld.so /data/local/tmp/libld.so
adb shell run-as com.linuxhub.myth.husk sh -c \
  'cd files/linux/husk && cp /data/local/tmp/libld.so new && chmod 755 new && mv -f new libld.so'
# 3) 清场（现在真的能杀干净了）
run-as ... sh files/linux/xe.sh bin/sh -c 'pkill -9 -f /usr/bin/dbus-daemon; pkill -9 -x gnome-shell'
# 4) App 里点显示器按钮，看 tmp/doro-startup-stage.log 走到 stage=exec-shell
```

loader：`b3edc99eac99b2afa201f18c7432514068f1273b1a75f90f650d1fd1bdbc600f`（738,816 B，含 §7.1 的 `waitid` 翻译与 `initgroups` 仿真）。
