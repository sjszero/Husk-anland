# 264 · fake identity 初始化与 rootfd 并发 close_range 审计

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。对象：`path-shim` 的 `FAKE_ID` 初始化、rootfd 和
`close_range()` wrapper。

## 0 · 结论

FACT：H-52 是代码层竞态。旧 `fake_id()` 只检查 `FAKE_ID_READY == 0`，多个线程
都可以进入初始化分支；线程 A 先完成 `setgid/setuid` 更新后，线程 B 仍可能用
启动时的 `NR_FAKE_UID/GID` 旧值覆盖 `FAKE_ID`。这不是“先置位再写值”的旧问题，
而是“多个初始化者都写”的新窗口。

FACT：最终修复不再在 `getuid/getgid` 的运行路径里懒初始化。`nr_boot` 在任何
构造器或 guest 线程存在之前，从 loader 收到的原始环境一次播种 `NR_FAKE_UID/GID`；
运行期只读原子身份值。这样从结构上删除了“多个运行期初始化者覆盖 setter”的窗口。

FACT：身份登记文件发布锁从布尔自旋锁改为记录 owner TID。若信号处理器打断同一
线程的 `getuid/getgid` 并再次进入发布路径，内层直接返回，外层恢复后重新读取最新
`FAKE_ID` 并发布，不会等待自己。不同线程仍互斥等待。

FACT：修复后新 loader SHA-256 为
`47562cb9f69001eef783ee0d4106ff92023e5ca3a56c9722474541cca3472516`。
真机压力探针使用 15 个 `getuid/getgid` 读线程、1 个 `setgid/setuid` 写线程，并让
`SIGALRM` 处理器高频重入 `getuid/getgid`；20 轮先行验证和 300 轮完整门禁均通过。

## 1 · 负对照与限制

旧 loader 同一压力探针也没有稳定复现失败（300 轮 `fails=0`），所以不能把动态
压力结果写成“已经复现旧竞态”。H-52 的判定来自代码构造；动态探针只作为修复后的
回归门禁，不能替代竞态检测器。

```text
旧 loader（未含 SIGALRM 重入的早期探针）: IDENTITY_RACE rounds=300 fails=0
新 loader（含 SIGALRM 重入，先行）: IDENTITY_RACE rounds=20 fails=0
```

旧 loader 的 `fails=0` 只说明运行期竞态没有稳定命中，不能据此宣称旧实现安全。

新增 `tools/probe/identity-init-race-probe.c` 和
`tools/device-tests/run-identity-init-race-probe.sh`，并接入 `make gate-a`。
探针在每个新进程中制造“身份查询 + 并发改身份 + 信号处理器重入”的交错；每轮都要
求最终身份仍是 setter 写入的 `1000:1000`，且 `SIGALRM` 必须实际送达。

## 2 · rootfd 并发负对照

新增 `tools/probe/rootfd-close-range-probe.c` 和
`tools/device-tests/run-rootfd-close-range-probe.sh`。探针让 8 个线程循环执行
`stat("/etc/os-release")` 和 `access("/usr/bin/bash", X_OK)`，同时 2 个线程循环执行
只指向 rootfd 的 `close_range(1024, 1024, 0)`。结果：

```text
ROOTFD_RACE stat_ok=379 stat_fail=0 access_ok=379 access_fail=0 range_ok=500 range_fail=0 first_errno=0
--- rootfd-close-range-probe: PASS
```

FACT：精确命中 rootfd 范围的并发 `close_range()` 没有把 rootfd 关掉，也没有让
路径操作失败。这里验证的是 wrapper 保护 rootfd 的行为。

作为边界对照，若并发执行 `close_range(3, ~0U, 0)`，路径操作会短暂看到
`ENOTDIR/EBADF`：该调用按 Linux 语义会关闭同一 fd table 中其它线程正在使用的
临时 fd。这不是 rootfd 越界，也不是 Husk 给出的“安全多线程 close_range”承诺。
需要并发关闭时，调用方仍应使用 `CLOSE_RANGE_UNSHARE`（当前 wrapper 明确返回
`EOPNOTSUPP`）或在单线程启动阶段完成。

## 3 · 真机回归

```text
DEV=UKHMMB8PYTGECMNV
PKG=com.linuxhub.myth.husk
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs
LD=files/linux/husk/libld.so
make gate-a
```

结果：

```text
PASS=38 FAIL=0
coverage-probe: PASS
coverage-sweep: SKIP (rootfs 没有 strace)
identity-init-race-probe: PASS rounds=300 fails=0
rootfd-close-range-probe: PASS
supplementary groups: PASS=1 FAIL=0
session locale: PASS
path race: PASS host sentinel unchanged
fd-identity-probe: PASS
```

## 4 · 边界

- FACT：H-52 修复进入当前候选 loader；不是桌面补丁，不修改 LinuxHub 2.0。
- FACT：身份初始化压力是 300 个新鲜进程，每进程 16 个线程。
- UNVERIFIED：没有用 ThreadSanitizer/竞态检测器证明旧实现的失败概率。
- UNKNOWN：其他 Android 版本、16 KiB/64 KiB 页和不同 loader 版本上的同一窗口。
- FACT：`close_range` 并发关闭临时 fd 的 Linux 语义边界仍然存在；不要把这条负对照
  写成“所有并发 close_range 都安全”。
