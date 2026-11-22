> **路线已定（2026-11-22）**：唯一方案是 T2+T1，见 `06-sole-plan-t2t1.md`。
> 本文件从此只作决策过程与证据索引，不再作为方案定义。

# LinuxHub Native Runtime — 当前最佳方案（定稿，2026-11-22）

本文件是实施依据。结论来自 RMX3357（Android 13 / 4.19）与 PLE110（Android 16 / GKI 6.6）实测。

## 1. 不可用路线（已实测排除）

- chroot / mount ns / user ns：两台设备 `CONFIG_USER_NS` 均未开启，shell 无 `CAP_SYS_ADMIN`；Shizuku 也无效。
- FUSE：`/dev/fuse` 在 app 与 shell 域均 EACCES。
- app 域 `openat2`：syscall 437 被 untrusted_app seccomp 直接杀进程，禁止调用。
- app 数据目录 exec：EACCES（只有 `nativeLibraryDir` 可执行）。

## 2. 分层策略（按设备能力启用）

| 层 | 机制 | 适用 | 状态 |
|---|---|---|---|
| T0 | 内核原生执行 | 我们自己写的静态 Rust 组件 | 目标 |
| T1 | 安装期改写 ELF（PT_INTERP/RPATH） | 可选优化 | 待做 |
| T2 | `LD_PRELOAD` + 纯 Rust 解析器 | 所有设备，Ubuntu 动态程序 | **已实现 v3.1，31 项上机全 PASS** |
| T3 | seccomp user notification | 6.6+（app 域可用） | 待接入 |
| T4 | ptrace | 仅静态/Go/直接 syscall 兜底 | 保底 |

## 3. 核心组件（v3.1，已实现并上机）

- `crates/runtime-core`：`no_std` 路径解析器。逐段 `openat(O_PATH|O_NOFOLLOW|O_DIRECTORY)` + `readlinkat`；绝对符号链接回到 root 重解，相对链接以 guest 路径栈拼接后重走；`..` 在 root 截断。**不依赖 openat2。**
- `crates/path-shim`：`no_std` cdylib、零 `DT_NEEDED`、panic=abort。已拦截的入口：
  - 打开/查看：`open/open64/openat/openat64/stat/lstat/fstatat/statx/access`
  - 改命名空间：`mkdir(at)/rmdir/unlink(at)/rename(at,at2)/symlink(at)/link(at)/chmod/fchmodat/chown/lchown/fchownat/truncate/utimensat/utimes`
  - 链接与目录：`readlink/readlinkat/chdir/fchdir/opendir(fdopendir)`
  - 进程视角：`getcwd`（内核原始路径改写成 guest 视图）、`execve`
  - 无 `*at` 变体的路径调用：xattr 全族、`statfs`（经 `/proc/self/fd/<parent>/<name>` 回填）
  - `/dev /proc /sys` 直通；`dirfd != AT_FDCWD` 的相对路径原样下传（fd 已在 rootfs 内）
- 关键实现细节（都是踩过的坑，别改回去）：
  1. `chdir/fchdir` 之后 guest cwd 必须每次重新由 `/proc/self/cwd` 推导——程序会绕过 shim 直接 `fchdir`，缓存会漂。
  2. `..` 弹栈只能弹到分隔符，多减一层就会把 `/tmp/d2/..` 算成 `/tm`。
  3. 本设备上 `openat(绝对 /data 路径, O_RDONLY|O_DIRECTORY)` 返回 `EINVAL`；`O_PATH|O_DIRECTORY` 正常。`opendir()` 因此不带 `O_DIRECTORY`，靠 `fdopendir()` 校验。
  4. `readlink` / `getcwd` / `opendir` / `lgetxattr` 都有自己独立的 libc 入口，只拦 `*at` 版本会漏。
- exec：`ld.so --library-path <root>/usr/lib/aarch64-linux-gnu <bin>`；先解析到 parent fd 再读 ELF 头判断是否需要 ld.so，静态 ELF 直接 exec。
- 交付形态：shim/loader 必须位于 APK `nativeLibraryDir`；rootfs 只读不 exec。

## 4. 已验证结果（PLE110，Android 16 / 6.6.118，app 域）

