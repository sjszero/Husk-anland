# 运行时表面：一台服务器该有什么，我们有什么

> 唯一目标机：RMX3357（Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`，无 root、无 Shizuku）。
> 全部数字来自 `tools/device-tests/rt-server-parity.sh`，在 guest 里跑，2026-11-23。
> 状态：**79 PASS / 5 PARTIAL / 6 FAIL**（58/5/18 → 71/5/6 → 73/5/6 → 76/5/6 → 79/5/6；最早基线 22 / 4 / 21）。

## 1. 对照的口径

容器和虚拟机卖的不是"一个 shell"，是**一整套约定**。拆成八层，逐层对：

| 层 | 一台容器/虚拟机提供什么 | 我们的实现手段 |
|---|---|---|
| 装载 | ELF 加载、动态链接、TLS、auxv、vdso | 改过的 glibc loader（shim 内嵌，零 ptrace） |
| libc | 完整的 glibc 语义：内部调用也走同一套路径 | GOT 重写 + 编译期槽位（evidence 105） |
| 文件系统 | 自己的根、自己的挂载表、自己的 `/proc` 视图 | 单目录根 + 虚拟挂载表 + 路径改写 |
| 进程 | fork/exec、PTY、信号、僵尸回收 | 全部实测通过（回归门禁） |
| IPC | unix/abstract/SCM_RIGHTS、eventfd/timerfd/epoll | 通过；SysV IPC 被 seccomp 墙 |
| 网络 | socket、DNS、监听、接口枚举 | socket/DNS/监听通过；接口枚举被墙 |
| 设备 | `/dev` 常规节点、pty、shm、日志 socket | null/zero/urandom/tty/ptmx/pts/shm 通过 |
| 服务 | system bus、syslog、init/监督、多用户 | system bus 已能起；`logger` 与 `syslog(3)` 都落到 `/var/log/syslog` |

## 2. 本轮修掉的（都是真问题，不是"补丁糊过去"）

| 项 | 之前 | 现在 | 做法 |
|---|---|---|---|
| `/proc/mounts`、`/proc/self/mounts`、`mountinfo` | 宿主挂载表（工具看到别的机器） | 我们自己的命名空间描述 | shim 用 memfd 提供；`df`/`findmnt` 正确 |
| `df` 文件系统类型 | 宿主 system 分区 760 M 100 % | `f2fs` 108 G 33 % | `statvfs`→`statfs` 内部调用被槽位接管 |
| 硬链接 `link(2)` | EACCES（内核拒） | 复制语义 + 一次性提示 | 内核 SELinux 不允许 app 建硬链接，只能等价替换 |
| `mkfifo`/`mknod` | ENOENT（路径没翻译） | 正常 | 补 `mknodat`/`mkfifo`/`mkfifoat` 的路径改写 |
| D-Bus system bus | 起不来（缺账号） | `system_bus_socket` 可用 | 补 `messagebus`/`pulse`/`whoopsie`/`geoclue`/`netdev` 账号（`tools/guest-config/`） |
| 装包（apt/dpkg） | 一个包都装不上（六个独立缺陷，见 evidence 106） | `apt-get install tree` / `ca-certificates` 全通，含 postinst 与 update.d 钩子 | usrmerge 解包修复 + 状态库重建 + `execl/execlp/system` + `fchown/fchmod/futimens` + `link` 少传参 + `APT::Sandbox::User` |
| usrmerge 链接 | `/bin`、`/sbin`、`/lib` 被解包器换成真目录，`/bin/sh` 不存在 | 三条链接都正确 | 自写 tar 读取器（跟随目录符号链接）+ `tools/guest-config/repair-usrmerge.sh` |
| 假 root | 需要 `NR_FAKE_ROOT=1` 且语义不完整 | 默认开，`getuid`/`chown`/`setuid` 一族统一按容器语义回答 | shim `fake_root_on()`；`NR_FAKE_ROOT=0` 可关 |
| `/proc/{uptime,loadavg,stat,filesystems,locks}`、`/proc/sys/*`、`/proc/net/*` | 全部被 SELinux 拒绝（EACCES） | 由命名空间自己回答（`uptime`/`top`/`free`/`nproc` 正常） | `path-shim` 的 `virt_open` + `sysinfo(2)`/`sched_getaffinity`；常量表标注来源（evidence 107） |
| `/sys/class/net` | 被 passthrough 到宿主，列出 guest 用不了的接口 | guest 根里物化 `lo` + `statistics/*` | `mksys-net.sh` + `passthrough()` 排除 |
| `/dev/log`、`syslog` | 没有接收端，guest 里所有 syslog 失败 | `logger` 能落盘到 `/var/log/syslog`（带 1 MiB 轮转） | 新 crate `runtime/crates/logd`（no_std、静态、无 libc），由启动器拉起 |

## 3. 剩下的墙（FAIL 里没有一项是"懒得做"）

| 被挡的东西 | 原因（实测） | 影响 | 解除条件 |
|---|---|---|---|
| `/proc/stat`、`/proc/loadavg`、`/proc/uptime` | SELinux：`untrusted_app` 读这些文件被拒（`head` 直接 EACCES） | `top`/`htop`/`uptime` 不可用 | 需要非 app 域的读取路径，或由 app 侧用可用的 Android API 合成 |
| `/proc/net/*`、`/sys/class/net`、netlink bind | 同上 + netlink 绑定被拒 | `ss`/`netstat`/`ip addr` 不可用 | 同上 |
| `/proc/sys/*`（`max_map_count`、`pid_max`、`somaxconn`） | 同上 | Redis/PostgreSQL 之类的启动前检查会告警 | 同上 |
| `/sys/fs/cgroup` 列目录 | 同上 | 无法给进程设资源上限 | 需要 cgroup 委派，app 拿不到 |
| SysV IPC（`shmget`/`semget`/`msgget`） | Android seccomp 固定回 `ENOSYS`（38） | 老式共享内存程序不可用（POSIX shm 用 `/dev/shm` 已可用） | 不可解除；只能让程序走 POSIX shm |
| `/dev/mqueue` | 内核没有挂 mqueue 文件系统 | `<mqueue.h>` 不可用 | 需要在 shim 里模拟命名队列 |
| `/dev/full` | 内核不给 app 建节点（无 CAP_MKNOD） | 极少数程序检查它 | shim 可模拟写 `ENOSPC` |
| `/dev/log` | 缺 syslog 数据报 socket | `husk-logd` + `connect`/`bind` 槽位，`logger`/`syslog(3)` 都通 | `runtime/crates/logd` + `libc-interpose.txt`（evidence 107/108） |
| `ping`（raw socket） | 无 `CAP_NET_RAW` | ICMP 探测不可用 | 不可解除（TCP/UDP 正常） |
| `su`/`passwd`（切用户） | 无 `CAP_SETUID` | 单用户运行；多用户服务要改造成同 uid | 不可解除 |

判断：这些是**平台对"零特权容器"的硬边界**，与我们的实现完整度无关。把它们写成表，比假装做到更接近"完整"。

## 4. 与容器/虚拟机的定位差异（一次说清）

- 和 **proot** 比：proot 每次路径调用都要 ptrace 一次；我们零 ptrace，路径改写发生在库入口，
  复杂度 O(1)，并且覆盖 libc 内部调用（evidence 105）。
- 和 **chroot/namespace 容器**比：它们能挂载、能建 cgroup，我们两项都没有；换来的是不需要 root。
- 和 **QEMU 虚拟机**比：我们和宿主共用同一个内核与同一份硬件驱动，没有翻译层，也没有内核启动开销；
  代价是我们暴露在内核的安全策略下（上面那张表就是代价的具体形态）。

## 5. 复现

```sh
adb push tools/device-tests/rt-server-parity.sh /data/local/tmp/
adb shell 'run-as com.linuxhub.husk sh -c "cd files && cp /data/local/tmp/rt-server-parity.sh rootfs-full/tmp/
  && XDG_RUNTIME_DIR=$PWD/rootfs-full/tmp/xdg-probe sh gexec3.sh rootfs-full usr/bin/bash tmp/rt-server-parity.sh"'
```

每行 `R|<area>|<name>|PASS|PARTIAL|FAIL|UNKNOWN|<detail>`，和别人用同样命令应当得到同样结论。

## 6. 已知限制（诚实记账）

* **合成状态库没有文件清单**：tarball 拼出来的包没有 `/var/lib/dpkg/info/<pkg>.list`，
  dpkg 会警告 `files list file ... missing; assuming package has no files currently installed`。
  装包不受影响；`apt-get remove <那些包>` 不会删文件。要根治得按包重建 `.list`，
  数据源只有 tarball 的文件树，暂不做（做了也只是猜）。
* **SysV IPC**（`shmget`/`semget`/`msgget`）与 `mq_open` 被内核 seccomp 直接回 ENOSYS，无解，只能记录。
* ~~`syslog(3)` 本身仍发不出去~~ 已修：`connect`/`bind` 进 libc 槽位表，重编 guest libc（evidence 108）。
* **netlink / cgroup / `/dev/full` / `/dev/mqueue`** 仍是 FAIL：内核拒绝 netlink 与 cgroup 委派；
  `/dev/full` 要拦 `write(2)` 才能造出 ENOSPC，超出本项目"不扩大拦截面"的规则；mqueue 随 `mq_open`。
