# 运行时方案（权威定义）— Husk（壳）

> 2026-11-22 定为**唯一路线**。早期探索记录（[history/](history/)）与设备证据（[evidence/](evidence/)）保留，
> 作为决策依据；本文件是方案的权威定义，与它冲突的其他描述一律作废。

## 0. 名字

产品名 **Husk（壳）**：我们加在 guest 外面的唯一东西就是一层壳——改过的 guest loader 是 ELF 的壳，
app 进程是进程的壳。一个 guest 系统实例叫一个 husk（`~/.linuxhub/husks/<distro>/`）。

## 0.1 一句话

让 guest 发行版（glibc）作为**普通 Android 进程**直接跑在内核上；路径命名空间由我们自己用 Rust 在用户态补上；
所有"必须改的东西"在**构建期静态改掉**，运行时**不拦截任何系统调用**。

- **运行期路径层**（`runtime/crates/path-shim`、`runtime/crates/runtime-core`）：只处理路径，不处理 syscall 流。
  **已下沉进 loader**：不再是 `LD_PRELOAD`，而是编译成 ELF 后嵌进 `ld.so` 的第三个 `PT_LOAD`，
  由 `_dl_init` 钩子接管符号介入（`evidence/14`）。
- **构建期改写**（`tools/loader`、`runtime/crates/nr-tools`）：loader 入口桩、必要调用点 NOP、将来的 `PT_INTERP`/`RPATH`。
  这些改写的工具是 `runtime/crates/nr-tools`（纯 Rust，零依赖，`nr-tools scan-syscalls` / `nr-tools embed` /
  `nr-tools provision`）；旧的 Python / sh 版本留作**可对拍的参考实现**，两条路径产出同一个字节
  （`evidence/22`、`evidence/23`）。

## 1. 为什么只有这一个（排除表）

| 方案 | 机制 | syscall 成本 | 无 root 可用 | 致命缺陷 |
|---|---|---|---|---|
| **Husk 运行时（本方案）** | 原生进程 + 用户态路径解析 + 构建期改写 | **0**（内核直接执行） | 是（已实测） | 需要维护改过的 loader，随 glibc 版本重新定位改写点 |
| PRoot | ptrace 全量翻译每个 syscall | **50~100us × 每条**，一个 `ls` 就是几万条 | 是 | 慢一个数量级；dma-buf/DRM 映射极脆弱；本机连 guest ELF 都 exec 不了（`evidence/11`） |
| seccomp 转发（用户态通知） | 命中的 syscall 转发给常驻 supervisor | 仅命中项付 IPC 往返 | 是 | 要常驻进程 + 环形缓冲；只适合当兜底，不适合当主干 |
| chroot / mount namespace | 内核对路径空间隔离 | 0 | **否** | 无 root 不可用（`evidence/04`），直接排除 |
| 虚拟机 / 容器运行时 | 完整 guest 内核 | 0 | 否（要 `/dev/kvm`） | 与"不 root、不开虚拟化"矛盾 |
| bionic 容器 + 桥接图形栈 | 宿主代做渲染，guest 拿结果 | 低 | 是 | 多一层桥接与拷贝；与本方案并存可以，但不作为运行时的地基 |

结论：**只有本方案同时满足「无 root」「零 syscall 开销」「硬件路径原生」三个条件。**

## 2. 分层（谁负责什么）

```
L0  内核      真正执行 syscall、真正 mmap、真正 ioctl/dev 访问 —— 不经过我们
L1  loader    改过的 guest ld.so（承载一切，运行时不再有第二个对象）
               · 必要调用点 NOP（构建期定位，运行期零成本）
               · `bl _dl_init` 上挂一个 208 字节的钩子：mprotect 自己的可写窗口 → 调 nr_boot
L2  path shim 同一份 Rust no_std 代码，构建期嵌进 loader 的一个额外 R|E PT_LOAD
               · 自举：自己补 R_AARCH64_RELATIVE 的载入基址，自己解析 12 个 import
               · 然后遍历所有对象的 .rela.dyn/.rela.plt，把 76 个名字的 GOT 槽改成我们的函数
                 —— 这就是 LD_PRELOAD 的符号介入效果，手工做一次，构造函数之前完成
               · 76 而不是 45：libc 自己的代码大量走私有别名（`__open64_nocancel`、
                 `__gen_tempname` 内部 open），GOT 补丁看不见，所以还补了 stdio 入口
                 （`fopen/creat`）、`realpath/canonicalize_file_name`、`dlopen`、
                 `posix_spawn/eaccess`、以及 `_FORTIFY_SOURCE=3` 产生的 `__*_chk`/`*64` 孪生
                 （`evidence/16` 逐条给出触发它的程序）
               · 绝对/相对 guest 路径 → rootfd 下解析（符号链接感知、.. 截断）
               · execve 认出动态解释器后改走同一个 loader（子进程天然带 shim）
L3  产品壳     进程启动、环境变量、PTY、会话（后续）
```

