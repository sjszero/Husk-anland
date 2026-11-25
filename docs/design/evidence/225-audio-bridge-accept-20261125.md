# 225 · 音频桥：一个不说话的客户端就能把整条监听堵住

日期：2026-11-25 23:45–2026-11-26 00:05（CST）。设备 RMX3357 / Android 13 / 4.19.191。
审计面：音频「中转」这条链的应用侧——`app-rs/src/audio.rs` 的两个桥（播放 / 采集）。

## 0 · 结论

| 问题 | 性质 | 现状 |
|---|---|---|
| `serve()` 在 accept 循环里**同步**跑 `handle_client()` | 一个连上来就不发数据的客户端把整条监听堵住（输出侧 30 秒读超时，采集侧**没有超时，永久**），两个监听口共用同一个循环 | 已改：每个客户端一个线程 + 上限 12 + panic 隔离；真机并发验证见 evidence 231 |
| 采集侧 `serve_capture()` 同样的形状 | 同上，而且更糟：`handle_capture` 连读超时都没有 | 同上；真机并发验证见 evidence 231 |

状态：**代码已改，编译已验证，并已在 Husk 最小包上做真机运行验证**。并发入口测试
`PASS=2 FAIL=0`，播放与采集的 OPEN/DATA/READ 均通过；原始输出见 evidence 231。下面的
§4 保留当时“为什么还没跑”的现场记录，不覆盖历史。

## 1 · 形状

```rust
if listeners.is_empty() { … }
while enabled() {
    for listener in &listeners {
        match listener.accept() {
            Ok((stream, _)) => { any = true; handle_client(&api, stream); }   // ← 同步
            Err(WouldBlock) => {}
            …
        }
    }
    if !any { sleep(25ms) }
}
```

`handle_client` 是一条**长连接**：它 `loop { read_line() … }`，协议里 `DATA frames=N` 之后
要一直等下一块 PCM。输出侧给这条连接设了 30 秒读超时，采集侧 `handle_capture` 一个超时
都没设。所以：

- 输出侧：一个 idle 客户端 → accept 循环停 30 秒。期间另一个监听口（`audio-bridge.sock`
  和 `run/husk/audio-out.sock` 是两个 listener、同一个循环）上的连接**一个都进不来**；
  如果 guest 侧的 relay 正好在这 30 秒里重连，它看到的是「连上了但没人应答」。
- 采集侧：一个 idle 客户端 → 循环永久停住，直到那个客户端自己断开。

这不是「慢」，是「拒绝服务」：任何一个能连上 socket 的进程都能让音频桥不再接受新客户端。

## 2 · 改法

- `Api` 只是一组 AAudio 函数指针（外加进程级 dlopen 句柄），AAudio 本身线程安全，
  所以补 `unsafe impl Send/Sync for Api`，共享时不再加锁。
- `serve()` / `serve_capture()` 里把 `api` 换成 `Arc<Api>`，accept 之后
  `spawn_client(...)`：起线程跑 `handle_client`/`handle_capture`，`catch_unwind` 隔离 panic，
  退出时把计数减回去。
- 上限 `MAX_AUDIO_CLIENTS = 12`，超出直接 `drop(stream)`（客户端看到 EOF）并记一行
  `audio|client|BUSY|refused`。这样坏掉的调用方最多占 12 个线程，而不是把线程数拉爆。

**没有**顺手给采集侧加读超时：一个合法的采集会话在两次 `READ` 之间可以空闲很久，
加超时会把正常用例砍掉。head-of-line blocking 已经由「每客户端一线程」解决，
不需要再动超时语义。

## 3 · 判据（探针已入库）

`tools/probe/audio-bridge-concurrency.py`，在 guest 里跑：

1. 客户端 A 连上 socket，**什么都不发**；
2. 客户端 B 连同一个 socket，发 `PING`，最多等 3 秒；
3. B 收到 `PONG` → PASS；B 超时 → FAIL（就是这条 bug）；socket 不在 → SKIP。

对 `/run/husk/audio-out.sock` 和 `/run/husk/audio-in.sock` 各判一次。
修前预期：输出口 FAIL（30 秒里进不去），采集口 FAIL（永久）。

## 4 · 为什么这轮只能到「编译过」

设备上装的不是仓库里的这个应用：

```
$ dumpsys package com.linuxhub.myth.husk | grep -E "versionName|versionCode"
    versionCode=201 … versionName=Myth-Prometheus
```

那是 2.0 那个包（`com.java.myapplication.MainActivity`）。仓库的 `app-rs` 还没装上去，
而且它找 marker 的路径是 `files/rootfs-full/...`，跟当前设备上的
`files/linux/distributions/ubuntu-24.04/rootfs` 不是一套布局 —— 也就是说这条桥在当前
设备上根本没被拉起过。要真跑 §3 的探针，得先把 `make apk` 的产物装上去，那属于发版/打包
那一步，不在本轮审计范围内。所以这一条只能记到「编译通过 + 探针入库」。

## 5 · 顺带：今天两把锁的真实负载回归

`dl_act_lock`（evidence 223）和 `once_begin/once_end`（evidence 224）都是自旋锁，
光跑探针不够，得放到真实程序里。工作负载（guest 内，连跑 4 次）：

```
gcc -O2 -flto × 6              每一条都会 dlopen LTO 插件
python3 -c "import ssl, json, hashlib, sqlite3, ctypes, decimal, unittest" × 20（并发）
python3 与 gcc 混跑 × 10（并发）   dlopen 与 fork/exec 交错
make -j4                        四个 gcc 进程并发
```

结果：4 次全部 `workload fails=0`，没有卡死、没有崩溃。`gcc -flto` 这条尤其相关：
它正是「一个短命进程在导入期 dlopen」的形状，也是加锁以后最容易死锁的形状
（如果哪条路径在 dlopen 里再调 dlopen 而不放行递归，就会卡在这里）。
