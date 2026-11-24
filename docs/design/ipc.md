# IPC 表面：POSIX 消息队列（以及后面的 SysV）

> 目标：guest 里的程序能像在普通 Linux 上一样用 `mq_open/mq_send/mq_receive/mq_unlink`。
> 前提：不 root、不解 BL、不开虚拟化、不扩大 seccomp 拦截面。设备 RMX3357 / 4.19.191 / `untrusted_app`。

## 0. 先看前人（开工门禁）

| 仓库 | 固定 commit | 读到的可复用点 | 不采用的理由 |
|---|---|---|---|
| youki | `f330e830fd1dec9be749cde99a2e589ff6373dca` | `mq_*` 的**语义矩阵**：`O_CREAT`/`O_EXCL`/`O_NONBLOCK` 三种 oflag 的返回、`mq_getattr` 的 `mq_flags`/`mq_curmsgs` 口径、`populate_mqueue()` 用"目录一个队列、文件一条消息"的形状 | 它靠真实 `mqueue` 内核文件系统挂载，我们没有 CAP_SYS_ADMIN；只借语义，不借实现 |
| bubblewrap | `1fb0aa26d3eb66254f7742e704add7f82f41a582` | 无 CAP_SYS_ADMIN 时的 IPC 命名空间边界：`--unshare-ipc` 之后哪些 syscall 必须自己回答 | 它只是把 IPC 命名空间隔离开，不提供队列服务 |
| ssfuisu/cortex | `351d8815e2c2247d2a3d46f79677d7cee8a752ee` | `app/src/main/cpp/hook/sysv_shm.c` 里 SysV 的三件套语义：`id-N` / `seg-N` / `key-K` 的命名分层，`SHM_RMID_GRACE` 的宽限期 | 它的 SysV 实现在 C、且改的是 bionic；这里要的是 glibc guest 内的纯 Rust 版 |

参考代码不进 Husk 仓库，笔记在 `husk-notes/prior-art/`。

## 1. 环境事实（FACT，实测）

- 本机没有 mqueue 文件系统：`/dev/mqueue` 只是 devtmpfs 里的空目录，`mkdir` 直接 `ENOENT`，没有挂载点可以借。
- **arm64 的 glibc 2.39 把 `mq_*` 实现成 legacy 系统调用**，不是文件系统实现。反汇编 `mq_unlink`：

  ```asm
  91910 <mq_unlink>:
    91918: ldrb w1, [x0]
    9191c: cmp  w1, #0x2f          ; name[0] == '/'
    91920: b.ne 0x91968            ; EINVAL
    91928: mov  x8, #0xb5          ; __NR_mq_unlink = 181
    9192c: svc  #0
  ```

  `__NR_mq_unlink`(181) 在 Linux 上从未实现，内核回 `ENOSYS`。所以**任何** glibc guest（含真机 root）调用
  `mq_unlink` 都会得到 `ENOSYS`；这是 glibc 在 arm64 上的老问题，不是我们的 bug。
  `mq_open/mq_send/mq_receive/mq_getattr` 同理（`__NR_mq_open` = 180 等）。
- 结论（JUDGMENT）：要么让这些 syscall 在进程里被**符号级 interpose** 掉，要么这个表面就永远不可用。
  扩大 seccomp 拦截面是硬约束禁止的，所以只能 interpose。

## 2. 设计

| 概念 | 表示 |
|---|---|
| 队列 | 目录 `/dev/mqueue/.q/<name>`（`mq_open` 用 `mkdirat` 建，mode 强制 `|0700`——0666 的目录进不去） |
| 消息 | 目录里的一个文件，名字 `p<prio 3位>_<单调纳秒 19位>_<pid>` |
| `mqdes` | 就是那个目录的 fd（`mq_close` = `close`） |
| 优先级顺序 | 文件名按字典序扫描即得：`p007…` < `p003…`，同优先级按纳秒（先入先出） |
| `mq_getattr` | `mq_flags` 从 `F_GETFL` 读 `O_NONBLOCK`；`mq_maxmsg` 未限（0）；`mq_msgsize` 8192；`mq_curmsgs` 现扫目录计数 |
| `mq_notify` | `EOPNOTSUPP`（如实说不支持，不假装） |

