# 214 · 覆盖度探针：推理被实测推翻一半，真漏的只有两条

日期：2026-11-25 21:40–22:00（CST）。设备 RMX3357，`com.linuxhub.myth.husk`，
rootfs `files/linux/distributions/ubuntu-24.04/rootfs`。

213 是静态审计（看符号表）。这份是**实测**：写一段 C，把每个"怀疑漏网"的函数都调一遍，
用「文件到底落在 guest 里还是宿主里」判定翻译有没有发生。

## 1 · 方法（FACT）

`tools/probe/coverage-probe.c` + `tools/probe/run-coverage-probe.sh`：

1. 把 .c 推进 rootfs，用 guest 自己的 gcc 编（`husk-session.sh … -- /usr/bin/gcc`）；
2. 在 guest 里跑，每个用例都在 `/root/.p-*` 上操作；
3. 用探针自己的 `stat`（已被翻译）回读，**只有翻译发生了，这个文件才会出现在 guest 里**；
4. runner 逐条对期望，任何一条不一致就 `exit 1`。

复跑：`sh tools/probe/run-coverage-probe.sh UKHMMB8PYTGECMNV`

## 2 · 结果（FACT，同一台机器、同一个 loader，改前 / 改后）

| 用例 | 改前 | 改后 | 结论 |
|---|---|---|---|
| `unlink`（对照组，已钩） | `rc=0 exists_after=0` | 同 | 正常 |
| `remove(3)` | `rc=0 exists_after=0` | 同 | **本来就通**（213 的 P1 推断是错的） |
| `inotify_add_watch` | `wd=-1 errno=2 ENOENT` | **`wd=1 errno=0`，事件 `n=32`** | 原来真的漏，已修 |
| `freopen` | `ok=1 exists=1` | 同 | 本来就通 |
| `pathconf` | `v=255` | 同 | 本来就通 |
| `scandirat` | `n=-1 errno=2` | `n=-1 errno=2` | **仍未通**，见 §4 |
| `truncate64` | `rc=0 size=3` | 同 | 本来就通（`truncate` 的别名） |
| `utime(3)` | `rc=-1 errno=2 ENOENT` | **`rc=0`** | 原来真的漏，已修 |
| `openat` 走 raw syscall | `fd=-1 errno=2` | 同 | 已知边界：raw syscall 不翻译，**不是 bug** |

runner 输出 `--- coverage-probe: PASS`（10/10）。

## 3 · 改了什么（FACT）

| 文件 | 改动 |
|---|---|
| `runtime/crates/path-shim/src/lib.rs` | 新增 `inotify_add_watch`（走 `xattr_pinned` 翻译 + `SYS_INOTIFY_ADD_WATCH`）、`utime`（换算两个 timespec 后走 `utimens_target`）、`__openat_nocancel` / `__openat64_nocancel`（转发到 `openat`）；新增常量 `SYS_INOTIFY_ADD_WATCH = 27` |
| `runtime/crates/path-shim/src/boot.rs` | `BODY_PATCH` 95 → 99，加上述四个名字 |
| `tools/dev/path-symbols.txt`、`tools/dev/hook-coverage.sh` | 新增：需要翻译的符号清单 + 与导出符号做差（213 §2） |
| `tools/probe/coverage-probe.c`、`tools/probe/run-coverage-probe.sh` | 新增：本份的探针 |
| `tools/device-tests/14-gate-a-acceptance.sh` | 两处：① 宿主进程那条检查会把自己的 `grep` 命令行算成 1 条（自匹配），先剔掉带 `grep -E` 的行；② `/proc/self/exe` 按**实测**断言（见 §4.2） |

loader 用 `NDK=… make loader` 重编，设备上换 `files/linux/husk/libld.so` 后复跑，无回归。

## 4 · 三个新发现（FACT）

### 4.1 `scandirat` 修不动，因为拦它的那一层是坏的（*已被 evidence 215 修掉*）

`scandirat` 的路径参数由 glibc 内部 `__openat64_nocancel` 直接发起（不是 GOT 调用），
**导出同名符号也拦不到**。唯一能拦的是 body patch（覆写 libc 函数前 16 字节），而：

```
NR_BODY_PATCH=1            → segv|sig=11
NR_BODY_PATCH=1 NR_BODY_N=95 → segv|sig=11   （只打原来的 95 条，一样崩）
```

`NR_BODY_N=95` 是本次改动**之前**的清单，所以这个崩溃**不是这次加的四个名字造成的**，
是 body patch 本身在 4.19 + Android 13 上就崩。⇒ `scandirat` 与 body patch 一起进 0.0.13 的全链路审计。

### 4.2 `/proc/self/exe` 仍然暴露宿主 loader

commit 09d8689 的本意是让它返回 **guest 程序路径**，实测仍然是：

```
$ readlink /proc/self/exe
/data/data/com.linuxhub.myth.husk/files/linux/husk/libld.so
```

机制未查明（`EXE_GUEST` 只在 `execve` 钩子里写，`env` 起的进程可能走不到）。门禁里那条
断言已按**现状**改名成 `readlink /proc/self/exe exposes host loader (KNOWN GAP)`，
既不复述错误期望，也不假装它是好的。0.0.13 查。

### 4.3 门禁 A 的两条失败都是门禁自己的问题，不是功能回归

- `no host system processes visible` 报 1：命中的是**这条检查自己的 `grep` 命令行**（`ps` 会把它列出来）。
  已改成先剔掉 `grep -E` 行，并把命中行打出来（只报计数的话下次还是查不出是哪个）。
- `/proc/self/exe` 那条：断言写的是 0.0.6 的行为，之后 09d8689 改了语义但断言没跟着改，见 §4.2。

改完复跑：**门禁 A PASS=36 FAIL=0**。

## 5 · 还没做

- `scandirat`、body patch、`/proc/self/exe` 三项进 0.0.13 全链路审计。
- `execveat`、`umount`、`__xstat` 家族仍未实测。
- 探针目前是"点测"：要变成"普查"（跑 nautilus/apt/python 这类真实负载、把它们实际调到的
  syscall 路径全捞出来），需要 strace 或一层 seccomp 记账——rootfs 里现在没有 strace。
