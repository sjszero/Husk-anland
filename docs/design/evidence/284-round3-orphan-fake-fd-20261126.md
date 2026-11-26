# 284 · 第 3 轮：假 fd 丢了身份表时要明确拒绝，不许静默

日期：2026-11-26（CST）。设备：无（本轮是源码改动 + 探针用例，未连设备）。
对应缺陷编号：A-08（假 DRM / V4L2 fd 经 `SCM_RIGHTS` 跨进程后身份表不存在）。

## 1 · 修前行为

FACT，`runtime/crates/path-shim/src/lib.rs`（`8078e58` 时）：

- `FAKE_FDS: [CInt; 8]` 按 **fd 号**记账，`is_fake_fd(fd)` 查的就是这张表。
- fd 号是进程本地的。guest 用 `SCM_RIGHTS` 把假 fd 传给另一个进程时，接收方拿到的是
  一个新的号，不在自己的表里。
- `ioctl()` 在表里查不到就走到最后一个分支 `ret_or_errno(syscall4(SYS_IOCTL, ...))`，
  也就是**直接问 memfd**。memfd 对 DRM ioctl 一律 `ENOTTY`。

后果（FACT，代码路径推出；INFERENCE 的是客户端反应）：
客户端拿到 `ENOTTY` 会判定「这不是 GPU」，退回软件渲染。整个链路没有任何日志，
排查时只能看到「GPU 突然不生效了」。

`docs/LIMITATIONS.md` 与 `docs/STATUS.md` 早就承认「跨进程不能保证继续模拟」，
但**没有规定失败形态**，所以「不伪成功」这条一直没有落成代码。

## 2 · 修后行为

FACT，`ioctl()` 的最后一道检查：

```rust
if drm_ioc_type(request) && fd_is_our_memfd(fd) {
    // 只打一次
    return set_errno(9); // EBADF
}
```

- `drm_ioc_type(request)`：`_IOC_TYPE` 是不是 `'d'`（0x64）。只对 DRM ioctl 触发，
  普通 ioctl 一次额外系统调用都不多花。
- `fd_is_our_memfd(fd)`：`readlinkat(/proc/self/fd/N)` 精确比 `/memfd:husk-drm` 或前缀
  `/memfd:husk-v4l2-`。判定依据是内核，不是我们的表，所以「是 ours 但没状态」可区分于
  「根本不是 ours」。
- 命中就 `EBADF` 并打一行 `nrshim drm: fake fd without state (SCM_RIGHTS across processes); refusing with EBADF`。

这样三种情形不再混在一起：

| 情形 | 修前 | 修后 |
|---|---|---|
| 本进程 open 出来的假 fd | 走 DrmCtx 模拟 | 不变 |
| 本进程 `dup` 出来的（shim 会复制记账） | 走 DrmCtx 模拟 | 不变 |
| 跨进程收到的 / 未经 shim 派生的同 memfd fd | `ENOTTY`，静默退回软件 | **`EBADF` + 一行日志** |
| 普通文件的 ioctl | 直通内核 | 不变（不触发检查） |

## 3 · 回归测试

FACT，`runtime/crates/fd-identity-probe/src/main.rs` 新增用例
`pathshim-drm-fake-fd-identity-lost`：

1. `open("/dev/dri/renderD128")` 拿一个 shim 派生的假 fd（记作 `registered`）；
2. `open("/proc/self/fd/<registered>")` 再开一次 —— 同一个 memfd、新 fd 号、**没有**记账，
   这正是 `SCM_RIGHTS` 接收方拿到的形状，单进程就能复现；
3. 对两个 fd 各发一次 `_IOC_TYPE=='d'` 的 ioctl；
4. 断言 `orphan` 上 `rc == -1 && errno == EBADF(9)`。

编译通过（`cargo check --target aarch64-unknown-linux-gnu -p fd-identity-probe`）。

## 4 · 未验证

- UNVERIFIED：没有在设备上跑这条新用例。`errno == 9` 的断言依赖 shim 先于 memfd 处理，
  需要真机确认 `libld.so` 的 `ioctl` 确实被调用（`/proc/self/fd` 的 open 是否绕开 shim 也待确认）。
- UNVERIFIED：真正的双进程 `SCM_RIGHTS` 传递没有测。本用例用 `/proc/self/fd` 复现了同样的
  「同一个 memfd、不同 fd 号、无记账」形状，但没有覆盖「接收方进程里根本没有 shim 状态」的
  极端情况（例如接收方不是 husk guest 进程）。
- 没做：**完整恢复身份表**。要做需要把 fd →（设备、buffer 表）随 fd 一起序列化，
  `SCM_RIGHTS` 没有承载附加数据的通道，必须自己定义握手协议（例如传 fd 前先发一条
  元数据消息）。那是新能力，不在本轮。

## 5 · 门禁归属

`fd-identity-probe` 属于设备侧探针，进 `make gate-a` 的 coverage 组。本次新增用例失败时
会在探针输出里打 `fdidentity|pathshim-drm-fake-fd-identity-lost|FAIL|...`。
