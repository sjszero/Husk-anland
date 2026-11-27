# 339：server-surface 矩阵去掉 guest Python/awk

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页

## 1. 修前问题（FACT）

`tools/device-tests/rt-server-parity.sh` 是运行时表面矩阵，但把 libc 语义检查交给了
guest 内的 Python one-liner，另有 2 处 `awk` 解析。静态计数：

```text
python3=26
awk=2
```

同一测试树的实际工具可用性：

```text
$ for t in awk logger hostname ping; do command -v "$t" || echo "$t=MISSING"; done
awk=MISSING
logger=MISSING
hostname=MISSING
ping=MISSING
```

因此旧矩阵有两类不可复现问题：

- Python/awk 缺失时，检查失败反映的是测试工具缺失，不是 Linux 运行时能力；
- 这条矩阵依赖的 `logger`、`hostname`、`ping` 也不存在，相关项无法独立复现。

重复执行还有一个自己的缺陷：hardlink 测试固定写 `/var/tmp/hl/a` 和 `/var/tmp/hl/b`，
第二轮先创建 `b` 时不会被清理，`link(2)` 会撞已存在目标，产生假 FAIL。

## 2. 改动（FACT）

- 新增 `runtime-probe server-parity`，覆盖：
  - `getgrnam` 验证 `usermod -aG`；
  - `flock`、POSIX `F_SETLK`；
  - `inotify_init1`、xattr set/remove、`O_TMPFILE`、1 GiB sparse truncate、`MAP_SHARED + msync`；
  - AF_UNIX pathname/abstract、`SCM_RIGHTS`；
  - SysV shm、POSIX mqueue、eventfd、timerfd、epoll、PTY；
  - DNS、TCP outbound/listen/loopback、UDP、IPv6、netlink bind、hostname、raw ICMP；
  - session D-Bus 启动与回收、`syslog(3)`；
  - CapEff 解析、dpkg 半配置状态解析。
- `rt-server-parity.sh` 只保留 Shell 编排和 `R|...` 输出协议，guest 内 Python/awk 调用均归零。
- 修复重复执行：
  - hardlink 和 bind/tmpfs 测试目录每轮先清理；
  - `/dev/pts/0` 的静态存在性检查改为 `/dev/pts` 目录，PTY 实际能力仍由 `openpty` 检查；
  - 独立 ldconfig reader/cache 缺失时记 `UNKNOWN`，不再冒充运行时 FAIL。
- 新增 `tools/device-tests/run-server-parity.sh` 和 `make server-parity`。
- `make check` 与 CI 的 host test 列表加入 `runtime-probe`，确保新解析逻辑有单测。

## 3. 修后验证（FACT）

### 3.1 编译与单测

```text
cargo check --tests -p runtime-probe --target aarch64-unknown-linux-gnu
PASS

cross-built runtime-probe:
Type: DYN (Shared object file)
Entry point: 0x95D80
SHA-256: 37afb055125dd57b298defcbeb10145cd403aaf3d8287e3e39acbc81558c907c
```

Rust 测试在真机通过 Husk loader 执行，6/6：

```text
running 6 tests
test egress::tests::accepts_complete_http_response ... ok
test egress::tests::rejects_non_http_response ... ok
test egress::tests::rejects_short_send ... ok
test server_parity::tests::cap_eff_parser_ignores_other_lines ... ok
test server_parity::tests::dbus_parser_accepts_address_then_pid ... ok
test server_parity::tests::dpkg_parser_matches_the_old_awk_rule ... ok
TEST_RC=0
```

### 3.2 真机完整矩阵

```text
DEV=UKHMMB8PYTGECMNV \
ROOTFS=files/rootfs-full \
LD=files/linux/husk/libld.so \
sh tools/device-tests/run-server-parity.sh
```

汇总：

```text
PASS=77
PARTIAL=5
FAIL=7
UNKNOWN=1
total=90
DONE
```

同一入口经 `DEV=<serial> make server-parity` 再跑一次，退出码 0，汇总仍为 77/5/7/1。

