# PRoot 与 T2+T1：同条件对照（验收第 5 项）

> 设备：PLE110 / Android 16 / kernel 6.6.118 / PAGE_SIZE 4096，SELinux 域 `untrusted_app`（uid 10512）。
> 同一份 rootfs：Ubuntu 24.04 arm64，glibc 2.39。
> 所有数字都是设备上跑出来的；每条后面标明是 **FACT**（有输出）还是 **INFERENCE**（由数据推出）。
> 原始输出：`evidence/13`、`evidence/18`（T2+T1）、`evidence/19`、`evidence/11`、`evidence/05`（PRoot）。

## 0. 结论

1. **在本机，PRoot 不是"更慢"，而是"跑不起来"**：这份随 LinuxHub 2.0 发布的 proot（v5.1.107.87）
   在 `untrusted_app` 里连 `execve("/usr/bin/true")` 都返回 `Permission denied`，连宿主 bionic 程序也
   起不来。四个 tmp 目录变体、`PROOT_NO_SECCOMP=1`、以及把 LinuxHub 自带的 `loader` 绑成
   `/lib/ld-linux-aarch64.so.1` 都试过（`evidence/19`、`evidence/11`）。
2. **在 PRoot 能跑的另一台机（RMX3357 / Android 13）上，同样的 harness 给出的是"每条 syscall 一次
   ptrace 往返"**：5000 次 `stat()` 带 PRoot 0.41–0.76s、不带 0.06s（`evidence/05`）。
3. **T2+T1 在本机是同条件的另一端**：内核直接执行，用户态只替换 C 入口点，**系统调用一条都没碰**——
   `getpid()` 实测 196ns/次（裸 syscall 的量级），PRoot 的量级是 5–10 万 ns（见下）。

## 1. 系统调用开销

| 项目 | T2+T1（PLE110，实测） | PRoot |
|---|---|---|
| 机制 | 不拦截。程序自己的 `svc` 直接进内核 | `ptrace` 逐条拦截并转译 |
| 单条 syscall 底噪 | `getpid()` ×200000 = **196 ns/次**（FACT，`evidence/18`） | 同 harness 下 **70–140 us/次**（RMX3357，5000 次 `stat` 0.41–0.76s vs 0.06s，FACT `evidence/05`；其中 ptrace 往返 ~50–100us，INFERENCE） |
| 安装成本 | 启动一次 `rt_sigaction` + 2 处静态 `nop` | 每个进程 `PTRACE_TRACEME` + 常驻 tracer 循环 |
| 保真风险 | 无：内核看到的就是原来那条 syscall | 每个 syscall 都要能转译；`dma-buf`/DRM ioctl、`clone` 变体是长期脆弱点 |

> **差距来源不是常数项**：PRoot 的成本与"程序发起多少 syscall"成正比（一个 `ls` 就是几万条），
> T2+T1 的成本与"程序调用多少次我们替换的 C 入口点"成正比，且 syscall 数为 0 增加。

## 2. 路径解析开销（同进程内测量，`bench.c`，guest gcc -O2）

| 用例 | 实测 | 说明 |
|---|---|---|
| `stat` 真实路径（已在 root 下 → 直通） | **1.8–2.9 us/次** | 只有我们的入口包装，没有解析（FACT） |
| `stat` guest 路径，1 个组件（`/etc/os-release`） | **4.3–5.4 us/次** | 解析器：逐组件 `openat(O_PATH)` + 符号链接感知（FACT） |
| `stat` guest 路径，4 个组件（`/usr/lib/aarch64-linux-gnu/libc.so.6`） | **11–12 us/次** | 约 2us/组件（INFERENCE，由上一行推出） |
| `open` guest 路径 | **6.2–7.5 us/次** | 同一次解析 + `openat`（FACT） |
| PRoot 路径开销 | **本机不可测**（无法 exec）；机制上等于"每条 syscall × trap 成本" | `evidence/19`、`evidence/05` |

> 同一小区间在两次运行间有 ±40% 抖动（热/后台负载），所以写区间而不是单点。绝对值比 `evidence/10`
> 的 LD_PRELOAD 版本（+5~20us）低，是因为现在 shim 与 loader 同一个对象、没有第二个 preload 的
> 局部性损失；但**路径解析仍是目前唯一值得优化的地方**（按 `(父 fd, 名字)` 缓存可望降到 1–2us）。

## 3. 进程创建（exec）

| 项目 | 实测 | 条件 |
|---|---|---|
| guest `fork+execve+wait`（整条：我们的 execve + ld.so + libc + 程序） | **2.49–2.53 ms/子进程** | PLE110，`bench.c`，100/100 成功（FACT，`evidence/18`） |
| 同上，进程内循环 200 次 | 1.25–1.35 ms/exec | PLE110，bash 内循环（FACT，`evidence/13`） |
| 原生 bionic exec（同目录 5.7KB ELF） | 6.8–8.0 ms | PLE110（FACT，`evidence/13`）——即 guest 路径比同域原生 exec 快约 3–5 倍 |
| PRoot 100 次 `cat` | 7.6 ms/exec | RMX3357，PRoot 能跑的唯一一台（FACT，`evidence/05`） |
| PRoot 本机 | **不可执行** | PLE110，`evidence/19` |

## 4. 为什么 PRoot 在本机起不来（机制，不是配置）

guest 程序的 `PT_INTERP` 是绝对路径 `/lib/ld-linux-aarch64.so.1`：

```
$ llvm-readelf -l usr/bin/cat | grep INTERP
     [Requesting program interpreter: /lib/ld-linux-aarch64.so.1]
```

PRoot 翻译的是 `execve()` 的 pathname，但**内核解析 `PT_INTERP` 时用的是宿主根**。Android 上没有
`/lib/ld-linux-aarch64.so.1`，所以 exec 先 `ENOENT`；把 LinuxHub 自带的 `loader` 绑上去之后变成
`EACCES`（解释器找到了，子进程 exec 仍被拒）。这就是 LinuxHub 2.0 必须自带 loader 的原因，
也是本方案的结构性优势：**我们不 exec 主程序，而是 exec 已经在我们控制下的 loader，把程序作为参数
交给它**，因此 `PT_INTERP` 这一关根本不存在（`evidence/14`）。

## 5. 这张表没有覆盖的东西（UNKNOWN，别当能力）

- PRoot 在本机的失败是否与 Android 16 的某个具体策略有关、换 PRoot 版本能否绕过 —— 未验证，
  因为那不是我们要走的路线。
- T2+T1 的路径开销在**冷缓存/深目录/大量符号链接**（如 `/usr/lib/python3/dist-packages`）下的分布 ——
  未测，当前数字只覆盖 1–4 个组件的常规路径。
- 多线程程序下的 `fork()` 语义（我们的 `posix_spawn` 用 `fork` 而非 glibc 的 `vfork/clone`）——
  单线程工具全部通过，多线程调用方未测（`evidence/16` UNKNOWN 段）。
