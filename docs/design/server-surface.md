# 运行时表面：一台服务器该有什么，我们有什么

> 参考设备：RMX3357（Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页 / `untrusted_app`，无 root、无 Shizuku）。
> 测量入口：`tools/device-tests/run-server-parity.sh`，2026-11-27 真机复跑。
> 当前结果：**77 PASS / 5 PARTIAL / 7 FAIL / 1 UNKNOWN，共 90 项**。
> 编排依赖：Shell；libc 与内核接口检查：Rust `runtime-probe server-parity`。guest 内 Python 和 awk 依赖均为 **0**。

## 1. 对照口径

容器和虚拟机提供的不是一个 shell，而是一套可预期的 Linux 用户态约定。这里拆成八层逐项测：

| 层 | 正常 Linux 环境应提供什么 | Husk 当前落地方式 |
|---|---|---|
| 装载 | ELF 加载、动态链接、TLS、auxv、vdso | 修改后的 glibc loader，无 ptrace |
| libc | 路径、进程、网络和锁语义可从 libc 入口获得 | GOT 重写 + 编译期插槽 + Path Shim |
| 文件系统 | 独立根、挂载视图、常规文件与 xattr | 单目录根 + 虚拟挂载表 + 路径改写 |
| 进程 | fork/exec、PTY、信号、僵尸回收 | 真机回归覆盖 |
| IPC | UNIX socket、SCM_RIGHTS、事件通知、SysV/POSIX IPC | UNIX/eventfd/timerfd/epoll 直通；SysV/POSIX 队列由 shim 仿真 |
| 网络 | socket、DNS、监听、回环、UDP、IPv6 | 真机通过；raw ICMP 受平台权限限制 |
| 设备 | `/dev` 常规节点、pty、shm | 现有节点通过；`/dev/full` 与 `/dev/log` 在当前树中缺失 |
| 服务 | system/session D-Bus、syslog、init | system/session bus 通过；测试树未部署 logd，syslog 两项失败 |

## 2. 本轮修复

| 项 | 修前 | 修后 | 验证 |
|---|---|---|---|
| server parity 编排 | 脚本含 26 处 `python3`，并依赖 2 处 `awk`；当前 guest 没有 `awk`、`logger`、`hostname`、`ping` | 新增 `runtime-probe server-parity`，覆盖 NSS、锁、xattr、稀疏文件、mmap、UNIX socket、SCM_RIGHTS、IPC、socket、D-Bus、syslog 和 dpkg 状态解析 | 真机 90 项完整跑完，guest 内 Python/awk 调用为 0 |
| hardlink 重复执行 | 上一轮残留 `/var/tmp/hl/b` 会让下一轮 `link(2)` 假失败 | 每轮先清理测试目录 | 连续两轮结果稳定为 PARTIAL，`nlink=1` |
| mount 重复执行 | bind/tmpfs 测试目录可能残留旧文件 | 每轮先清理测试目录，再挂载/卸载 | bind、tmpfs、ro bind 连续通过 |
| PTY 静态检查 | 没有活动终端时把 `/dev/pts/0` 不存在误报为 FAIL | 改查 `/dev/pts` 目录，并保留 `openpty` 实际调用 | `/dev/pts` 与 `ipc pty` 均 PASS |
| ld.so.cache | 辅助件缺失时被算成运行时 FAIL | 缺少独立 reader 或 cache 时明确记 UNKNOWN | 本轮为 UNKNOWN，不冒充通过或失败 |

## 3. 当前矩阵

PASS 覆盖：

- 身份与账户：模拟 root、`getpwnam/getgrnam`、`useradd/groupadd/usermod -aG`；
- 文件系统：`flock`、POSIX lock、inotify、xattr、`O_TMPFILE`、1 GiB sparse truncate、`MAP_SHARED + msync`；
- IPC：AF_UNIX pathname/abstract、SCM_RIGHTS、eventfd、timerfd、epoll、PTY、FIFO、SysV shm、POSIX mqueue；
- 网络：DNS/getaddrinfo、TCP outbound/listen/loopback、UDP、IPv6、netlink bind；
- 系统视图：`/proc/stat`、`loadavg`、`uptime`、`filesystems`、`locks`、`/proc/sys/*`、`/proc/net/*`；
- 包管理：dpkg、apt、索引、时区、locale、包状态一致；
- 服务：system D-Bus、session D-Bus。

PARTIAL：`setuid`、hardlink、raw ICMP、systemd、传统 `su`。这些要么缺内核 capability，要么只提供子集。

FAIL（7）：

| 项 | 实测 | 分类 |
|---|---|---|
| `/sys/class/net` | 当前 guest 视图没有可枚举目录 | 系统视图缺口 |
| `/proc/version` | `untrusted_app` 读取被拒 | 平台权限边界 |
| `/sys/fs/cgroup` | 列目录被拒 | 平台权限边界 |
| `/dev/full` | 节点不存在 | 设备视图缺口 |
| `/dev/log` | 节点不存在 | 当前测试树未部署 logd |
| `svc syslog` | 没有接收端，发送失败 | 同上 |
| `svc syslog_libc` | `syslog(3)` 无可连接 socket | 同上 |

UNKNOWN（1）：`ld.so.cache`。当前测试树没有 `/usr/sbin/ldconfig.real.glbc` 和 `/etc/ld.so.cache`，无法独立验证，不能写成 PASS。

## 4. 边界判断

- SysV shm、SysV sem/msg 和 POSIX mqueue 在本机内核上并不直接可用；guest 看到的是 Husk 的仿真实现。它们对普通 Linux 程序可用，但不能让应用内的 bionic X server 直接使用 guest 侧 IPC。
- `/proc/version` 与 cgroup 枚举是 `untrusted_app` 权限边界；本轮没有绕过。
- `/dev/full` 和 `/dev/log` 属于设备/服务视图缺口，不是“所有 Android 设备必然没有”。要看具体发行树是否包含对应节点和守护进程。
- raw ICMP 需要 `CAP_NET_RAW`，本机明确返回 `EPERM`；TCP/UDP 不受影响。
- hardlink 在当前内核策略下退化为复制，`nlink` 仍为 1，所以只能记 PARTIAL。
- Path Shim 是 Linux 用户态兼容层，不是恶意代码安全沙箱。静态 ELF、raw syscall 和绕过 loader 的进程不受 GOT 改写控制。

## 5. 复现

```sh
DEV=<adb-serial> \
ROOTFS=files/rootfs-full \
LD=files/linux/husk/libld.so \
sh tools/device-tests/run-server-parity.sh
```

或者：

```sh
DEV=<adb-serial> make server-parity
```

每行协议固定为：

```text
R|<area>|<name>|PASS|PARTIAL|FAIL|UNKNOWN|<detail>
```

`DONE` 表示矩阵完整跑完，不表示所有项通过。

## 6. 已知限制

- 本轮没有 bump `VERSION`、没有打 tag、没有发 Release。
- 测试结果只覆盖当前 RMX3357 的 Android 13 / 4.19.191 / 4 KiB 环境，不能据此宣称所有 Android 版本、页大小和 GPU 家族一致。
- 当前 rootfs 是测试树，缺 logd 和独立 ldconfig reader；这两类结果必须和真实发行树结果分开记录。
- SysV/POSIX IPC 是用户态仿真，不拥有真实内核 IPC 的跨域可见性、资源配额和安全属性。