## 3. 性能预算（PLE110 实测）

| 项目 | 现状 | 目标 |
|---|---|---|
| 系统调用 | **0 额外开销**（不拦截） | 0 |
| 路径类 libc 调用（open/stat/rename…） | 1 个组件 **+2.5us**、4 个组件 **+8us**（`evidence/18`：stat-real 2.9us → stat-guest 5.4us，deep 11.1us；每走一个组件约 2us） | 按 (父 fd, 名字) 缓存后 1~2us |
| 进程创建 exec | **与原生同量级**（guest 2.58ms vs 原生 bionic 8.00ms，`evidence/14`） | 已达成 |
| 启动 | 一次 `rt_sigaction` + 2 处静态 `nop` | 不变 |
| 内存 | **0 额外拷贝**（rootfs 直接 mmap，页缓存与原生一致） | 0 |

## 4. 明确不做（防止又滑回去）

- 不 ptrace、不 seccomp supervisor、不 FUSE、不 namespace、不虚拟机、不常驻翻译进程。
  （`runtime/crates/capability-probe` 里出现 `ptrace`/`unshare`/`mount` 是**诊断探针**：它只回答"这台机器允许什么"，
  用来写基线报告，运行时路径里一行都没有；APK 里的 `libprobe.so` 也不参与任何容器工作。）
- 不做图形/硬件的软件加速层（virgl、llvmpipe、软件解码）。硬件能不能用是 **SELinux 域权限**问题，不是运行时问题；本方案不参与、也不阻断。
- 不为了覆盖率去逐个 patch libc 调用点；libc 内部绕过 PLT 的调用（NSS、`realpath`）留给 loader 内解析 + 少量 `__*_chk` 入口解决。

## 5. 验收（硬标准）

1. `cd /etc && cat os-release` 类用例通过（虚拟 cwd 只暴露 guest 路径）。（已达成）
2. `execve` 全部走 runtime-core 解析器，无字符串前缀兜底；`PT_INTERP` 由 loader 处理。（已达成）
3. `untrusted_app` 域内 bash + coreutils 31 项矩阵全绿。**且不需要 `LD_PRELOAD`**（`evidence/12` 是 preload 阶段；
   `evidence/14` 是 shim 已在 loader 内、APK 里连 libpathshim.so 都不再打包：31 PASS / 0 FAIL / exit 0；
   `evidence/17` 是最终 loader（起始 cwd 改成 `/` 之后）复跑：31 PASS / 0 FAIL / exit 0，
   `evidence/18` 是同一 loader 的 11 项越界测试：0 escapes）
4. **gcc 真实编译（含 `cc1`/`as`/`ld` 子进程）+ make 构建通过。**（已达成，`evidence/16`；
   `evidence/17` 在最终 loader 上复跑）
5. 与 PRoot 同条件基准写入 `docs/design/`，含 PRoot 在本机不可用的证据。（**已达成**：`proot-comparison.md`
   + `evidence/19`——本机 PRoot 连宿主 bionic 程序都起不来，三类开销表按"本机 PRoot 不可测 / RMX3357 上可测"分开列）
6. 以上每项都有设备输出，存 `evidence/`，区分 FACT / INFERENCE / UNKNOWN。

## 6. 剩余工作（按顺序）

0. ~~在 `untrusted_app` 域跑通~~ **已完成**（§详见 `history/03-decision-log.md` §4.5）。
1. ~~路径解析下沉进 loader，去掉 `LD_PRELOAD`~~ **已完成**（`evidence/14`：shim 是 loader 的第三个 `PT_LOAD`，
   `_dl_init` 钩子做符号介入；31/31 通过；exec 2.58ms vs 原生 8.00ms，路径调用 +1.5us）。
