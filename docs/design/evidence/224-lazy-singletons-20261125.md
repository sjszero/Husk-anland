# 224 · GPU / 相机侧的懒初始化：并发第一次 open 会拿到别人写了一半的上下文

日期：2026-11-25 23:26–23:45（CST）。设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB。
审计面：`/dev/dri/*` 假节点、`/dev/video*` 假节点、`/dev/mali0` 元数据，以及
sigsys 去重表 —— 都是「懒初始化 + 静态变量」这一族。

## 0 · 结论

| 问题 | 性质 | 现状 |
|---|---|---|
| `SEEN_NR` / `SEEN_COUNT` 在 SIGSYS 处理器里 `if count < len { …; count += 1 }` | 计数可以越过数组长度 → 之后每次遍历都读越界元素，去重从此失效 | 已修：CAS 领槽 |
| `DRM_CTX` / `DRM_MALI_FD`、`V4L2_CTX`、`DRM_DIR_HOST`、`MALI_DEV/RDEV` 都是 `if READY == 0 { 初始化; READY = 1 }` | 两个线程同时第一次 open 会各自初始化一遍（fd/socket 泄漏、结构体一半新一半旧）；aarch64 上普通 store 之间没有顺序保证，另一个核可能先看到 READY 再读数据 | 已修：统一用 `once_begin`/`once_end`（CAS + owner tid + Release/Acquire） |
| `FAKE_FDS` 的「找空槽 → 写进去」不是原子的 | 两个线程看中同一个空槽，后写的覆盖先写的，被覆盖的 fd 从此不在表里（它的 ioctl 落到 memfd 上） | 已修：整张表加锁，`inherit` 三步合成一次持锁 |

## 1 · 现场能看到的那个数字：DRM_IOCTL_VERSION 偶发 EACCES

探针：8 线程 × 20 轮 `open("/dev/dri/renderD128")` → `ioctl(DRM_IOCTL_VERSION)` → `close`，
每个进程跑完打印一次，脚本连跑 15 个进程。

修前（FACT，15 个进程里 2–3 个出现，合计 3 次）：

```
  fail rc=-1 errno=13        ← EACCES
threads=8 rounds=20 ok=159 enotty=0 other=1 drm_major=1 errnos=e13=1
```

每次都只有 160 次里的 1 次，而单线程跑同样的循环 160 次从来不出现。EACCES 不是
「不是我们的 fd」（那条会走 memfd，答 ENOTTY），而是拿到 kbase 上下文之后被拒——
对得上「上下文结构体被别人同时写」。

修后（FACT）：30 个进程 × 160 次 = 4800 次 ioctl，`enotty=0 other=0`，`drm_major=1`。

`drm_major=1` 这个数也不是白拿的：它证明 ioctl 真的走到了 kbase（假节点 + 真
`/dev/mali0`），而不是被 memfd 吞掉。

## 2 · 五个懒初始化点，同一个形状

```
DRM_DIR_HOST / DRM_DIR_LEN / DRM_DIR_READY     /dev/dri 的假目录
MALI_DEV / MALI_RDEV / MALI_META_READY         /dev/mali0 的 dev/rdev 缓存
DRM_MALI_FD + DRM_CTX                          kbase 上下文
V4L2_CTX / V4L2_READY                          camera.sock 客户端
```

统一改成：

```rust
if state.load(Acquire) != 2 {
    if once_begin(&state, &owner) {
        …初始化…
        once_end(&state, ok);      // 数据写完之后才发布 2（Release）
    } else if state.load(Acquire) != 2 {
        …如实报失败，不编一个 0 出去…
    }
}
```

`once_begin` 在别人正在初始化时自旋等，只有当 owner tid 是自己（递归）时才返回
false —— 防止「初始化里又走到同一条路径」把自己等死。

失败路径也要 `once_end(false)` 把状态放回 0，否则一次失败会把这条路径永久钉死为
「不可用」。这是上面那个 `rl == 0` 分支必须显式复位的原因。

## 3 · `FAKE_FDS`：8 个槽，抢同一个空位的两个线程

```rust
unsafe fn register_fake_fd(fd: CInt) -> bool {
    while i < FAKE_FDS.len() { if FAKE_FDS[i] < 0 { FAKE_FDS[i] = fd; return true } }
}
```

两个线程同时第一次 open 渲染节点时都会看到同一个 `-1`，都写进去；先写的那个 fd
从表里消失。之后对它的 `ioctl`/`mmap` 不再被接管，落到 memfd 上 —— 客户端一问
`DRM_IOCTL_VERSION` 拿到 ENOTTY，判定「这不是 GPU」，退回软件渲染。
这就是 `docs/LIMITATIONS.md` §7 那条「桌面看起来在跑，其实一路软渲」的一种成因。

修法：`FAKE_FDS_LOCK`（CAS 自旋）覆盖扫描、登记、注销、区间注销；`drm_inherit_fake`
把「判断 old 是假 fd / 注销 new / 登记 new」三步合并成一次持锁。

**这条是 INFERENCE，不是实测**：8 槽 + close 立刻归还，探针里没能把「丢槽」稳定复现
（4800 次 ioctl 里 ENOTTY 是 0）。改它的依据是代码形状 + 后果的严重性（一次丢槽就让
那个客户端整套退回软件），不是一次可复现的崩溃。

## 4 · sigsys 去重表：信号处理器里的 TOCTOU

```rust
while i < SEEN_COUNT { if SEEN_NR[i] == key { return false } i += 1 }
if SEEN_COUNT < SEEN_NR.len() { SEEN_NR[SEEN_COUNT] = key; SEEN_COUNT += 1 }
```

这段跑在 SIGSYS 处理器里，异步于任何线程。两个线程同时命中时都读到同一个下标、
都写同一个槽、各自 `+1`，`SEEN_COUNT` 可以变成 25 —— 下一次遍历读 `SEEN_NR[24]`
已经越界（UB），而且从此每个 (nr, mode) 都被当成「第一次」，日志又开始刷。
改成 `fetch_update` 领槽：领到的下标一定 `< 24`。

## 5 · 回归

- 门禁 A：`PASS=38 FAIL=0`。
- dl 并发探针（evidence 223 那个 8 线程 × 60 轮 dlopen）：10 / 10 全过，没被这批改动带坏。
- DRM 并发探针：30 / 30 全过。
- 跑完的 `v4l2` 那侧仍然打印 `camera.sock connect failed` —— 应用没在前台，没有 socket，
  这是如实报错；本次只把「同一时刻两个线程各建一个 client」收敛掉，没有改变「没有 socket
  就失败」这个既有语义。

## 6 · 还没做（UNKNOWN）

- `DRM_CTX` 内部的 ioctl 表会不会在 `init` 之后被别的线程读到半成品，取决于 `drm_shim`
  的实现细节，本轮只保证「只有一个线程进 init」，没有审 `DrmCtx` 自身的字段布局。
- `FAKE_FDS` 只有 8 槽，一个程序同时开超过 8 个渲染节点副本（含 dup）仍然会走到
  「槽位耗尽」，只是现在这条路径会打一行日志，不再静默退化。
