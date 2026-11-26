# 250 · V4L2 假 fd 高于 1024 的生命周期，以及相机上下文并发保护

日期：2026-11-26 11:20（CST）。设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB。
探针：`runtime/crates/v4l2-ref-probe`，通过 Husk 会话在 guest 内运行。

## 0 · 修前事实

`v4l2-shim::Ctx::refs()` 只扫描 fd `0..1023`：

```rust
let mut fd = 0;
while fd < 1024 {
    if fd_dev_of_kernel(fd) == Some(i) { n += 1; }
    fd += 1;
}
```

FACT：`dup()` 可以把合法 fd 放到 1024 以上。探针把 `/dev/video0` 的 memfd 复制到 fd 1100，再关闭原 fd，
旧实现没有看到 1100，直接把相机 socket 当成最后一个引用拆掉。随后对 1100 做 `QUERYCAP` 失败：

```text
OPEN fd=4 errno=0
DUP_HIGH fd=4 high=1100 errno=0
HIGH_LINK n=28 target=/memfd:husk-v4l2-0 (deleted)
CLOSE_ORIGINAL rc=0 errno=0
[husk v4l2] unknown fd for ioctl=0x80685600
   fd=1100
   target=/memfd:husk-v4l2-0 (deleted)
IOCTL_HIGH rc=-1 errno=13 driver=
```

FACT：`V4L2_CTX` 的 socket、设备和 buffer 表没有锁。path-shim 的 lazy init 由 once 状态保护，
但 init 之后的 `open` / `ioctl` / `mmap` / `close` / `fcntl(F_SETFL)` 可以对同一份 `static mut`
并发读改写；preload 层也定义了锁，却从未使用。静态源码确认，动态并发损坏后果为 UNKNOWN。

## 1 · 修法

1. `refs()` 改为用 `getdents64` 枚举 `/proc/self/fd`，不再假设 fd 小于 1024。
2. 目录读取失败时 fail closed：保留设备状态，不因一次 `/proc` 读取失败误拆活 socket。
3. path-shim 的 `V4L2_CTX` 全部访问加自旋锁；lazy init 仍由原有 once 状态保证只执行一次。
4. `v4l2-preload` 启用原本闲置的锁，覆盖 rootfs 初始化、`Ctx`、ABI 转发、stdio hook。
5. 新增 Rust 回归探针 `runtime/crates/v4l2-ref-probe`，在进程内起一个只实现 `INFO` 握手的假
   `camera.sock`，断言三件事：高 fd 复制后原 fd 可关、socket 仍存活、最后一个引用关闭后才 EOF。
6. 探针接入门禁 A；不需要真实相机权限，也不伪称采到了摄像头帧。

实现中先犯了另一个 AArch64 常量错误：`O_DIRECTORY` 写成 asm-generic 的 `0o200000`，本机 UAPI
实际是 `0o40000`。第一次候选版因此在真机打印
`[husk v4l2] close: cannot enumerate refs; preserving state`；改成 `0o40000` 后枚举正常。

## 2 · 修后真机输出（FACT）

```text
V4L2_REF|open|fd=6
V4L2_REF|dup|high=1100
V4L2_REF|original-close|socket=alive
V4L2_REF|ioctl-high|rc=0|driver=husk
V4L2_REF|last-close|socket=eof
V4L2_REF|PASS
```

用旧 `0..1023` 实现做负对照，同一探针稳定失败并提前断开 socket；修后版本稳定通过。

## 3 · 回归

```text
PASS=38 FAIL=0
coverage-probe: PASS
coverage-sweep: PASS（1400 条候选路径，无未翻译 guest 绝对路径）
V4L2_REF|PASS
make check: PASS
```

## 4 · 未验证 / 边界

- UNKNOWN：真实 Camera2 后端的连续取帧、STREAMON/DQBUF 与高 fd 复制组合。本探针只验证 V4L2
  假 fd 的生命周期和同一 `camera.sock` 握手，不冒充真实相机画质或帧率验收。
- UNKNOWN：通过 `SCM_RIGHTS` 把 V4L2 memfd 传给另一进程后，接收进程没有本地 `Ctx` 槽位，当前不保证继续模拟。
- UNKNOWN：多线程同时 open/close/mmap/ioctl 的长时间压力。锁已覆盖所有已知访问点，但没有把
  静态保护写成动态长跑通过。
