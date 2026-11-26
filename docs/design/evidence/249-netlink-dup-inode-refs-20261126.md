# 249 · 假 netlink fd 的 dup/dup2/fcntl 复制与最后引用清理

日期：2026-11-26 11:00（CST）。设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB。
探针：`tools/probe/coverage-probe.c`。

## 0 · 修前事实（0.0.22）

```text
NETLINK_DUP before=6 opened=9 family0=16 family_dup=1 family_after_original_close=1 after=6
```

FACT：

- 原始 netlink socket 的 `getsockname()` 家族是 `AF_NETLINK(16)`。
- `dup()` 出来的 fd 被报成 `AF_UNIX(1)`，说明它没有被 netlink 模拟层识别。
- 关闭原 fd 后，复制 fd 仍是 `AF_UNIX(1)`；同时 `close(original)` 会注销槽位并关闭 shim 私有 peer，复制 fd 即使保留也无法继续 netlink 通信。

根因：`NL_SLOTS` 只按“原始 client fd 号”查找。`dup`/`dup2`/`fcntl(F_DUPFD*)` 产生的新 fd 天然不在表里；
关闭原 fd 又把共享的 peer 当成最后一个引用清掉。

## 1 · 修法

`runtime/crates/path-shim/src/lib.rs`：

1. netlink 槽位以 client / peer 的 socket inode 作为身份，不再以 fd 号作为身份。
2. `nl_slot()` 先按 fd 当前 inode 查槽位，因此 `dup`、`dup2`、`fcntl(F_DUPFD_CLOEXEC)` 都自动命中同一模拟 socket。
3. `nl_forget()` 计算当前进程里同一 client inode 的引用数；只有关闭最后一个 live 引用时才注销槽位并关闭私有 peer。
4. peer 仍要 inode 校验后才关闭，避免 fd 号复用误关无关对象。
5. `close_range()` 与 netlink 引用计数共用 `max_live_fd()`，从 `/proc/self/fd` 取得真实上界，不写死 1024。

## 2 · 修后真机输出（0.0.23，FACT）

```text
NETLINK_DUP before=6 opened=11 families=16,16,16,16 dup=16,16,16 final=16,16,16 after=6
CLOSE_RANGE_NETLINK before=6 opened=8 rc=0 errno=0 after=6
CLOSE_RANGE_ALL child=27735 exit=0 exists=1
```

解释：

- `families` 四个值对应 original、`dup`、`dup2`、`fcntl(F_DUPFD_CLOEXEC)`，全部是 `AF_NETLINK`。
- 依次关闭 original、dup、dup2 后，剩余复制 fd 仍保持 `AF_NETLINK`。
- 关闭最后一个引用后，fd 数回到 6：client 与私有 peer 都没有泄漏。
- `close_range(3,~0U)` 子进程仍能打开 guest 路径，root fd 保留。

## 3 · 回归

```text
coverage-probe: PASS
PASS=38 FAIL=0
```

`cargo check --target aarch64-unknown-linux-gnu -p path-shim` 通过。

## 4 · 边界

- UNKNOWN：通过 `SCM_RIGHTS` 把假 netlink fd 传给另一个进程后，对方进程没有本地槽位，当前不保证继续模拟。
- UNKNOWN：多线程同时 dup/close 同一个 netlink inode 的极端竞态未做专项压力测试。
- FACT：引用计数按当前进程 `/proc/self/fd` 的 live fd 扫描，不修改内核 fd 语义；`CLOSE_RANGE_UNSHARE` 的既有边界不变。