2. ~~静态改写自动化~~ **已完成**（`evidence/15`）：`nr-scan-syscalls.py` 读 `.gnu.version_d` 自检版本，
   反查 `mov x8,#N` 定位 `svc` 调用点，并断言「后继指令只写 x0 不读 x0」（证明返回值被忽略）；
   版本不符/数量不符/调用点消失/后继读 x0 四种情况一律 `REFUSING` 且不写文件。产物与手写偏移逐字节一致。
3. ~~gcc / make 覆盖~~ **已完成**（验收第 4 项，`evidence/16`）。顺带解决了两件事：
   工具链供应脚本化（`tools/device-tests/provision-toolchain.sh`；2026-11-22 已 Rust 化为
   `nr-tools provision`，阶段目录逐文件对拍 2866 项零内容差异，`evidence/23`），
   以及 guest 起始 cwd 改为 `/`（`evidence/17`，第一版把子进程的继承 cwd 也重置了，
   31 项矩阵掉到 28，改成"只在 root 外时才 chdir"后回到 31/31）。
4. ~~基准对照补齐~~ **已完成**（验收第 5 项，`proot-comparison.md`、`evidence/19`）：
   本机 PRoot 不可执行（ENOENT→EACCES，四种 tmp 配置 + 自带 loader 绑定 + 宿主 bionic 控制组），
   RMX3357 上同一 harness 给出"每条 syscall 一次 ptrace 往返"（5000 次 stat 0.41–0.76s vs 0.06s）；
   本机 shim：`getpid` 196ns/次、stat 1 组件 4.3–5.4us、4 组件 11–12us、exec 2.49–2.53ms。
5. ~~seccomp trap 兜底~~ **已定论**（`evidence/20`）：handler 现在是**可信兜底**（直接调用被 trap 的
   `set_robust_list`/`rseq`：log 模式 rc=159 且留记录，enosys 模式返回 ENOSYS 并存活；
   31 项矩阵 + gcc/make 全程在 `NR_SIGSYS=enosys` 下通过）。但**不**把它当覆盖率手段——
   主机制仍是"构建期 nop 掉两个调用点，运行时一次都不 trap"。
6. 路径层的覆盖补洞：libc 内部调用绕过 PLT 的路径访问（`evidence/16` 已按症状补了 7 类入口，
   但 `freopen`/`glob`/`nftw`/`dlmopen` 仍未覆盖；`execl` 家族见 §7）。
6. 与 LinuxHub 2.0 集成，保留 PRoot 作为对照开关（不是回退默认）。

## 7. 已知未解决（别当能力）

- seccomp trap 的 `-ENOSYS` 兜底**已验证**（`evidence/20`）：`set_robust_list(99)`/`rseq(293)` 直接调用时
  log 模式 rc=159 并留记录、enosys 模式返回 `ENOSYS` 且进程存活；31 项矩阵与 gcc/make 全程在
  `NR_SIGSYS=enosys` 下通过。适用范围仍然有限：它只对"缺这条 syscall 时有官方回退路径"的可选加速器成立，
  **不是**通用 syscall 模拟层；默认路径依旧是静态 NOP + 一次都不 trap（成本 0）。
- ~~静态偏移是写死的~~ 已由第 2 项解决（`evidence/15`）。现在的失败模式是：换 glibc 后扫描器的版本/数量断言
  直接报错，需要人看一眼新报告再更新期望值 —— 这是设计好的行为。
- **`execl/execle/execlp` 拦不到**（`evidence/18`）：glibc 的 execl 自己拼 argv 后走**内部** execve，
  调用方没有 GOT 槽可改；稳定版 Rust 无法定义 C 变参函数，汇编蹦床又会被 rustc 的 cdylib 导出表
  本地化（nr-embed.py 读不到它）。影响面很小：`execve/execv/execvpe/execvp/posix_spawn*` 都已拦，
  shell、make、gcc、python 全部走这些。guest 里的 C 程序请用 `execve(2)`/`execv(3)`/`posix_spawn(3)`。
- ~~guest 起始 cwd 继承宿主路径~~ 已修（`evidence/17`）：`nr_boot` 第 4 步做一次 chdir，
  仅当继承的 cwd 在 root 之外时才生效。
