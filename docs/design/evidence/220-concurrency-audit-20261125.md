# 220 · 共享状态并发审计（第一批）：三处会给出错误答案的竞态

日期：2026-11-25 22:55–23:05（CST）。纯读码 + 本机实测，设备 RMX3357 / Android 13 / 4.19.191。

## 0 · 做法

`path-shim` 是常驻在**每个** guest 进程里的库，任何一处共享状态被两个线程同时碰就会出问题，
而 guest 里的程序大多是多线程的（gnome-shell、python、javac、electron）。这一批的系统做法：

1. 扫出 `lib.rs` 里全部 `static mut` 声明（63 处，含函数内的）与 `Atomic`（25 处）；
2. 对每处找出「运行时写点」（排除初始化一次的那些）；
3. 只看**写点是否被锁/原子保护**，以及**读者会不会因此拿到错误答案**——性能问题排后面。

## 1 · 修掉的三处（都会给出错误答案，不是慢）

### BUG-A · `PEER_CACHE`：24 字节元组撕裂 → 把一个进程的身份安到另一个进程头上

`peer_identity(pid)` 的结果缓存在 `PEER_CACHE: [(pid, uid, gid); 8]`，缓存项是**元组**，
`PEER_CACHE[i] = (pid, uid, gid)` 在 aarch64 上是三条独立的 store。它在**stat 路径**上被调用
（`lib.rs` 3468 行附近，给「这个文件属于谁」定身份），多个线程同时 stat 不同进程的文件时，
读者可能看到「pid 是新的、uid 还是上一条的」。

修法：seqlock。写前序号 +1（变奇数）、写完 +1；读者只在偶数时读、读完核一次。
拿锁用**有界自旋**（超过 10 万次就不写缓存、让下次重新算）——缓存只是加速，
不值得为一个缓存把进程卡在可能的信号重入死锁上。

### BUG-B · `FAKE_ID_READY`：先置位再写值 → 可能把 root 报成 0 之外的错误身份

```rust
if FAKE_ID_READY == 0 {          // 旧代码
    FAKE_ID_READY = 1;           // ← 先宣布"就绪"
    FAKE_UID = parse_dec(...);   // ← 后写值
}
```
两个线程同时进 `fake_id()`（每个进程的第一批 stat 就打在这里），后进的那个看到「已就绪」
却读到还没写的 0。

修法：`FAKE_ID_READY` 换成 `AtomicU8`，**先写值再 release 置位**，读者 acquire。
两线程同时进来最多重复解析一次 env，结果一样。

### BUG-C · `RAND_COUNTER`：非原子自增 → 两个线程拿到同一个「随机」名字

`next_random()` 给 `mkstemp` 一族产生临时文件名，原来是 `static mut u64` 上的 `+= 1`。
两个线程同时读到同一个值就产出一模一样的名字（`O_EXCL` 会挡住碰撞，但等于把可避免的失败
丢给调用方）。修法：`AtomicU64::fetch_add`。

## 2 · 核过、确认不是 bug 的（免得下次重查）

| 项 | 结论 |
|---|---|
| `SHM_ATT`（SysV shm 附着表） | 写点在 `shmat_inner()` 里，而 `shmat()` 外层已经 `ipc_lock()`；`shmdt()` 自己加锁（FACT，读码） |
| `NL_SLOTS`（netlink 槽位） | `nl_install()` / `nl_forget()` 都在 `nl_lock()` 内（FACT） |
| `MNT[]` / `MNT_N` | `mount()` 里 `MNT_N >= MNT_MAX` 先返回 `ENOSPC`；读文件那条循环条件是 `MNT_N < MNT_MAX`；两张表都在 `mnt_lock()` 内（FACT） |
| `ROOT_*`（rootfd、root 真路径、dev/ino） | 只在 boot 阶段单线程写一次（FACT） |
| `OWN_*`（虚构所有权表） | `OWN_LOCK` + 状态机保护，写点注释写明「Callers hold OWN_LOCK」 |
| `DRM_*` / `V4L2_*`（设备 shim） | 惰性初始化 + fd 表，都在首次打开设备时写；上层调用本身是单线程的（INFERENCE） |

## 3 · 还没结论的（UNKNOWN，下一批）

`ALIAS_DEV` / `ALIAS_INO`（硬链接别名表）、`LINK_COPY_NOTICED`、`SEEN_NR`、`TMPFS_SEQ`、
`IDENT_PUB_UID` / `IDENT_PUB_GID`、`LEN` / `LINE`（函数内 scratch）。这些的写点
要么频率极低、要么调用方本身单线程，但**还没有逐个读透**，所以这里不写「安全」。

## 4 · 验证

```
$ DEV=UKHMMB8PYTGECMNV make gate-a
PASS=38 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: PASS
```

功能面上，`id` / `ps -e -o user,pid` / `ls -l` 的身份映射在改后仍正确；并发压测
（6 组 × 10 轮并行 `ps` + `stat`）跑完无异常输出。

**验证等级说清楚**：这三处是**按构造修掉**的（seqlock / 原子自增 / release-acquire），
但「压测没看到异常」不等于证明了竞态消失——要证明需要线程竞争检测器，
本机没有可用的 Rust 竞态检测（no_std + 自建 loader）。这一条留在 0.0.13 的
审计结论里，作为**未验证**项标注。
