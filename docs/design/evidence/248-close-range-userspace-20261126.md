# 248 · `close_range`：seccomp 拦截后的用户态兜底，以及 netlink peer 泄漏

日期：2026-11-26 10:54（CST）。设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB。
被测 loader：0.0.22 候选版。完整输出来自 `tools/probe/coverage-probe.c`。

## 0 · 修前事实

0.0.21 的 `close_range()` 在真机上直接返回错误：

```text
CLOSE_RANGE_NETLINK before=6 opened=8 rc=-1 errno=38 after=8
CLOSE_RANGE_PIPE pipe=0 rc=-1 errno=38 fd0=0 fd1=0
CLOSE_RANGE_CLOEXEC pipe=0 rc=-1 errno=38 fd0=0 fd1=0
```

单独探针连续 8 轮也保持相同结果：

```text
start=5
round=0 fd=3 rc=-1 errno=38 fds=7
round=1 fd=5 rc=-1 errno=38 fds=9
...
round=7 fd=17 rc=-1 errno=38 fds=21
```

FACT：`errno=38` 是 `ENOSYS`。Android app seccomp 拦截 syscall 436，loader 的 SIGSYS 处理器按既有策略把它
翻译成 `ENOSYS`；glibc 的 `close_range()` 没有继续回退，所以调用整体失效。

FACT：netlink 假 socket 每轮实际新增两个 fd（client 和 shim 私有 peer），批量关闭又没执行，fd 数线性增长。

## 1 · 同时发现的第二个缺陷

修前 `close_range` 实现即使在内核允许 syscall 436 的环境里，也只清理 DRM / Adreno 记账：

- V4L2 假 fd 的 `CTX` 不知道 fd 已被批量关闭；fd 号复用后可能把无关 fd 当成相机。
- netlink 槽位同样会留下 stale client。
- `nl_forget()` 只释放槽位，不关闭 shim 私有的 peer，单 fd `close()` 也会每次泄漏一个 fd。

这些不是 seccomp 的锅，是 Husk 自己的记账缺口。

## 2 · 修法

`runtime/crates/path-shim/src/lib.rs`：

1. `close()` 与 `close_range()` 共用 `forget_fake_fd()`，统一注销 V4L2、netlink、DRM、Adreno 和硬链接别名状态。
2. `close_range()` 不再依赖被 seccomp 拦截的 syscall 436。先枚举 `/proc/self/fd` 取当前最大 fd，再按范围执行
   原始 `close(57)` 或 `fcntl(F_SETFD, FD_CLOEXEC)`；`3..UINT_MAX` 不会变成四十多亿次循环。
3. 内部 root fd 始终跳过，保证关闭全部 fd 后路径解析仍可用。
4. `CLOSE_RANGE_CLOEXEC` 只设置标志，不注销仍在使用的假 fd。
5. `CLOSE_RANGE_UNSHARE` 明确返回 `EOPNOTSUPP`，不假装完成了无法模拟的 fd-table unshare。
6. netlink 槽位记录 client / peer 的 socket inode，fd 号复用后不会误认或误关无关 fd。
7. `nl_forget()` 只在 peer inode 仍匹配时关闭私有 peer，消除 netlink fd 泄漏。

## 3 · 修后真机输出（FACT）

```text
CLOSE_RANGE_NETLINK before=6 opened=8 rc=0 errno=0 after=6
CLOSE_RANGE_PIPE pipe=0 rc=0 errno=0 fd0=-1 fd1=-1
CLOSE_RANGE_CLOEXEC pipe=0 rc=0 errno=0 fd0=1 fd1=1
CLOSE_RANGE_UNSHARE rc=-1 errno=95
CLOSE_RANGE_ALL child=26090 exit=0 exists=1
```

解释：

- netlink socket 打开时从 6 增至 8，关闭后回到 6：client 与 peer 都没有残留。
- 普通 pipe 两端 `F_GETFD=-1`：确实关闭。
- `CLOEXEC` 两端 `F_GETFD=1`：fd 仍打开且带 `FD_CLOEXEC`。
- `UNSHARE` 明确失败，不伪装成功。
- 子进程执行 `close_range(3,~0U)` 后仍能通过 guest 路径创建文件，说明 root fd 保留、枚举范围收敛。

## 4 · 回归

```text
PASS=38 FAIL=0
coverage-probe: PASS
coverage-sweep: PASS（1400 条候选路径，无未翻译 guest 绝对路径）
```

`cargo check --target aarch64-unknown-linux-gnu -p path-shim` 通过。构建产物 interpose 表为 199 项。

## 5 · 未验证 / 边界

- UNKNOWN：多线程并发关闭与同时 open 的竞态压力；本次未做高并发 fd churn。
- UNKNOWN：V4L2 相机 fd 的 `close_range` 动态用例。清理入口已接入，但本轮没有打开相机设备。
- UNKNOWN：Adreno 真机上的 `close_range` 回归；本机只有 Mali。
- FACT：`CLOSE_RANGE_UNSHARE` 仍不支持。调用方若强依赖 fd-table unshare，会看到 `EOPNOTSUPP`，不会得到假成功。
