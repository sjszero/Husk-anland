# 27 — 两个真根因：loader BSS 没被清零、app 里缺 NR_LD

日期：2026-11-22
设备：RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4 KiB）
结论只来自本机实测，未外推。

## 一、`exit 159` 不是 SIGSYS（先纠正旧结论）

FACT：在 untrusted_app 域里跑 guest bash，sigsys.log 里**只有** `nr-init|...`，
没有任何 `sigsys|` 记录 → 没有任何 seccomp TRAP 发生。kill 家族的 interpose 确实生效
（我们自己把它拦下了，内核根本没看到那次调用）。

所以之前「handler 找不到 sigframe → exit 159」的推断是错的：159 是 guest 内部
`_Exit(159)`，调用点 `libc.so.6+0x3f1f8`（`mov w0, w23 ; bl _exit`）。

## 二、根因 1：embed 之后 ld.so 的 BSS 没被清零（装载器直接炸）

FACT（实测现象）：`nr-tools embed` 产出的 loader 一跑程序就死：
- `--list` 在 `LD_DEBUG=all` 下打印到 `generating link map` 就 SIGBUS；
- 连一个 0x1000 字节的空 PT_LOAD 加在后面也一样死；只往文件尾部补零则完全正常。

FACT（机制）：内核只对**最大 PT_LOAD 的末尾**做一次 `padzero(elf_bss)`。我们把 shim
作为新的最大段放在 0x42000 之后，于是 ld.so 自己的 BSS 页不再被清零 —— 而那一页的
文件内容是「工具链顺手写进去的 section header」。
证据：`/tmp/husk/ldi.so`（旧构建，shoff=0x50520，那一页是 0）能跑；同一份 phdr、
同一份映射内容的 `ld-test.so`（shoff=0x316c0，正落在 0x31000 页内）SIGBUS。
用入口级 SIGSEGV/SIGBUS reporter 抓到的现场：`signo=7 (SIGBUS) code=1 si_addr=0x…bbb`。

FIX（`runtime/crates/nr-tools/src/main.rs`，embed 末尾）：
1. 取 **loader 的** RW PT_LOAD（不是 shim 的），把 `[off+filesz, 页尾)` 在文件里清零；
2. 如果 section header 表正好落在这一段，先把它搬到文件末尾再清零。
输出里会打印两行 `bss pad: ...`，可复现。

VERIFIED：修完后 `ld-*.so --library-path … rootfs/usr/bin/true` → exit 0；
`bash -c 'echo HI; /usr/bin/groups'` → 正常执行。

## 三、根因 2：app 里没有 NR_LD → 子进程 exec 全部 EACCES

FACT：shim 的 `execve()` 对动态 ELF 要 re-enter loader：优先用 env `NR_LD`，
没有就退回 `<rootfs>/usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1`
（源码注释里已写明「app 私有目录不可执行」）。
term.rs/trace.rs 之前**没有传 NR_LD**，于是 bash 每次子命令 exec 都 EACCES，
bash 打印 `bash: /usr/bin/groups: Permission denied` 后以 159 退出。

FIX：`app-rs/src/term.rs`、`app-rs/src/trace.rs` 的子进程 env 增加 `NR_LD=<nativeLibraryDir>/libld.so`。

VERIFIED（untrusted_app 域，脚本 `/tmp/t.sh`）：
    START
    10302 9997 20302 50302
    RCDONE
    ALIVE
    exit 7
即 fork+exec+shim re-entry 全链路在 app 域里是通的。

## 四、还差一段：`bash -i` 自己给自己发 SIGSYS

FACT：`bash -i` 时（PTY 或非 PTY 都一样），bash 在 job control 初始化阶段调用
`kill(getpid(), 31)`；栈上能看到的帧：`bash+0x8b1a4`、`bash+0x8e314`、
`libtinfo.so.6.4+0x200e0`、`bash+0x8ede8`、`bash+0x8f0f0`。
我们的 kill interpose 把它吞掉（返回 0），bash 随后 `_Exit(159)`。

UNKNOWN / 待查：bash 在这个位置为什么发 SIGSYS（不排除是它对「前一次信号」的 re-raise，
即 `termsig_handler` 里的 `kill(getpid(), sig)`）。下一步按 bash 0x8b1a4 反汇编定位。

## 五、可复现命令

    # loader
    nr-tools scan-syscalls patch <rootfs>/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1 /tmp/ld-s1.so --version 2.35 --expect set_robust_list=1
    nr-tools embed <shim.so> /tmp/ld-s1.so init-hook.bin libld.so
    # 验机
    run-as com.linuxhub.husk sh -c 'cd files; ./ld.so --library-path $PWD/rootfs/usr/lib/aarch64-linux-gnu rootfs/usr/bin/true; echo $?'

## 六、续：`bash -i` 的 159 已解决（sigaction guard）

FACT：把 `sigaction`/`signal` 也 interpose 之后，第一次看到真正的 trap：
    sigsys|code=1|nr=151|...|mode=2   ← setfsgid
    sigsys|code=1|nr=152|...|mode=2   ← setfsuid
即 untrusted_app 域里被拦的是 **setfsuid/setfsgid**（TRAP 档，可仿真）。

INFERENCE（有实测支撑）：之前看不到这些记录，是因为 bash 自己
`sigaction(SIGSYS, …)` 覆盖掉了我们的处理器；于是一次 trap 交给了 bash 的
`termsig_handler`，它 `kill(getpid(), sig)` 再 `exit(128+sig)` → 159。
反汇编 bash 的 0x8b1a4 正好是 `add w0, w21, #0x80 ; bl exit`，与 159 完全对上。

FIX：shim interpose `sigaction`/`signal`，SIGSYS 的安装一律「返回成功但不生效」，
其它信号按两种结构体布局（libc 152B / kernel 24B+mask）翻译后透传。

VERIFIED（untrusted_app，真机）：`bash -i` 起来并停在提示符：
    [?2004hI have no name!@localhost:/$
进程存活（/proc/<pid> 在），sigsys.log 里有 nr=151/152 的仿真记录。
