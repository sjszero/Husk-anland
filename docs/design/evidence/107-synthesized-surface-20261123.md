# 107 · 服务器表面（二）：自己回答 /proc、自己的网络视图、自己的 syslog 接收端

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`
（全部数字来自 `tools/device-tests/rt-server-parity.sh`，在 guest 里跑）

## 成绩

| 时点 | PASS | PARTIAL | FAIL |
|---|---|---|---|
| 本轮开始（evidence 105 之后） | 22 | 4 | 21 |
| 上一轮（evidence 106） | 58 | 5 | 18 |
| **本轮结束** | **70** | **6** | **6** |

## 1. 为什么必须"自己回答"

FACT（实测，不是猜测）：`untrusted_app` 域读这些一律 EACCES，重试多少次都一样。

```
/proc/loadavg /proc/uptime /proc/stat /proc/filesystems /proc/locks
/proc/net/*    /proc/sys/*    /sys/class/net（目录列举）    netlink(AF_NETLINK) bind
```

容器/虚拟机卖的是"这套约定"，所以由命名空间自己回答。原则：**能诚实读到的就读**，
读不到的用本命名空间的既定默认值，并且在代码注释里写明"这是默认值，不是这台机器的测量"。

## 2. 实现（`path-shim`，纯 Rust，无依赖）

`virt_open()` 现在是"虚拟文件"的统一入口（原来只服务 `/proc/mounts`），新增：

| 文件 | 数据来源 | 说明 |
|---|---|---|
| `/proc/uptime` | `sysinfo(2)` | 真实 uptime |
| `/proc/loadavg` | `sysinfo(2)` | 真实 1/5/15 分钟负载，16.16 定点转两位小数 |
| `/proc/stat` | `sysinfo` + `sched_getaffinity` + `clock_gettime` | 每个 CPU 一行；`btime` 由 `realtime - uptime` 得出；**CPU 时间只记 idle**（这个域拿不到 per-core 忙时，编一个忙时比诚实报零更糟） |
| `/proc/filesystems` | 本运行时支持的集合 + `statfs` 读回的根文件系统类型 | |
| `/proc/locks` | 空表 | 这个域看不到锁表，空表就是真话 |
| `/proc/net/dev`、`/proc/net/route`、`tcp/udp/unix` | `lo` 必在；其余来自 `NR_NETIF` | 见下 |
| `/proc/sys/{vm/max_map_count,kernel/pid_max,net/core/somaxconn,net/ipv4/ip_local_port_range,vm/overcommit_memory,kernel/randomize_va_space}` | 常量表 | 只读；写返回 EACCES（背后没有内核旋钮） |

实测：

```
$ uptime
 10:20:18 up 20:50,  0 user,  load average: 22.14, 22.21, 22.27
$ nproc
8
$ top -b -n1 | head -4
%Cpu(s):  0.0 us,  0.0 sy,  0.0 ni,100.0 id,  0.0 wa,  0.0 hi,  0.0 si,  0.0 st
Tasks:   4 total,   1 running,   3 sleeping,   0 stopped,   0 zombie
```

## 3. 自己的网络视图

`/sys/class/net` 原来被 `passthrough` 送到**宿主**（列出 guest 根本用不了的接口名）。
现在它和 `/dev/log` 一样在 `passthrough()` 里被排除，改为在 guest 根里由
`tools/guest-config/mksys-net.sh` 物化：`lo` 的 `address/type/flags/mtu/operstate/
statistics/*`。`lo` 是唯一能在没有平台输入时诚实描述的接口（loopback 本身实测通）。

`NR_NETIF` 是留给运行时的接口：`name;addr/prefix;mac;name;...`，由 app 侧用
Android 的 `NetworkInterface` 填。**没填就不编**，文件里只有 `lo`。

`netlink` 仍然 FAIL：内核不让这个域 bind `AF_NETLINK`，`ip addr` 无法列举接口。
这不是我们能补的，除非用 ioctl(`SIOCGIFCONF`) 兼容层单独照顾 net-tools。

## 4. `/dev/log`：一个真正的接收端

`syslog(3)` 不是写文件，是用 `AF_UNIX SOCK_DGRAM` 连 `/dev/log` 发数据报。没有接收端，
guest 里所有 `syslog()` 都失败，守护进程无处可写。

新增 `runtime/crates/logd`（`husk-logd`）：

* `no_std` + 静态链接 + **不链接 libc**（12 KB）。原因有两个：给 guest 目标链 glibc
  要求 libc6-dev，而装 libc6-dev 会把运行时自己打过补丁的 libc6 顶掉；并且它刻意只
  用**宿主路径**（`argv[1]` = guest 根），这样 socket 能在任何翻译逻辑起来之前就创建。
* `bind <root>/dev/log` → `chmod 0666` → 循环 `recvfrom` → 追加到 `<root>/var/log/syslog`
  （带 UTC 时间戳，1 MiB 轮转；时间戳自己算，不依赖 tzdata）。
* 已有实例占用 socket 时静默退出 0，所以"每次启动都跑一遍"是安全的。
* 由启动器启动（`gexec3.sh` 里已加；产品启动器同样要在 exec guest 之前起它）。

实测：

```
$ logger -t husk "parity probe"
$ tail -1 /var/log/syslog
2026-11-23T10:25:36Z <13>Nov 23 10:25:36 husk: parity probe
```

**还没完全好的一点（标 PARTIAL，不装好）**：`syslog(3)` 本身仍然发不出去。
glibc 的 `syslog()` 内部调 `__connect`，那是 libc 自己的别名、没有 GOT 槽，
所以没被 interpose；`logger` 能通是因为 util-linux 自己建 socket 自己 connect。
修法与 evidence 105 同路：把 `connect`/`bind` 加进 `tools/loader/libc-interpose.txt`
重编 guest libc，本轮没做。

## 5. 剩下 6 个 FAIL 的定性（都不是"再试一次"能过的）

| 项 | 为什么 | 能不能补 |
|---|---|---|
| `ipc sysv_shm` | 内核 seccomp 直接回 ENOSYS | 不能（除非拦截 syscall，违反本项目硬约束） |
| `ipc posix_mq` | `mq_open(2)` 同上不可用 | 能，但要自己在 shim 里实现整套优先级队列语义，收益低，未做 |
| `net netlink` | 内核拒绝这个域 bind netlink | 只能做 ioctl 兼容层分别照顾 net-tools |
| `sys cgroup` | cgroup 树没有委派给 app | 不伪造：编一棵假的 cgroup 树会让 systemd/工具做出错误判断 |
| `dev /dev/full` | SELinux 拒绝对该节点的 open | 需要拦截 `write(2)` 才能造出 ENOSPC 语义；会扩大拦截面，本项目规则内不做 |
| `dev /dev/mqueue` | 同上，且 mq 不可用 | 随 posix_mq |

## 6. 复跑

```sh
sh tools/guest-config/build-logd.sh             # 编 husk-logd
sh tools/guest-config/mksys-net.sh <rootfs>     # 物化 /sys/class/net/lo
sh files/gexec3.sh <rootfs> usr/bin/bash tmp/rtp.sh   # 70 / 6 / 6
```
