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

## 3. 语义验收（`tools/device-tests/06-posix-mqueue.py`，12 项）

见 `docs/design/evidence/114-posix-mqueue-20261123.md`：建队列、发 3 条（优先级 3/7/1）、
按优先级 7→3→1 取出、空队列 `EAGAIN`、`curmsgs` 计数、`close`、`unlink` 删掉仍有消息的队列、
再 `unlink` 得 `ENOENT`。**12/12 PASS**。同一次构建下 `05-mount-audit.sh` 仍 11/11 PASS。

## 4. 未做 / 明确不支持

- `mq_notify`：`EOPNOTSUPP`。
- 两个线程同时 `mq_receive` 同一队列：可能同时挑中同一条消息，抢输的一方 `open` 失败后重扫
  （不会丢数据，但会多一次扫描）。未做专门的并发压测 → **UNKNOWN**。
- `mq_maxmsg` 未限：靠目录增长，磁盘满时才失败。
- 跨进程共享同一条队列是设计内的（同一 rootfs 私有目录），跨 guest 实例不支持。

## 5. 下一步

SysV IPC（`shmget/shmat/shmdt/shmctl`、`semget/semop/semctl`、`msgget/msgsnd/msgrcv`），语义按
cortex `sysv_shm.c` 的三层命名（`id-N` / `seg-N` / `key-K`）与 `SHM_RMID_GRACE` 宽限期实现，纯 Rust。
