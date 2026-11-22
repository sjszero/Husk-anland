# LinuxHub Husk（壳）· Native Runtime 研究索引

**产品名：Husk（壳）。** 一层壳是它的全部机制——改过的 guest loader 是 guest ELF 的壳，
app 进程是进程的壳；不加 namespace、不加虚拟机，所以不叫 container、不叫 VM。
命名：一个 guest 系统实例 = 一个 husk（`~/.linuxhub/husks/ubuntu-noble/`）；
技术全名仍写 `LinuxHub Native Runtime (T2+T1)`，出现在方案与证据里。

目标：在无 root、无虚拟化、无 Termux 的 Android 普通应用权限下，把 Linux 用户态执行开销压到接近原生。

文件：

- `01-capability-baseline-20261122.md` — 设备能力与权限边界实测（RMX3357 / Android 13 / 4.19）。
- `02-pure-rust-path-shim.md` — 纯 Rust、零 ptrace 路径运行时原型（已跑通 bash/cat）。
- `03-interception-tiers.md` — 原生/链接期/libc/syscall/ptrace 五层对比与实测数据。
- `04-ple110-matrix.md` — Android 16 / GKI 6.6 实测矩阵与架构定稿。
- `05-best-plan.md` — 决策过程与证据索引（路线已定，不再作为方案定义）。
- `06-sole-plan-t2t1.md` — **唯一方案：T2 + T1（权威定义，实施依据）**，含验收清单与剩余工作。
- `07-proot-comparison.md` — 与 PRoot 的同条件对照（系统调用 / 路径 / exec 三张表 + PRoot 本机不可执行的证据）。
- `evidence/` — 原始设备输出（01–23），编号与方案里的引用一一对应；`22`/`23` 记录构建工具 Rust 化（逐字节对拍）。

规范：FACT / INFERENCE / UNKNOWN 必须显式标注；未实测不得写 PASS。
