# 222 · 会话形态审计：umask、/proc 魔术链接、xattr 家族的 errno

日期：2026-11-25 22:52–23:20（CST）。设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB。
审计面：会话入口（`husk-session.sh` / `husk-desktop.sh`）到底把一个什么样的进程交给 guest。

## 0 · 四条结论

| 问题 | 性质 | 现状 |
|---|---|---|
| 会话 umask 是 0000（Android 给 run-as 的值），guest 里 `mkdir` 得到 0777、文件 0666 | 兼容性：DING 拒绝从世界可写的 `~/Desktop` 启动，daemon 对 mode 的判断跟着错 | 已修：两个入口都 `umask 022` |
| `readlink("/proc/self/cwd")`、`readlink("/proc/self/root")` 返回 ENOENT | 兼容性：`realpath` 类相对路径解析、「我在哪」判断全部失效 | 已修：魔术链接的**内容**从 guest 视图回答 |
| `/proc/self/fd/N` 的链接目标直接是宿主路径，`/proc/self/fd/1024` 甚至指向应用数据目录 | 信息泄漏 + 程序按宿主路径做判断 | 已修：落在 rootfs 里的目标改写成 guest 路径 |
| 路径解析失败时，`inotify_add_watch` / `statfs` / `getxattr` 一族报的是 **EPERM**，不是真实 errno | 行为错误：把「文件不存在」说成「没有权限」 | 已修：`xattr_pinned` 改为回 `-errno` |

## 1 · umask 是 0000（已修）

宿主侧（FACT）：

```
$ adb shell umask                      → 000
$ adb shell run-as com.linuxhub.myth.husk sh -c umask   → 000
```

会话继承它以后（修前，FACT）：

```
umask=0000
mkdir /root/umt-1   → drwxrwxrwx
touch /root/umt-1/f → -rw-rw-rw-
```

宿主上留下的痕迹同样说明问题：`files/linux/*.sh` 这批测试脚本全是 `-rw-rw-rw-`。

**根因**：shim 不碰 umask（它属于进程属性，不经过任何被钩的 libc 函数），所以 Android 给
什么就是什么。普通 Linux 登录会话是 022。

**修法**：`tools/guest/husk-session.sh` 与 `tools/guest/husk-desktop.sh` 在 `set -eu`
之后各加一行 `umask 022`。两个都是会话入口，且桌面那条不经过 `husk-session.sh`。

**验证**（FACT，同一台设备，修后）：

```
umask=0022
dir  /root/umt-1      755
sub  /root/umt-1/sub  755
file /root/umt-1/f    644
sh -c 'echo x > f'    644
```

## 2 · `/proc/self/{cwd,root}` 读不出东西（已修）

修前（FACT）：

```
$ readlink /proc/self/cwd   ; rc=1   errno=ENOENT
$ readlink /proc/self/root  ; rc=1   errno=ENOENT
$ ls -l /proc/self/cwd      ; No such file or directory
$ python3 -c 'os.readlink("/proc/self/cwd")' ; FileNotFoundError
```

**根因**：`passthrough()` 明确拒绝把 `/proc/*/root` 和 `/proc/*/cwd` 交给内核——它们是宿主
命名空间的魔术链接，按路径解析过去会跳出 rootfs（这个拒绝是对的）。但「拒绝解析」被顺手
用到了**读链接**上，而读链接只需要一个字符串。

**修法**：`readlinkat` 里识别 `/proc/{self,thread-self,<自己的 pid>}/{cwd,root}`：`root` 直接
答 `/`，`cwd` 用内核给的宿主路径过一遍 `readlink_guestify()` 折回 guest 路径。别人的 pid 一律
不答，仍走原来的拒绝路径。

修后（FACT）：

```
cwd      -> /root
root     -> /
fd/9     -> /etc/passwd        (exec 9</etc/passwd)
fd/1024  -> /                  (runtime 的私有 rootfd)
fd/0     -> socket:[7817670]   (不被改写，只在 rootfs 内才改写)
pwd -P   -> /root
symlink  -> /usr/bin/env       (普通 guest 符号链接不受影响)
```

**仍然拒绝**：按路径解析穿过它们，例如 `ls -l /proc/self/cwd` 依旧 ENOENT。也就是说
`open("/proc/self/cwd/x")` 这类写法不通——这是有意的边界，已记进 `docs/LIMITATIONS.md` §14。

## 3 · 同一个失败，exec 族和 inotify 报两个不同 errno（已修）

线索来自桌面会话的会话总线日志（FACT，修前）：

```
dbus-daemon: Cannot setup inotify for '/root/.local/share/dbus-1/services';
             error 'Operation not permitted'
```

