# 213 · 覆盖度审计：path-shim 的边界就是它的符号表

日期：2026-11-25 21:30–21:50（CST）。纯静态审计，没有跑设备。

问题：Husk 的路径翻译不是 ptrace 逐系统调用，而是**按符号名改写 GOT/PLT**。所以
「覆盖到什么程度」等价于「导出符号表里有什么」。这份把这件事变成可重跑的输出，
并给出漏掉的清单。

## 1 · 机制对比（FACT，写在这里是因为它决定了后面的风险形态）

| 维度 | Husk | PRoot | chroot | LXC/Docker | Droidspaces |
|---|---|---|---|---|---|
| 拦截层 | 动态链接器 + GOT/PLT 符号 | ptrace 逐系统调用 | 内核 | 内核 namespace | 内核 namespace |
| 静态 ELF | **绕过**（SECURITY.md 已承认） | 覆盖 | 覆盖 | 覆盖 | 覆盖 |
| 程序自己发 raw syscall | **绕过** | 覆盖 | 覆盖 | 覆盖 | 覆盖 |
| 每次文件操作的额外成本 | 0（一次 GOT 改写） | 2 次上下文切换 | 0 | 0 | 0 |
| guest 里跑 gdb/strace | **可以**（ptrace 没被占用） | 不行（ptrace 被 PRoot 占用） | 可以 | 可以 | 可以 |
| 需要 root | 不需要 | 不需要 | 需要 | 需要 | 需要 |
| 真 mount namespace | 无（模拟） | 无（模拟） | 有 | 有 | 有 |
| 漏网时的表现 | **静默**（路径原样交给内核） | 一般不错，只是慢 | 正确 | 正确 | 正确 |

结论（JUDGMENT）：我们的优势是零开销和 ptrace 可用；我们的风险形态是**静默漏网**，
而 PRoot 的风险形态是**慢**。这两种风险要用不同手段兜：PRoot 靠优化，我们靠**可枚举**。

## 2 · 机械化差集（FACT）

新增 `tools/dev/path-symbols.txt`（按 glibc 24.04 导出表整理的 111 个需要路径翻译的符号）
与 `tools/dev/hook-coverage.sh`（从 path-shim 源码抽导出符号做 comm）。

```
$ sh tools/dev/hook-coverage.sh
need=87+24 hooks=184
```

path-shim 实际导出 **184** 个符号，需要翻译的 111 个里**覆盖 87，未覆盖 24**。

## 3 · 未覆盖的 24 个，逐个定性

### 3.1 其实不用做（8 个，属清单噪声）

| 符号 | 为什么不用 |
|---|---|
| `faccessat2` | glibc 走 `faccessat` 回退；且 439 已在 `sigsys_answer()` 里答 ENOSYS 强制回退 |
| `sem_open` / `shm_open` | glibc 自己拼 `/dev/shm/<name>` 再调 `open`，已被 `open` 钩子翻译 |
| `swapoff` / `swapon` | 需要 CAP_SYS_ADMIN，Android 上无论如何不可用 |
| `chroot` | 见 3.2，语义上做不了「真 chroot」 |
| `fanotify_mark` | 需要 CAP_SYS_ADMIN，`fanotify_init` 就先失败 |

### 3.2 逐个实测（2026-11-25 21:5x，见 evidence 214）

上面那张「按推理定性」的表已经被实测推翻了一半——**`remove`、`freopen`、`pathconf`、
`truncate64` 实测都是通的**（另有 glibc 内部调用点走的是被 GOT 改写的符号，不是直接发系统调用）。
真正漏的只有 `inotify_add_watch`、`utime`，以及 `scandirat`（走 glibc 内部的
`__openat64_nocancel` 直接调用，body patch 默认关闭）。
下表左列保留推理，右列是实测结果，**以右列为准**。

### 3.2 真漏网，按影响排序（推理；实测结论见右列「实测」）