### 三个必须踩过的坑（都是上机才知道的）

1. **目录必须有 x 位**：`mq_open` 传 `0666` 建目录后自己都进不去（`EACCES`）。创建时 `|0700`，
   并在拿到 fd 后用 `fchmod` 修一遍（旧版本留下的 0666 目录也能救回来）。
   *不能*用 raw `fchmodat` 修——raw syscall 绕过路径改写，作用到宿主路径上，静默没效果。
2. **getdents64 有偏移**：目录 fd 扫一遍后偏移停在末尾，第二次 `mq_receive` 会误判"空队列"→`EAGAIN`。
   每次扫描前 `lseek(fd, 0, SEEK_SET)`。
3. **成功时不能污染 errno**：`mkdirat` 的 `EEXIST` 会把 errno 写成 17，调用方读到的就是假的失败。

### 拦截点

`mq_*` 在 libc 里是**函数体**（`svc #0`），没有 PLT 可改，只能改函数入口。用的是本仓库已有的
`nr-tools libc-interpose`：把入口前 6 条指令搬到"stub 页"，入口改成
`adrp/ldr/cbnz` → 运行时填进来的 shim 地址。

`mq_unlink` 一开始被这个工具**拒绝**：它的第 5 条指令是 `b.ne 0x91968`，窗口搬到离 .text 一页之外的
stub 页后，19 位分支（±1 MiB）够不着。修法是让重定位器把够不着的条件分支**展开**成
"反条件跳过 + 全宽 `b`（±128 MiB）"两条指令，stub 页改成变长紧凑排布。这一改把整张表从 74/87 提到
82/87，`__open_2` / `__openat_2` / `__openat64_2` / `renameat2` / `mkstemps` 也一并补上了。

## 3. 语义验收（`ipc-probe mq`，12 项）

见 `docs/design/evidence/114-posix-mqueue-20261123.md`：建队列、发 3 条（优先级 3/7/1）、
按优先级 7→3→1 取出、空队列 `EAGAIN`、`curmsgs` 计数、`close`、`unlink` 删掉仍有消息的队列、
再 `unlink` 得 `ENOENT`。**12/12 PASS**。同一次构建下 `05-mount-audit.sh` 仍 11/11 PASS。

## 4. 未做 / 明确不支持

- `mq_notify`：`EOPNOTSUPP`。
- 两个线程同时 `mq_receive` 同一队列：可能同时挑中同一条消息，抢输的一方 `open` 失败后重扫
  （不会丢数据，但会多一次扫描）。未做专门的并发压测 → **UNKNOWN**。
- `mq_maxmsg` 未限：靠目录增长，磁盘满时才失败。
- 跨进程共享同一条队列是设计内的（同一 rootfs 私有目录），跨 guest 实例不支持。

# SysV IPC

## 5. 环境事实

`shmget`/`semget`/`msgget` 在这台机器上一律 `ENOSYS`（实测），`/proc/sysvipc` 不存在——
内核没编 SysV IPC。旧文档把这条写进"不可解除的平台硬边界"，那是错的：这条 ABI 和 mqueue 一样，
可以在库入口接管。X11 MIT-SHM、PostgreSQL、Qt 共享像素、一堆老守护进程都还在用它。

## 6. 设计：文件 + mmap + flock

```
/run/husk/ipc/shm/<id>      共享段本体（`shmat` 就是 mmap(MAP_SHARED) 它）
/run/husk/ipc/shm/<id>.m    key size mode cuid cgid cpid ctime rmid
/run/husk/ipc/shm/<id>.a    每个挂接进程一行（宿主 pid）
/run/husk/ipc/sem/<id>      nsems 个 int32 信号量值
/run/husk/ipc/sem/<id>.m    key nsems mode rmid
/run/husk/ipc/msg/<id>/     一个队列一个目录，一条消息一个文件
/run/husk/ipc/msg/<id>.m    key mode qbytes rmid lspid lrpid stime rtime ctime
/run/husk/ipc/keys          key 与 id 的对应表
/run/husk/ipc/nextid        id 计数器
/run/husk/ipc/lock          flock() 目标，所有元数据操作都持有它
```

