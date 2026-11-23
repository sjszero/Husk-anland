# 网络表面：让 `ip`、`ss`、`getaddrinfo` 在 guest 里说人话

> 目标：guest 里的程序能像在普通 Linux 上一样看到网卡、地址、路由，能解析主机名。
> 前提：不 root、不解 BL、不开虚拟化、不扩大 seccomp 拦截面。设备 RMX3357 / 4.19.191 /
> 4 KiB / `untrusted_app`。本文件只讲**内核网络表的读取**（D 段）；发包能力属于平台限制，
> 不在本阶段承诺范围内。

## 0. 先看前人（开工门禁）

| 仓库 | 读到的可复用点 | 不采用的理由 |
|---|---|---|
| ssfuisu/cortex | `app/src/main/cpp/hook/net.c` 里对 `getaddrinfo` 的处理：清 `AI_ADDRCONFIG`，让 `localhost` 在没有全局地址时仍能解析 | 它自述做不了 netlink dump（`ip addr` 这类），而且改的是 bionic 侧 hook；我们要的是 glibc 函数入口 + 内核表 |
| waydroid / redroid | 能力面对照：容器内 `/proc/net/*` 与 netlink 的可用边界、binder/netlink 跨世界桥接的形状 | 它们靠 root / 内核模块 / 真实网络命名空间，我们没有 CAP_NET_ADMIN |
| youki / crun | 无 CAP_NET_ADMIN 时网络命名空间里"什么一定要自己回答"的清单 | 它们只做隔离，不提供表服务 |

参考代码不进 Husk 仓库，笔记在 `husk-notes/prior-art/`。

## 1. 环境事实（FACT，实测）

- 本域**不能建 netlink socket**：`socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE)` → `EACCES`。
  `untrusted_app` 的 seccomp/域策略把这条路径关死了。
- 因此不能靠"拦 syscall 再转"（零 ptrace、不扩大拦截面），唯一的落点是**函数入口 interpose**：
  libc 的 `socket`/`bind`/`send`/`sendto`/`sendmsg`/`recvmsg`/`getsockname`。
- 能用的东西：`AF_INET` 的 `SOCK_DGRAM` 探针 socket + 传统 `ioctl`
  （`SIOCGIFCONF`/`SIOCGIFFLAGS`/`SIOCGIFINDEX`/`SIOCGIFMTU`/`SIOCGIFNETMASK`）在本域都成功。
- `SIOCGIFHWADDR` 被拒 → 拿不到真实 MAC；`/proc/net/dev`、`/proc/net/route` 在本内核不存在。

## 2. 形状

**一对 socketpair 冒充 netlink。** `socket(AF_NETLINK, ...)` 不落内核，改成 `socketpair(AF_UNIX)`：
把一端的 fd 交给调用者（client），另一端（peer）留在 shim 里。调用者以为自己拿到了 netlink 号，
其实是一个已连接的本地数据报端点。

- `send/sendto/sendmsg` → 若 fd 是我们的槽位，`nl_handle()` 同**同步**解析请求，把回复写进 peer。
- `recvmsg` → 加 `#[no_mangle]`（关键：glibc 内部也调 `recvmsg`，不导出就没有 interpose），
  队列空时回 `NLMSG_DONE`（带请求 seq），否则 iproute2 的 `__rtnl_recvmsg` 会对 `EAGAIN` 死循环。
- `getsockname` → 填 `sockaddr_nl`，`nl_pid` 用伪造的 pid（=`getpid`），否则 iproute2 丢弃回复。

**数据从哪来。** `nl_interfaces()` 用探针 socket 的 `SIOCGIFCONF` 拿到接口清单，再对每个接口
`ioctl` 取 flags/index/mtu/netmask；地址来自 `SIOCGIFCONF` 里 `sin_addr`。地址在内部按 host 序存，
上线时 `to_be_bytes`；`SIOCGIFNETMASK` 要 `from_be_bytes`。

**支持的消息。** `RTM_GETLINK`/`RTM_NEWLINK`、`RTM_GETADDR`/`RTM_NEWADDR`、`RTM_GETROUTE`/
`RTM_NEWROUTE`、`NLMSG_DONE`、`NLMSG_ERROR`（非 dump 请求回 16 字节完整 `nlmsgerr` 回显，
短了 iproute2 报 "ERROR truncated"）。

**`/proc/net/{dev,route}` 合成。** `ip` 的部分路径直接读这两个文件；本内核没有，就按 procfs 列布局合成。
`/proc/net/route` 的十六进制是**地址字节的小端序**，输出前 `swap_bytes()`（`192.168.1.0` → `0001A8C0`，
`/24` → `00FFFFFF`）。

**名字服务。** `if_nametoindex`/`if_indextoname` 由 shim 导出（走 `SIOCGIFINDEX`/`SIOCGIFNAME`），
glibc 自己的实现要 bind netlink，本域必 `EACCES`。`getaddrinfo` 崩溃的真因是 **fd 号复用**：
槽位的 fd 被 libc 内部 `close` 掉后，号被 UDP socket 复用，UDP socket 拿到 netlink 回复，
`getsockname` 家族变成 `AF_NETLINK`，触发 glibc `rfc3484_sort` 断言。修法：`nl_is_fake()` 用
`getsockname` 复核 fd 当前家族必须是 `AF_UNIX`，否则 `nl_forget`。

## 3. 已知限制（写清楚，不假装没有）

- MAC 地址全 0（`SIOCGIFHWADDR` 被拒）；链路类型对非 `lo` 报 `ether`，`lo` 报 `loopback`。
- 无 IPv6 地址（`SIOCGIFCONF` 只给 IPv4）。
- 无默认网关（本内核不给，合成表里只列 link-scope 前缀路由）。
- `ss` 的 `sockstat`/`snmpstat` 是 netlink `SOCK_DIAG`，本域 `EACCES` → 只出表头、空表。可接受。
- 只有"读"：`RTM_NEWADDR`/`RTM_NEWROUTE` 这类**改**网络的请求返回 `EPERM`，不假装成功。

## 4. 验收

脚本：`tools/device-tests/09-network-surface.sh`（11 项，全通过）。
详细输出见 `docs/design/evidence/116-netlink-surface-20261123.md`：`ip addr`/`ip route`/`ip link show`/
`ip -br`/`hostname -I`/`/proc/net/{dev,route}`/`if_nameindex`/`getent ahostsv4`/`getaddrinfo` 全通，
`dpkg`/`apt` 无回归。
