# evidence 116 — 网络表面：`ip` / `ss` / 主机名解析在 guest 里可用

设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`。2026-11-23。
设计：`docs/design/network.md`。装载：loader `libld39.so` 460288 B（含 6 个新 interpose 入口）。

## 1. 起点（FACT）

```
socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE)   -> -1 errno=13 EACCES
ip addr                                        -> 空（没有 netlink，也没有 /proc/net/{dev,route}）
```

`/proc/net/dev`、`/proc/net/route` 在本内核的 guest 视图里不存在；`SIOCGIFHWADDR` 被拒。
可用的只有 `AF_INET` 探针 socket 上的传统 ioctl。

## 2. 改了什么

- libc 入口 interpose：`socket`、`bind`、`send`、`sendto`、`sendmsg`、`recvmsg`、`getsockname`
  （`tools/loader/libc-interpose.txt` 现 108 行）。`recvmsg` 必须 `#[no_mangle]` 导出，
  否则 glibc 内部调用绕开 interpose —— 这是"回复读不到"的根因。
- `AF_NETLINK` 的 `socket()` 改成 `socketpair(AF_UNIX)`：一段交给调用者，一段留在 shim 里回话。
- 回复的 `nlmsg_pid` 用伪造 pid（= `getpid`）；dump 读空回 `NLMSG_DONE`（带 seq），
  否则 iproute2 的 `__rtnl_recvmsg` 对 `EAGAIN` 死循环；非 dump 请求回 16 字节完整 `nlmsgerr`。
- 数据来自 `SIOCGIFCONF` + 每接口 `SIOCGIFFLAGS/INDEX/MTU/NETMASK`。
- `/proc/net/{dev,route}` 从同一份数据合成；`route` 的十六进制按字节小端序输出。
- `if_nametoindex` / `if_indextoname` 由 shim 导出（`SIOCGIFINDEX` / `SIOCGIFNAME`）。
- **写动词不再假成功**：`RTM_NEW*/DEL*/SET*` 回 `EPERM`；`ip route get`（需要 FIB 查找）
  与 `RTM_GETNEIGH` 等未合成的读表面回 `EOPNOTSUPP`。

## 3. 实测（FACT，原样输出）

```
$ ip -br addr
lo               UP             127.0.0.1/8
wlan0            UP             192.168.1.5/24

$ ip addr
1: lo: <LOOPBACK,UP> mtu 65536 state UP qlen 1000
    link/loopback 00:00:00:00:00:00
    inet 127.0.0.1/8 scope link dynamic lo
34: wlan0: <BROADCAST,MULTICAST,UP> mtu 1500 state UP qlen 1000
    link/ether 00:00:00:00:00:00
    inet 192.168.1.5/24 scope link dynamic wlan0

$ ip route
127.0.0.0/8 dev lo proto kernel scope link src 127.0.0.1
192.168.1.0/24 dev wlan0 proto kernel scope link src 192.168.1.5

$ ip link show lo            # ifi_index=0 + IFLA_IFNAME 点名
1: lo: <LOOPBACK,UP> mtu 65536 state UP qlen 1000
    link/loopback 00:00:00:00:00:00

$ hostname -I
192.168.1.5

$ cat /proc/net/dev
  lo: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
  wlan0: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0

$ cat /proc/net/route
lo	0000007F	00000000	0001	0	0	0	000000FF	0	0	0
wlan0	0001A8C0	00000000	0001	0	0	0	00FFFFFF	0	0	0

$ getent ahostsv4 localhost
127.0.0.1       STREAM localhost
127.0.0.1       DGRAM
127.0.0.1       RAW

$ python3 -c 'getaddrinfo("localhost",80,TCP)'
('::1', 80, 0, 0)

$ ss -lnt
State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess      # 表头，空表（见限制）
```

if_nameindex：`lo=1`、`wlan0=34`，`if_indextoname` 双向一致。

写动词（改前是静默成功 + 什么也没改，现在是明确拒绝）：

```
$ ip addr add 10.9.9.9/24 dev wlan0     -> RTNETLINK answers: Operation not permitted
$ ip route add default via 192.168.1.1   -> RTNETLINK answers: Operation not permitted
$ ip link set lo down                   -> RTNETLINK answers: Operation not permitted
$ ip route get 8.8.8.8                  -> RTNETLINK answers: Operation not supported
$ ip neigh                              -> RTNETLINK answers: Operation not supported
```

## 4. 回归（同一次构建）

```
tools/device-tests/09-network-surface.sh  PASS=11 FAIL=0   # 本文件 §3 的机器可复现版
tools/device-tests/05-mount-audit.sh      PASS=11 FAIL=0
06-posix-mqueue.py                     RESULT PASS
07-sysv-shm.py                         RESULT PASS
08-sysv-sem-msg.py                     RESULT PASS
dpkg -l | grep -c '^ii'                1193
apt-get -s install sl                  Inst sl 5.02-1 / Conf sl 5.02-1
```

## 5. 已知限制（明确写，不假装没有）

- MAC 全 0：`SIOCGIFHWADDR` 在本域被拒。链路类型对非 `lo` 报 `ether`，`lo` 报 `loopback`。
- 无 IPv6 地址：`SIOCGIFCONF` 只给 IPv4。
- 无默认网关：本内核不提供，合成表只有 link-scope 前缀路由。
- `ss` 的 `sockstat`/`snmpstat` 走 `NETLINK_SOCK_DIAG`，本域 `EACCES` → 空表。
- `ip -s link` 无计数器（没有 `IFLA_STATS64` 来源）。
- 只有读：任何改网络的请求都拒绝，符合"不假装有 CAP_NET_ADMIN"。

## 6. 一次修复的实证（为什么 `nl_is_fake` 要复核家族）

fd 号复用会让 glibc `getaddrinfo` 崩溃：槽位 fd 被 libc 内部 `close`（不走我们的 `close`），
号被后来的 UDP socket 复用，UDP socket 收到 netlink 回复后 `getsockname` 报 `AF_NETLINK`，
`rfc3484_sort` 断言炸。`nl_is_fake()` 现在用 `getsockname` 复核家族必须 `AF_UNIX`，否则丢弃槽位。
修复后 `getent ahostsv4 localhost` 与 `getaddrinfo` 稳定，无崩溃。
