# 兼容层机制：拦在哪、漏了什么、代价多少

> 这份回答一个问题：**Husk 的路径兼容层现在有多完整，缺口在哪，代价是多少。**
> 数字全部可重跑，命令在每节末尾。缺口的实测明细见
> [evidence 218](evidence/218-unhooked-symbols-20261125.md)，`/proc/self/exe` 那条见
> [evidence 217](evidence/217-proc-self-exe-20261125.md)。

## 0 · 三条路，各拦一段

| 层 | 位置 | 拦得住 | 拦不住 | 每进程成本 |
|---|---|---|---|---|
| L1 符号改写 | `boot.rs` 第 2 步，改每个对象每个符号的 GOT/PLT 槽 | 程序与各 DSO 通过 PLT 的调用——绝大多数 | 静态 ELF、程序自己发的 raw syscall、libc 内部直接 `bl` | 启动时一次（187 个导入 × 对象数），之后为 0 |
| L2 libc 体内跳板 | 构建期在 libc 里挖槽位（[libc-interpose.md](libc-interpose.md)） | libc 内部直接调用：NSS、locale、`/etc/*`、apt/dpkg 状态文件 | — | 设计完成、**未实施**（见 §3） |
| L3 内核直通 | 无 | 其余全部系统调用原样到内核 | — | 0 |

`unshare --user` 在本机直接 `EINVAL`、`CapEff` 全 0（`CapBnd` 只有 `CAP_SETUID`/`CAP_SETGID` 两位），
所以「建 namespace 走内核」这条路从设备权限上就不通——我们只能做用户态语义，
这一点不是选择，是边界。**注意**：guest 里 `mount -t tmpfs` 返回 0 是 shim 模拟的，
不是内核真的挂上了。

## 1 · 覆盖账（可重跑）

```
$ sh tools/dev/hook-coverage.sh
need=98+13 hooks=198
```

需要路径翻译的符号共 **111**（`tools/dev/path-symbols.txt`，按 glibc 2.39 导出表整理），
path-shim 导出 **198** 个，其中覆盖 **98（88%）**。未覆盖 13 个，逐个实测的结论：

| 类别 | 个数 | 符号 | 为什么不算缺口 |
|---|---:|---|---|
| 实测通 | 11 | `remove` `freopen` `freopen64` `pathconf` `truncate64` `shm_open` `sem_open` `chroot` `umount` `swapon` `swapoff` | 前 7 个 glibc 内部转调了已改写的入口；后 4 个是权限层本来就不给（EPERM/EINVAL） |
| 不是符号 | 1 | `faccessat2` | 只有系统调用号，glibc 走 `faccessat` 回退 |
| 未测 | 1 | `fanotify_mark` | 需要先 `fanotify_init`，而这需要 CAP_SYS_ADMIN；**UNKNOWN** |
| 真缺口 | 0 | — | `dlmopen` 与 `execveat` 已在 0.0.13 收口（evidence 219） |

**0.0.11–0.0.13 期间补上的**：`inotify_add_watch`（GIO 文件监视原来静默失效）、`utime`、
`__openat_nocancel`/`__openat64_nocancel`、`scandirat`（自己实现扫描体）、
`__xstat` 家族 6 个（老二进制走的就是这些）、`dlmopen`（钩子会撞自己，原版地址从
link map 里跳过自身地址取）、`execveat`（折成一次 `execve`）。

这张表之外还有一类**不是符号**的翻译面，覆盖度不体现在 111 里：
`/proc` 虚拟文件（`/proc/self/exe`、`/proc/mounts`、`/proc/sysvipc/*`）、`/dev/shm`、
`/run/husk/ipc`、进程视图、`getcwd`。这类只能靠用例覆盖，不能靠符号差集。

## 2 · 代价：和 ptrace 型（PRoot）差在哪

机制差别一句话：**PRoot 在系统调用边界上拦，我们在函数调用边界上拦。**
前者每个路径操作要两次上下文切换 + 一次字符串解析；后者是启动时改一个指针。

同一台设备、同一份 rootfs、只换运行器（5 轮中位数，
原始日志 [bench-*-il.log](evidence/) 与总表 [BENCHMARKS.md](../BENCHMARKS.md)）：

| 项 | Husk | proot | 倍数 | 说明 |
|---|---:|---:|---:|---|
| `stat` × 20 000 | 650 ms | 3 734 ms | **5.7×** | 每次路径调用都要过一次 ptrace |
| `open`+`close` × 20 000 | 1 252 ms | 8 481 ms | **6.8×** | 同上 |
| `make -j4`（200 个目标） | 5 194 ms | 24 182 ms | **4.7×** | 编译期路径调用密集 |
| `find /usr` | 1 484 ms | 5 464 ms | **3.7×** | |
| 顺序写 256 MB | 1 294 ms | 1 321 ms | 1.0× | **带宽不差** |
| 顺序读 256 MB | 133 ms | 159 ms | 1.2× | 同上 |

读法（INFERENCE，与上表一致）：差距 ≈ **每次路径调用的固定成本 × 调用次数**，
不是吞吐。所以「文件大的活」几乎一样快，「小文件多、路径调用密」的活差 4–7 倍。
这也是为什么 `apt-get update` 只有 1.1×（时间花在解压和校验，不花在路径上）。

## 3 · 还没闭合的四件事

1. **L2 体内跳板未实施**。libc 自己读 `/etc/passwd`、`/usr/lib/locale` 这类是直接
   `bl`，没有 GOT 槽可改。方案与硬约束写在 [libc-interpose.md](libc-interpose.md)：
   本机 4.19 上 `mprotect` 回 `r-x` 被拒（execmod），所以只能在**构建期**做，
   运行时改代码页这条路是死的。现状：`NR_BODY_PATCH=1` 在本机上 `sig=11` 崩，
   默认关闭——也就是说**这一层现在是 0 覆盖**，不是"部分覆盖"。
2. ~~两个真缺口：`dlmopen`、`execveat`~~ —— 0.0.13 已收（evidence 219）。现在没有
   已知的「符号层真缺口」，剩下的 13 个都是实测无后果或权限层不可达。
3. **并发**：`lib.rs` 里 `static mut` 63 处、`Atomic` 21 处。已经给 `/proc/self/exe`
   的发布通道加了 seqlock（evidence 217），其余共享状态还没逐个过。
4. **普查负载偏窄**：`coverage-sweep` 现在跑的是 4 条命令行负载（`ls`/`find`/`python3`/`apt`），
   GUI 侧（Nautilus、Shell、浏览器）还没纳入。

## 4 · 故障形态：我们和 PRoot 的坏法不一样

| | Husk | PRoot |
|---|---|---|
| 漏网时 | **静默**：路径原样交给内核，要么 ENOENT，要么命中宿主 `/` | 一般不会错，只是慢 |
| 兜底手段 | 三道门禁：静态差集 + 运行时探针 + 真实负载普查 | 优化 ptrace 次数 |
| guest 里能不能跑 gdb/strace | **能**（ptrace 没被占） | 不能（ptrace 已被占） |
| 静态 ELF / raw syscall | 绕过 | 拦得住 |
| 需要 root | 不需要 | 不需要 |

结论（JUDGMENT）：**主路径是完整的，边角靠「可枚举」兜**。这个设计的风险不是慢，
是漏了没人知道——所以覆盖度审计得是常设门禁，不是一次性文档。