目录确实不存在，正常 Linux 应该说 "No such file or directory"。C 探针按符号调用
（GOT 路径，不走 raw syscall）：

```
修前                                     修后
/etc                              wd=1  errno=0      wd=1  errno=0
/usr/share                        wd=2  errno=0      wd=2  errno=0
/root                             wd=3  errno=0      wd=3  errno=0
/root/.local/share/dbus-1/services wd=-1 errno=1 EPERM   wd=-1 errno=2 ENOENT
```

**根因**：`xattr_pinned()` 解析失败时写 `errno=e` 并返回 `-1`，而调用方一律把返回值喂给
`ret_or_errno()`；后者看到 `-1` 就当作 `-EPERM` 处理，把 errno 覆盖成 1。
`-1` 既是「失败」又是「EPERM 的编码」，两者撞在一起。

影响面（FACT，读代码确认）：`inotify_add_watch`、`statfs`、`getxattr`、`setxattr`、
`listxattr`、`removexattr` 全部走这条。`statvfs` 那条路径里有一处 `r == -1` 的特判，所以它
没中招——这也说明这个坑不是靠「注意点」能守住的。

**修法**：`xattr_pinned` 改成失败时返回 `-(e as Ssize)`，把 errno 编码留给
`ret_or_errno`/`ssize_or_errno` 统一处理。

**验证**：修后上面那条 `dbus-daemon --session` 起来时不再打印 inotify 抱怨
（`grep -c "Cannot setup inotify" → 0`）。

## 4 · 回归

- 门禁 A：`PASS=38 FAIL=0`（含 coverage-probe PASS、coverage-sweep PASS），修前修后各跑一次。
- gnome-shell A/B（同一份 rootfs、同一套环境变量、只换 loader）：
  修前构建 `libld-prev.so` 与修后构建各起一次 `gnome-shell --x11 --replace`，
  **两边都在 25 秒后仍然活着**（FACT）。今天这三处改动没有引入桌面启动的回归。
- 桌面（窗口/合成器）级验证：**未完成 NOT VERIFIED**。原因写在下面，不是「看起来没问题」。

### 为什么桌面级没验完

这台设备的 X server 在应用进程里。测试到一半它不再应答：`xdpyinfo`、`xprop` 从「立刻返回」
变成「挂住直到超时」，同一时刻 `dumpsys activity top` 显示前台是 launcher 而不是本应用，
`/tmp/.X11-unix` 也不存在了。23:21 直接截屏看屏幕（FACT）：**设备处于锁屏**，停在
「仅限紧急呼叫 / 23:21」那一屏。也就是说应用被系统收到后台、它的 X server 跟着停止服务，
`husk-desktop.sh` 的 `gnome-shell` 于是停在
"Running GNOME Shell (using mutter 46.2) as a X11 window and compositing manager" 之后没有下文
（同一份日志里 DING 扩展还起过：`DING: GNOME nautilus 46.4`，说明 shell 本身是起来了的）。

结论：**这一轮桌面起不来的原因是设备锁屏 → 应用退后台 → 显示服务不在，不是 shim**。
要判桌面，得先解锁、把应用拉到前台再跑一遍；这一点写在这里，别当成「看起来没问题」。

## 5 · 会话形态的其余观测（本轮只记不改）

| 项 | 实测 | 判断 |
|---|---|---|
| 继承的 fd | `0`=socket、`1`=pipe、`2`=socket、`1024`=rootfd、`3`=`/proc/self/fd`（`ls` 自己的） | 正常；rootfd 读得到但穿不出去（`/proc/self/fd/1024/..` 仍 ENOENT） |
| `ulimit -n` | 32768 | 与 Android 应用一致，不是 loader 改的 |
| 信号处置 | `SigCgt=0x400004c8`（SIGILL/BUS/FPE/SEGV/SYS），`SigIgn=0x2000000000` | **与宿主 `adb shell` 逐位相同**（同一台机直接 `cat /proc/self/status` 对照）——继承来的，shim 不加也不清 |
| cwd / HOME / PATH | `/root` / `/root` / 标准 guest PATH | 正常 |
| `/proc` 进程视图 | 895 条，含本应用自己的进程，无其它 Android 应用 | 门禁 A 那条断言覆盖的就是这个 |

## 6 · 顺带确认的一件事

`ctypes.CDLL("libc.so.6").inotify_add_watch(...)` 拿到的是 libc 里**原始**符号，绕过 GOT 改写，
所以同一个调用在 python 里返回 ENOENT、在 C 探针里返回被翻译后的结果。这不是 bug，是
`LIMITATIONS` §9 那条「raw syscall / dlsym 直取」边界的一个具体样子——写探针时要按符号调用，
否则量到的是宿主内核的行为。