关键正向项：

```text
R|id|usermod_member|PASS|usermod -aG seen through getgrnam
R|fs|flock|PASS|exclusive flock
R|fs|posixlock|PASS|fcntl lock
R|fs|xattr|PASS|user. xattr on /tmp
R|fs|sparse|PASS|1 GiB truncate
R|fs|mmap_shared|PASS|MAP_SHARED + flush
R|ipc|unix_stream|PASS|AF_UNIX pathname
R|ipc|unix_abstract|PASS|abstract namespace
R|ipc|fdpass|PASS|SCM_RIGHTS
R|ipc|sysv_shm|PASS|shmget
R|ipc|posix_mq|PASS|mq_open
R|ipc|eventfd|PASS|eventfd
R|ipc|timerfd|PASS|timerfd_create
R|ipc|epoll|PASS|epoll
R|ipc|pty|PASS|openpty (/dev/pts)
R|net|getaddrinfo|PASS|glibc resolver + nss_dns
R|net|tcp_outbound|PASS|TCP to a public address
R|net|tcp_listen|PASS|bind 0.0.0.0 (unprivileged port)
R|net|loopback|PASS|loopback
R|net|udp|PASS|UDP loopback
R|net|ipv6|PASS|AF_INET6
R|net|hostname|PASS|localhost
R|net|netlink|PASS|netlink bind (ip addr enumerates interfaces)
R|svc|dbus_session|PASS|session bus starts
R|pkg|db_consistent|PASS|0 package(s) not fully configured
```

连续两轮 hardlink 结果稳定：

```text
R|fs|hardlink|PARTIAL|PASS nlink=1 (kernel result measured)
```

### 3.3 仍然不利的结果

FAIL（7）：

```text
R|net|sys_class_net|FAIL|
R|proc|/proc/version|FAIL|denied to untrusted_app
R|sys|cgroup|FAIL|cgroup tree (listing denied => no limits)
R|dev|/dev/full|FAIL|missing
R|dev|/dev/log|FAIL|missing
R|svc|syslog|FAIL|syslog ->
R|svc|syslog_libc|FAIL|
```

PARTIAL（5）：`setuid`、hardlink、raw ICMP、systemd、传统 `su`。

UNKNOWN（1）：`pkg ld_cache|independent cache reader or cache absent`。


### 3.4 完整回归

```text
NDK=<ndk>/bin make check
PASS（整 workspace、Clang 检查、shell 语法、发布契约和文档链接）
checked 366 markdown files, 0 broken link(s)
version ok: 0.0.36

DEV=UKHMMB8PYTGECMNV \
ROOTFS=files/rootfs-full \
LD=files/rootfs-full/root/libld-test.so \
make gate-a
PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: PASS（没有发现未翻译的 guest 绝对路径）
fd-identity-probe: PASS
supplementary-groups: PASS=1 FAIL=0
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
rootfd-close-range: PASS
path-race: PASS
```

临时 loader、runtime-probe、测试脚本和 `/data/local/tmp` 副本均已清理。

## 4. 解释边界（FACT / INFERENCE）

- SysV shm、SysV sem/msg 与 POSIX mqueue 的 PASS 来自 Husk 的仿真层；这不等于
  Android 内核原生提供这些 IPC，也不等于应用内 bionic X server 能直接使用 guest IPC。
- 当前 `rootfs-full` 测试树没有 `/dev/log`，因此 syslog 两项 FAIL；这是测试树缺少
  logd/设备节点，不是“已验证可用”。对应组件和发行树仍需重新部署后复验。
- `ld_cache` 记 UNKNOWN，因为当前树没有 `/usr/sbin/ldconfig.real.glbc` 和
  `/etc/ld.so.cache`，没有独立 reader 可测。
- raw ICMP 的实际结果是 `EPERM`；TCP/UDP/HTTPS 不受影响。
- 本轮没有 bump `VERSION`、没有 tag、没有发 Release。
- runner 退出时清理 runtime-probe、脚本和 `/data/local/tmp` 副本。