`evidence/09-shimv3-functional-20261122.txt`：**31 项全部 PASS**，同一份 rootfs 内为真实 Ubuntu 24.04 glibc 2.39 + 真实 ld.so：

- 创建/删除/改名/硬链接/符号链接（相对、绝对、`..`、跨目录）、`mkdir -p` 深层建目录都落在 rootfs 内正确位置
- 读写/截断/大文件（22KB 写读一致）、`chmod`/`stat`/`find`/`ls -l`/`sort`/`sha256sum`/glob/管道/子 shell
- `cd`、`pwd -P`、`cd ..`、`/../../etc/os-release` 截断、`/evil -> /system/build.prop` 不可达、`mkdir /../esc` 不越界
- `getcwd` 返回 guest 路径（宿主路径不泄漏），`ls -l` 不再打印假的 ENOENT

`evidence/10-shimv3-performance-20261122.txt`：进程内路径开销 +5~20us/次；进程创建的开销主要来自「只要有 LD_PRELOAD 就要多付约 10ms/exec」（用无关的 libgmp.so.10 做对照组得到同样数字），不是解析器逻辑。PRoot 是每个进程每个 syscall 都付 50~100us（`evidence/05`）。

**域说明（很重要，别把结论外推）**：以上全部在 `runas_app` 域（`run-as` 起的进程，uid 是 app 的，SELinux 域不是 `untrusted_app`）。在 `runas_app` 下 app 私有目录可执行，所以能直接 exec rootfs 里的 ld.so。真正的 app 进程是 `untrusted_app` 域，那里 app 私有目录 **exec 是 EACCES**（`evidence/06`），只有 `nativeLibraryDir` 能执行。因此产品化时 shim、ld.so、以及需要 exec 的 guest 二进制必须落在 `nativeLibraryDir`（或做 T1 的 ELF 改写）。这是下一步的第一优先项，不是可选项。

`evidence/11-proot-on-ple110-20261122.txt`：同机同 rootfs 上，PRoot v5.1 直接跑不起来——guest ELF 的 `PT_INTERP` 是 `/lib/ld-linux-aarch64.so.1`，内核按宿主根解析，PRoot 只能翻译 execve 的路径，翻不了解释器（ENOENT；补了 `/lib` 软链后变 EACCES）。LinuxHub 2.0 是靠自研 loader（`jniLibs/libproot-loader.so`）绕过的；我们的 shim 是自己在 execve 里认出动态解释器、改用 rootfs 的 ld.so 重入，所以不需要自研 loader、不需要 ptrace、不需要常驻进程。

## 4.5 untrusted_app 域打通（2026-11-22，`evidence/12`）

真 app 进程（`untrusted_app`）一启动 bash 就 `exit=159`（128+31 = SIGSYS）。原因不是路径，是 **seccomp**：

1. **FACT（capability-probe，36 个系统调用逐个 fork 试）**：本机 app 域把这些调用直接 **TRAP 成 SIGSYS**，不是返回 EPERM/ENOSYS：
   `rseq(293)` `set_robust_list(99)` `get_robust_list(100)` `futex_waitv(449)` `faccessat2(439)` `openat2(437)` `io_uring_setup(425)` `mount_setattr(442)`。
   其余常见调用（`mmap/mprotect/brk/futex/clone3/set_tid_address/rt_sigaction/getrandom/statx`）都能用，`clone3` 返回 EINVAL（glibc 会回退 `clone`）。
2. **FACT**：`set_robust_list` 是在 guest `ld.so` 内部的 `__pthread_initialize_minimal_internal` 里调的（`ld.so` 偏移 `0x106b0`，紧接着才是 `rseq`，`0x106fc`），**早于任何 `LD_PRELOAD` 构造函数**。所以路径 shim 永远来不及装 SIGSYS handler——这就是「shim 在 runas_app 全绿、到真 app 域直接死」的根因。
3. **解法（已实测）**：给 guest `ld.so` 打一个**入口桩**：
   - 把 `_start` 开头那条 `nop`（`0x1aa40`）改成 `b 0x26730`；
   - `0x26730` 起 304 字节的 aarch64 桩（`tools/nr-loader-hook/entry-stub.S`）：先 `rt_sigaction(SIGSYS)` 装 handler，再跳回 `0x1aa44`；
   - 桩是纯 PIC（0 重定位），塞进原有 RX `PT_LOAD` 的页尾空隙，只把 `p_filesz/p_memsz` 从 `0x2672c` 涨到 `0x26860`；
   - **`e_entry` 绝对不能改**：glibc 用 `*user_entry == &_start` 判断「我是不是被直接执行」，改了就走到解释器分支，`rtld_setup_main_map` 断言失败（`GL(dl_rtld_map).l_libname`）。
