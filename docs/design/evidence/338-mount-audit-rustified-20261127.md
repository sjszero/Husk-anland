# 338：mount 审计中的 Python 检查迁入 Rust

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页

## 1. 修前问题（FACT）

`tools/device-tests/05-mount-audit.sh` 的 mount 编排本身是 shell，但仍把下列
需要直接 libc 调用的检查交给 Python：

- read-only bind 的 fallback；
- dirfd 下的 openat/mkdirat/unlinkat/fchmodat；
- chmod/chown/utimensat/truncate/setxattr；
- 六个线程 stat 期间并发 mount/umount churn；
- setxattr/getxattr/removexattr 往返。

当前测试树没有 `awk` 和多个常见工具，不能继续把 Python 或其它非运行时依赖当成
这些语义检查的前置条件。

## 2. 改动（FACT）

- `runtime-probe mount-audit` 新增四个入口：
  `mount-bind-ro`、`mount-write`、`mount-race`、`xattr PATH`。
- 只读写入逐项要求 `EROFS(30)`；成功返回的 fd 会立即关闭并计为失败。
- 并发检查使用 1 个 mount/umount churn 线程和 6 个 stat worker；每个 worker
  扫描 3000 轮，覆盖绝对路径、相对路径、目录和 `/proc/self/cmdline`。
- 修前文件中共有 12 处 `python3` 调用；迁移后为 0。
- `05-mount-audit.sh` 保留 tmpfs、长路径、mount table 和 symlink 这些 shell
  编排，只调用 Rust 探针完成 libc 级检查。
- 新增 `run-mount-audit.sh`，负责临时 loader、runtime-probe、脚本入场和清理。

## 3. 修后正向结果（FACT）

真机执行 `run-mount-audit.sh`：

```text
PASS openat write EROFS
PASS mkdirat EROFS
PASS unlinkat EROFS
PASS fchmodat EROFS
PASS fchownat EROFS
PASS utimensat EROFS
PASS chmod EROFS
PASS chown EROFS
PASS truncate EROFS
PASS setxattr EROFS
PASS mount churn: no resolution errors
PASS xattr set/get/remove
...
PASS=21 FAIL=0
```

当前 runtime-probe SHA-256：

```text
c96c02b70d207b5518b49311f46ec34fdb4207d76ff197c0ecaa8df0c2281e21
```

完整回归：

```text
NDK=<ndk>/bin make check
PASS（整 workspace、Clang 格式检查、shell 语法、发布契约和文档链接）

DEV=<serial> ROOTFS=files/rootfs-full LD=files/rootfs-full/root/libld-test.so make gate-a
PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: PASS（1089 条候选，0 漏网）
fd-identity-probe: PASS
supplementary-groups: PASS=1 FAIL=0
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
rootfd-close-range: PASS
path-race: PASS
```

原先按组汇总时报告 `PASS=18`；本轮把组内每一项分别计数，所以总项数变为 21，
不是新增了未被验证的功能。

## 4. 边界（FACT / UNVERIFIED）

- 该检查验证 Husk 的 mount 语义层，不宣称 Android app 拥有真实 `CAP_SYS_ADMIN`。
- 本轮没有 bump `VERSION`、没有 tag、没有发 Release。
- 测试结束后 runtime-probe、脚本、临时 loader 和 `/data/local/tmp` 副本均清理。
