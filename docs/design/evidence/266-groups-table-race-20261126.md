# 266 · `getgroups()` 组表竞态与越界写修复

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。对象：`path-shim` 的虚构补充组表、`getgroups()`、
`setgroups()` 和 `initgroups()`。

## 0 · 结论

FACT：旧 `getgroups()` 存在确定的内存安全缺陷。它先在锁内读一次组数量并解锁，
用这个旧数量检查调用方缓冲区，随后再次加锁并从可能已经变长的 `FAKE_GROUPS_N`
复制组表。另一个线程若在两次读取之间执行 `setgroups()`，`getgroups(1, buf)` 可以
向只有一个 `u32` 的缓冲区写入最多 32 个 `u32`。

FACT：旧 loader 负对照已经动态命中该缺陷。20 个新鲜进程中有 19 个输出
`GROUPS_RACE_FAIL what=buffer-overrun`，并伴随 `*** stack smashing detected ***`。
这不是理论推断，也不是只靠静态审阅下的结论。

FACT：修复后，长度读取、缓冲区校验和复制全部在同一个组表锁区间内完成；锁从
布尔自旋锁改为记录 owner TID 的锁。同一线程的信号处理器重入时不会等待自己，
而是返回只含主组的非阻塞快照。新 loader 同一探针 20/20 轮通过。

## 1 · 旧实现窗口

旧顺序可以简化为：

```text
n = fake_groups()          // 锁内读长度，随后解锁
if size < n: EINVAL        // 用旧 n 校验
lock()
copy FAKE_GROUPS_N         // 重新读取，可能已从 1 变成 32
unlock()
```

`setgroups()` 与 `getgroups()` 在同一进程的不同线程里同时调用时，组表长度不是稳定值。
一旦复制阶段读到更大的新长度，缓冲区边界已经失效。

## 2 · 修复

- `FAKE_GROUPS_LOCK` 从 `AtomicBool` 改为 owner TID 的 `AtomicI32`。
- `getgroups()` 在同一个锁区间内完成首次组表播种、读取长度、检查 `size` 和复制。
- `size == 0` 的计数查询也在同一锁区间内返回，不再先取旧快照。
- 同一线程在信号处理器里重入时返回 `{主组}`，不读取正在更新的补充组表，也不自旋等待自己。
- `fake_has_gid()`、`setgroups()`、`initgroups()` 全部适配新的 owner-TID 锁。
- 新增 `tools/probe/groups-race-probe.c`：2 个读线程执行带前后 canary 的
  `getgroups(1, ...)`，1 个写线程在 1 组和 32 组之间切换，`SIGALRM` 每毫秒重入
  `getgroups()`。任何 canary 变化、非 `EINVAL` 错误、异常计数或超时都失败。
- 新增 `tools/device-tests/run-groups-race-probe.sh` 并接入 `make gate-a`，默认
  20 个新鲜进程。

## 3 · 负对照与修后结果

旧 loader：

```text
SHA-256 47562cb9f69001eef783ee0d4106ff92023e5ca3a56c9722474541cca3472516
GROUPS_RACE rounds=20 fails=19
GROUPS_RACE_FAIL what=buffer-overrun
*** stack smashing detected ***: terminated
```

新 loader：

```text
SHA-256 b38e3b5ce68f404ed9552631f7dead1b4a33de9a3aec2bfa0173ea05c1454009
GROUPS_RACE rounds=20 fails=0
--- groups-race-probe: PASS
```

上述结果证明：探针能稳定抓住旧实现，修后实现不再产生越界。单次短轮次负对照曾通过，
因此该探针不应退化为低轮次运行；20 轮是当前能稳定区分旧、新实现的门禁设置。

## 4 · 完整回归

```text
make check: PASS
make docs: PASS
gate-a: PASS=38 FAIL=0
IDENTITY_RACE rounds=300 fails=0
GROUPS_RACE rounds=20 fails=0
ROOTFD_RACE stat_ok=165 stat_fail=0 range_ok=500 range_fail=0
path race: PASS host sentinel unchanged
coverage-probe: PASS
coverage-sweep: SKIP (rootfs 没有 strace，不能算通过)
```

APK 完整构建成功。产物 SHA-256：

```text
libld.so        b38e3b5ce68f404ed9552631f7dead1b4a33de9a3aec2bfa0173ea05c1454009
libhuskapp.so   ce74f50ea07b9375de3cb0feb3abe7e7fd68cb173366c61b2fedb24031245e24
libhuskguard.so 4b33119e1b5bb33860a83c1a7854b2c2cf51b4ee5cddc27b81391f8fd9262897
app-debug.apk   7db3993a61d62245450a02e72f1a792ee2b8218d9fdee56dfbc67c3d8713d07a
```

## 5 · 边界

- FACT：修复落在 Husk 的 `path-shim` 和候选 loader，不是桌面补丁，不修改 LinuxHub 2.0。
- FACT：动态负对照使用旧候选 loader 与新候选 loader 的同一探针、同一设备、同一 rootfs。
- INFERENCE：若程序绕过 libc/GOT 直接发 raw syscall，仍不在本次组表模拟范围内，和既有的
  raw-syscall 边界一致。
- UNKNOWN：其他 Android 版本、不同 arm64 处理器和 16 KiB/64 KiB 真机上的调度差异。
- 注意：信号处理器重入时只能得到主组快照；此时组表可能正被另一个线程替换，返回一个
  一致的子集比读取半更新表或自旋自锁更安全。