| 符号 | 谁会踩 | 现在的实际行为 | 优先级 |
|---|---|---|---|
| `inotify_add_watch` | GIO 文件监视（Nautilus 自动刷新、gedit/gnome-text-editor 外部改动、构建工具的 watcher） | guest 路径直接进内核 → `ENOENT`，监视器静默失效 | **P1** — 实测确认：`wd=-1, errno=2`；已修，修后 `wd=1` 且事件 `n=32` |
| `remove` | 大量 C 程序删文件/空目录 | 原先判为 P1 | **实测是通的**（`rc=0, exists_after=0`）——推断错，glibc 内部调用点落在被改写的符号上 |
| `freopen` / `freopen64` | 把 stdin/stdout 重定向到文件的程序 | 原先判为 P2 | **实测是通的**（`ok=1, exists=1`） |
| `execveat` | 少数用 `execveat(AT_FDCWD,...)` 的程序 | 绕过 loader（raw syscall 面），未实测 | P2（0.0.13 审） |
| `__xstat` / `__lxstat` / `__fxstatat`（含 64） | 用 glibc < 2.33 编译的第三方二进制 | 走版本化符号，我们只导出了 `stat`/`lstat`/`fstatat` → 老二进制 stat 到宿主 | P2（要按 `_STAT_VER` 填结构） |
| `pathconf` | 少数探测文件系统的程序 | 原先判为 P3 | **实测是通的**（`255`） |
| `scandirat` | GNU 扩展，少见 | **实测不通**（`n=-1, errno=2`）：glibc 内部走 `__openat64_nocancel` 的直接调用，导出同名符号也拦不到，只有 body patch 能拦，而 body patch 本身在 4.19 上 `sig=11`（见 214 §4） | P3，0.0.13 处理 |
| `dlmopen` | 命名空间式加载库，罕见 | 绝对路径 .so 加载会失败 | P3 |
| `truncate64` / `umount` / `utime` | 老 API 别名 | `truncate64` 实测通；`utime` 实测**不通**（`rc=-1, ENOENT`）、已修（修后 `rc=0`）；`umount` 未测 | P3 |

## 4 · 为什么这类漏网值得专门建机制

「按符号枚举」的特点是：**漏一个不会报错，只会静默走错路**。而漏网的形态很统一——
内核会收到一个 guest 形态的绝对路径（不以 rootfs 前缀开头）。

于是有一个我们独有、PRoot 用不了的检测手段（INFERENCE，待做）：

> guest 里 `strace` 可用（ptrace 没被我们占用）。跑一段真实负载，把 trace 里出现的
> 所有 path 参数收集起来，凡是「不是宿主 rootfs 下的绝对路径」的，就是漏网的调用点。

这比再加十个钩子更值：它把「还有多少没想到的」从猜测变成一条命令。
计划落成 `tools/guest/coverage-probe.sh` + 门禁用例（跑 nautilus / gedit / apt / python 四类负载）。

## 5 · 下一步（按性价比）

| # | 做什么 | 估时 |
|---|---|---|
| 1 | 补 7 个真漏网符号：`inotify_add_watch`、`remove`、`freopen(64)`、`execveat`、`truncate64`/`umount`/`utime` 别名 | 0.5 天 |
| 2 | `coverage-probe.sh`：strace 采路径 + 断言无未翻译绝对路径；接进门禁 | 0.5 天 |
| 3 | `__xstat` 家族（老二进制） | 1 天 |
| 4 | `pathconf` / `scandirat` / `dlmopen` | 0.25 天 |
| 5 | `SECURITY.md` / `LIMITATIONS.md` 口径改成「符号面 + 已测量的漏网清单」 | 0.25 天 |

## 6 · 不属于 bug、但要写进介绍的边界（JUDGMENT）

- 静态 ELF、程序自己发的 raw syscall：**永远不覆盖**，这是设计选择换来的零开销。
- 真 mount namespace / 真 UID：Android 不给 userns，做不了；Droidspaces 靠 root 才有。
- 恶意代码隔离：不成立，别宣传成沙箱。