4. **另外两处确定性改动**（可选加速器，调用方丢弃返回值，NOP 掉语义安全）：
   - `ld.so 0x106cc`（`set_robust_list` 的 `svc`）→ `nop`
   - `libc.so.6 0x85714`（`__pthread_initialize_minimal_internal` 里的 `svc`）与 `0xbd510`（导出的 `__set_robust_list`）→ `nop`
   - `rseq` 用官方开关 `GLIBC_TUNABLES=glibc.pthread.rseq=0` 跳过，不打补丁。
5. **结果（FACT）**：`evidence/12` —— 真 `untrusted_app` 进程里，Ubuntu 24.04 glibc 2.39 的 bash + coreutils 跑同一份 31 项矩阵：**31 PASS / 0 FAIL / exit=0**。rootfs 全程只 `mmap` 不 `exec`，rootfs 内文件不需要可执行权限。
6. **ucontext 布局是量出来的，不是猜的**：本机 SIGSYS 帧里 `uc_mcontext` 在 `+176`（头文件算出来是 168，因为 `sigcontext` 要 16 字节对齐），`pc` 在 `+440`，`regs[0]` = `pc-256`。稳定做法：拿 `si_call_addr` 去 ucontext 里扫同值槽位（且后一个 qword 高 32 位为 0，像 pstate），扫到即 pc。
7. **未解决（UNKNOWN，别当能力）**：把 trap **模拟成 `-ENOSYS`**（改 `regs[0]` 并 `pc += 4`）会跑出错误行为——子进程 `fork` 开始报 `No such file or directory / Permission denied / Success` 这种无意义 errno。所以当前**唯一可信机制是静态 NOP 掉已知调用点**；trap 模拟只当探针（`NR_SIGSYS=log` 只记录然后按 159 退出）。要恢复「通用兜底」能力，得先把这条查清楚。

## 5. 已知边界与剩余工作（按顺序）

0. ~~先在 `untrusted_app` 域跑通~~ **已完成（§4.5，`evidence/12`，31/31 PASS）**。剩余收尾：
   - trap 模拟（`-ENOSYS`）行为不对，要么修好当通用兜底，要么明确只保留静态 NOP；
   - 静态改动要变成**安装期自动定位**（在 `ld.so`/`libc.so` 里按 `mov x8,#N; svc` 模式扫），而不是写死偏移；换 glibc 版本要能自检；
   - guest 起始 cwd 应该是 `/`；现在继承宿主 cwd（`/data/data/<pkg>/files`），`find` 会打印「Failed to restore initial working directory」。
1. **T2 的天花板**（FACT）：libc 内部调用会绕过 PLT。实测 `ls` 通过 NSS 读 `/etc/passwd` 时直接命中宿主文件；`realpath()` 内部 `readlink` 同理。要么补拦（`__xstat`/`__readlink_chk` 之类），要么交给 T3。
2. T3：seccomp user notification 兜底 exec/open 边缘（6.6+ app 域可用），能堵住上一条。
3. T1：安装期改写 ELF（`PT_INTERP` + `RPATH`），把 exec 路径的解析成本降到 0。
4. 性能优化：resolver 结果缓存（fd→路径）、`/usr/bin/*` 快路径；目前每段一次 `readlinkat`。
5. 覆盖率继续扩：gcc/make、python3、git、Node；包管理器真实安装流程。
6. 与 LinuxHub 2.0 集成，保留 PRoot 回退开关。

## 6. 纪律

- 未实测不写 PASS；区分 FACT / INFERENCE / UNKNOWN。
- 每项改动必须有设备输出证据，存 `evidence/`。
