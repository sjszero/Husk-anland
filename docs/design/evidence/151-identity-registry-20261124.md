# 151 · `hidepid=2` + 独立 mount namespace 下的 peer 身份：rootfs 身份注册表（FACT）

日期 2026-11-24 · RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4KiB / arm64）
包 `com.linuxhub.myth.husk`，`untrusted_app`
loader `cb5d440673cecfab8c7eb40c8ea17d670f57df1dd303a8e14de3f70f7d6f3563`

## 0 · 症状

桌面起来以后，凡是"某个 daemon 去问内核『连过来的这个 pid 是谁』"的链路全线失败：

- polkitd：`Only trusted callers can query other subjects`，`pkcheck` 恒为非 0；
- 会话总线上的 `GetConnectionCredentials` 对本 app 的连接返回 `uid=None`；
- at-spi 无障碍总线：认证后无法完成，无障碍客户端连不上注册表；
- gvfs 点对点总线：只能回退到会话总线。

共同点：这些 daemon 全在**桌面会话的名字空间里**跑，而被调用的客户端是终端起的进程。

## 1 · 根因（FACT，两步实测）

**(a) `/proc` 是 `hidepid=2`：**

```
$ adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "grep -E \" /proc \" /proc/mounts"'
/dev/block/dm-32 /proc ext4 ro,seclabel,relatime,hidepid=2 0 0
```

**(b) 桌面会话与终端不在同一个 mount namespace（因此看到的 `/proc` 不是同一张表）：**

```
$ adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "readlink /proc/self/ns/mnt"'   # 终端侧
mnt:[4026536156]
$ ... 在会话里同一条命令
mnt:[4026536537]
```

所以在会话内的 dbus-daemon 里 `openat("/proc/<终端 pid>/environ")` 直接 **ENOENT**
（不是 EACCES：那个 pid 在它那张 `/proc` 表里根本不存在）。shim 拿不到对端的
`HUSK_UID`/`HUSK_GID`，只能把内核给的**真实 uid（无对应 passwd 项）**往上抛，
于是 EXTERNAL 认证走不通，掉到 ANONYMOUS；polkitd 只看到 pid、没有 uid，判定
"不可信调用者"。

> 之前查了很久没定位，是因为在终端侧验的一切都正常（读得到 environ、uid 也对）——
> 差异只在"从哪个名字空间读"。

## 2 · 做法（shim 内，零系统调用拦截的前提下）

rootfs 是所有名字空间共享的那棵树，而且是这台机上**唯一一个所有 guest 进程都能写的
共同路径**，于是把身份搬过去：

```
<rootfs>/tmp/.nr-ident/<pid>   ->   "<uid> <gid>\n"
```

- **发布**：`fake_id()` 每次报到时调 `ident_sync()`，值变了才落盘（launcher 播一次种、
  `su` 中途改身份都能覆盖）；`mkdir` 一次、`O_TRUNC` 覆写，几十字节。
- **查询**：`peer_identity()` 先读注册表，再回退 `/proc/<pid>/environ`（覆盖"对端还没
  报到过"的窗口期）。
- **同一个 pid 复用**：进程退出后文件留着没关系——pid 复用后新进程报到时会被覆写。

同时删掉了一处错误的**否定缓存**：读 environ 失败时会把该 pid 记成"不是我们的"，
此后永不再查。ENOENT 是暂时状态（对端可能还没 exec 完），不能缓存。

## 3 · 实测（FACT，全部为本机输出）

注册表有内容（`ls <rootfs>/tmp/.nr-ident`）：

```
26003 26007 26047 26049 ...
```

终端侧客户端连 **system bus** 做 EXTERNAL 认证（`/tmp/c4.py`）：

```
resp b'OK dfdffccc0093b1352122113f6b052bbc\r\n'
```

`GetConnectionCredentials` 遍历全部连接（`/tmp/pk3.py`）：

```
:1.10 pid=26262 uid=1000  olduid=1000        # 修复前此处为 uid=None
```

polkit 真正放行（对 gnome-shell 的 pid 做一次关机动作的鉴权）：

```
$ pkcheck --action-id org.freedesktop.login1.power-off --process <gnome-shell pid>
$ echo $?
0
```

at-spi 无障碍总线（`/tmp/a11y.py`）：

```
connected :1.8   names ['org.freedesktop.DBus', 'org.a11y.atspi.Registry']
```

## 4 · 复现步骤

```sh
# 1) 起桌面（App 内点显示器按钮），等 30–40 s
adb shell input tap 1546 805

# 2) 身份注册表
adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "ls /tmp/.nr-ident | head"'

# 3) system bus EXTERNAL 认证
adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "python3 /tmp/c4.py"'

# 4) 所有连接的凭据
adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "python3 /tmp/pk3.py"'

# 5) polkit
adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c \
  "pkcheck --action-id org.freedesktop.login1.power-off --process \$(pidof gnome-shell); echo rc=\$?"'

# 6) at-spi
adb shell 'run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c "python3 /tmp/a11y.py"'
```

## 5 · 代价与残留

- 每个报到进程一个几十字节的小文件；目录常在 `/tmp`，随 rootfs 生命周期消失。
- pid 复用、进程非正常退出都会留下陈旧条目；只在"某个 pid 恰好被复用且从未报到"的
  极窄窗口里可能读到旧身份，随后被新进程的报到覆写。
- 诊断开关留给现场：`/tmp/nr-ident.log`，每进程最多 4096 行，标签 `o`（open 结果）、
  `e`（读失败）、`R`（命中注册表）。当前实测只剩 `R` 与偶发 `e`。

## 6 · 关联

- polkit 调用者可信层（A1）✅
- at-spi 无障碍总线（A2）✅（同因）
- gvfs 点对点总线（A3）待复查：仍回退会话总线，需确认是否同因残留