| 表面 | 实现要点 |
|---|---|
| `shmat` | `mmap(MAP_SHARED)` 段文件：页真的共享，一致性交给内核；我们只管簿记 |
| `IPC_RMID` | 打 `rmid` 标记，最后一个挂接者 `shmdt` 时才删——就是 cortex `SHM_RMID_GRACE` 的语义 |
| `semop` | 每次尝试都在**值文件**上 `flock(LOCK_EX)`：读全组 → 判断是否全可前进 → 全应用，保证 all-or-nothing 跨进程成立 |
| `semtimedop` | 绝对超时，按 `CLOCK_REALTIME` 差值和 2 ms 轮询 |
| 消息顺序 | 文件名前缀是 20 位零填充的 `mtype`，字典序扫描 == 内核的"按类型、同类型先进先出" |
| `msgrcv` 的 `msgtyp` | 0 = 最早一条；>0 = 该类型最早一条（`MSG_EXCEPT` 取反）；<0 = 类型 ≤ 绝对值中最小的 |
| `E2BIG` | 先 `lseek` 问真实长度：超长且没给 `MSG_NOERROR` 时返回 `E2BIG` 且**消息留在队列里** |

### 上机才发现的坑

1. **死段留下的 key 行会把新段挡死**：`IPC_RMID` 删了段文件却没删 `keys` 里的行，下次
   `shmget(key, O_CREAT)` 找到那个死 id、读不到 meta、直接 `ENOENT`。改法：删段时一并删 key 行，
   并且读到死 id 时先清行再走创建路径。
2. **`semid_ds` 的字段位置**：glibc 2.39 arm64 是 `ipc_perm` 0–48、`sem_otime` 48、`sem_ctime` 56、
   `sem_nsems` **64**（不是教科书里的 88）。必须先上机把结构体 dump 成 u64 数组核对，再写。
   `IPC_STAT` 只写这已知的 72 字节，绝不越界写——多写 16 字节会把调用方堆写坏，崩在后面的 `free`。
3. `ctypes` 把 va_list 里的指针按 32 位传会截断（测试脚本的坑，不是 shim 的）。

## 7. 语义验收

| 脚本 | 项数 | 结果 |
|---|---|---|
| `ipc-probe shm` | 13 | PASS（含 fork 出的子进程写、父进程读的真实共享；RMID 宽限期；key 生命周期） |
| `ipc-probe sem-msg` | 24 | PASS（P/V、`EAGAIN`、all-or-nothing、跨进程阻塞唤醒、按类型/负类型取消息、`E2BIG`、`MSG_NOERROR`、`IPC_STAT` 计数） |

## 8. 明确不支持

- `SEM_UNDO`：返回 `EOPNOTSUPP`（95），不假装支持——静默忽略会让程序在崩溃路径上留下被占的死锁。
- 信号量的 `sem_otime`/`sem_ctime`：留 0（没有维护时间戳）。
- `/proc/sysvipc/*` 已合成（见 §10），`ipcs` 能列出段；`ls /proc/sysvipc` 这种**目录列目录**还不行（`/proc` 的 getdents 不经过合成表）。
- 队列 `qbytes` 固定 16384，不做阻塞式满队列等待（`msgsnd` 不因满而阻塞）。

## 9. 下一步

`/dev/full`；网络表面（AF_NETLINK）；`/proc/sysvipc` 的目录项。

## 10. `/proc/sysvipc/*`

`ipcs` 不扫目录，它直接 `open("/proc/sysvipc/shm")`。所以在 `virt_synth` 里按 procps 的列宽合成这三张表，
数字直接来自同一个存储（`ipc_each()` 遍历 `<id>.m`）。实测：

```
$ ipcmk -M 8192 ; ipcmk -S 4 ; ipcmk -Q
$ ipcs
------ Message Queues --------
0x58c3de1c 13  root  644  0  0
------ Shared Memory Segments --------
0x1d9c92fe 11  root  644  8192  0
------ Semaphore Arrays --------
0x34e7db6a 12  root  644  4
$ ipcrm -m 5     # 真的把段删掉了
```

顺手踩到的：`ipc_each` 最初写成 `len > 3` 才认 `.m` 后缀，而 `"5.m"` 的名字长度正好是 3，于是所有段都被跳过——
表头有、行没有。改成 `len > 2`。
