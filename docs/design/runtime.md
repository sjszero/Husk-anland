# 运行时方案（权威定义）— Husk（壳）

> 本文件是方案的权威定义，与它冲突的其他描述一律作废。设备证据见 [evidence/](evidence/)。

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
  `nr-tools provision`）。工具链现在只有 Rust 一套实现。

## 1. 为什么只有这一个（排除表）

| 方案 | 机制 | syscall 成本 | 无 root 可用 | 致命缺陷 |
|---|---|---|---|---|
| **Husk 运行时（本方案）** | 原生进程 + 用户态路径解析 + 构建期改写 | **0**（内核直接执行） | 是（已实测） | 需要维护改过的 loader，随 glibc 版本重新定位改写点 |
| PRoot | ptrace 全量翻译每个 syscall | **50~100us × 每条**，一个 `ls` 就是几万条 | 是 | 慢一个数量级；dma-buf/DRM 映射极脆弱；guest 的 `PT_INTERP` 还得单独做 loader 才能启动 |
| seccomp 转发（用户态通知） | 命中的 syscall 转发给常驻 supervisor | 仅命中项付 IPC 往返 | 是 | 要常驻进程 + 环形缓冲；只适合当兜底，不适合当主干 |
| chroot / mount namespace | 内核对路径空间隔离 | 0 | **否** | 本机 `CONFIG_USER_NS` 未开、app 域直接杀进程（见 `capability-baseline.md`），直接排除 |
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
                 （每条都由具体程序触发，测试脚本在 `tools/device-tests/`）
               · 绝对/相对 guest 路径 → rootfd 下解析（符号链接感知、.. 截断）
               · execve 认出动态解释器后改走同一个 loader（子进程天然带 shim）
L3  产品壳     进程启动、环境变量、PTY、会话（后续）
```

## 3. 性能预算

| 项目 | 现状 | 目标 |
|---|---|---|
| 系统调用 | **0 额外开销**（不拦截，程序自己的 `svc` 直接进内核） | 0 |
| 路径类 libc 调用（open/stat/rename…） | 每走一个路径组件付一次用户态解析（量级 1~2us/组件）；**本机尚未复测** | 按 (父 fd, 名字) 缓存后稳定在 1~2us |
| 进程创建 exec | 与原生同量级（结构性结论：不拦截、不 ptrace） | 保持 |
| 启动 | 一次 `rt_sigaction` + 2 处静态 `nop` | 不变 |
| 内存 | 0 额外拷贝（rootfs 直接 mmap，页缓存与原生一致） | 0 |

> 路径开销的具体数字要以本机实测为准，**目前 UNKNOWN**，测出来再写进这里和 `RESULTS.md`。

## 4. 明确不做（防止又滑回去）

- 不 ptrace、不 seccomp supervisor、不 FUSE、不 namespace、不虚拟机、不常驻翻译进程。
  （`runtime/crates/capability-probe` 里出现 `ptrace`/`unshare`/`mount` 是**诊断探针**：它只回答"这台机器允许什么"，
  用来写基线报告，运行时路径里一行都没有；APK 里的 `libprobe.so` 也不参与任何容器工作。）
- 不做图形/硬件的软件加速层（virgl、llvmpipe、软件解码）。硬件能不能用是 **SELinux 域权限**问题，不是运行时问题；本方案不参与、也不阻断。
- 不为了覆盖率去逐个 patch libc 调用点；libc 内部绕过 PLT 的调用（NSS、`realpath`）留给 loader 内解析 + 少量 `__*_chk` 入口解决。

## 5. 验收（硬标准）

1. `cd /etc && cat os-release` 类用例通过（虚拟 cwd 只暴露 guest 路径）。（已达成）
2. `execve` 全部走 runtime-core 解析器，`PT_INTERP` 与 shebang 都由 loader 处理。（已达成）
3. `untrusted_app` 域内 bash + coreutils 矩阵全绿，且不需要 `LD_PRELOAD`。（已达成）
4. gcc / make 真实编译（含 `cc1`/`as`/`ld` 子进程）通过。（已达成）
5. 越界测试 0 escapes；SIGSYS 收敛后 600 秒 / 11,360 次 fork-exec 零僵尸。（已达成）
6. 16 KiB / 64 KiB 页内核实机运行。**UNKNOWN**

失败组合与最小探针清单见 `acquire-fence.md`；每次改动后上述 1–5 必须重跑。

## 6. 剩余工作（按顺序）

1. 复测本机的路径开销与 exec 成本，写进 §3 与 `RESULTS.md`。
2. `libc-interpose.md` 里的槽位方案（apt/dpkg/locale 现在还读宿主文件）。
3. 桌面帧出屏：ScreenCast → PipeWire → dma-buf → AHB → SurfaceControl。
