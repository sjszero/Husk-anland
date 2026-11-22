# 阶段一：Android 权限边界与执行能力基线（2026-11-22）

状态：进行中。本文件只记录**已实测**的结果；未测项目明确标 UNKNOWN。

## 1. 测试对象与方法

- 设备：realme RMX3357（MediaTek 天玑），Android 13 / SDK 33，内核 `4.19.191+`，8 核，4 KiB 页。
- 应用：`com.linuxhub.probe`，minSdk 24，targetSdk 33，debug 签名，SELinux 域 `untrusted_app`，uid 10301。
- 探针：`linuxhub-runtime/crates/capability-probe`，Rust + libc，交叉编译 `aarch64-linux-android`（NDK 29）。
- 对照：同一二进制在 `adb shell`（域 `shell`，uid 2000）下运行，用于区分"内核能力"与"应用权限"。
- Activity 单测逐项运行（每次只跑一个能力），避免应用进入后台被 Android 冻结后误判为"挂死"。

## 2. 实测结果

| 能力 | shell 域 | untrusted_app 域 | 结论 |
|---|---|---|---|
| 执行 Android ARM64 ELF（`/system/bin/sh`） | PASS | PASS | 内核 ABI 正常 |
| `memfd_create` + `PROT_EXEC` 映射 | PASS | PASS | 可替代 ashmem |
| `ptrace(PTRACE_TRACEME)` | PASS | PASS | 现有 PRoot 机制可行 |
| `process_vm_readv` 读子进程内存 | PASS | PASS | 代理层可读 tracee 内存 |
| 直接 exec glibc 动态 ELF | — | ENOENT | `PT_INTERP=/lib/ld-linux-aarch64.so.1` 在 Android 根不存在 |
| exec 位于 app data 的文件 | — | **EACCES** | Android 10+ W^X 限制，`files/` 不可执行 |
| `openat2(RESOLVE_IN_ROOT)` | ENOSYS(38) | **进程被杀** | 4.19 无该 syscall；app seccomp 下不可用 |
| `seccomp(NEW_LISTENER)` | EINVAL(22) | EINVAL(22) | 4.19 无 `SECCOMP_RET_USER_NOTIF` |
| `unshare(CLONE_NEWUSER)` | EINVAL(22) | EINVAL(22) | 未启用 user namespace |
| `unshare(NEWNS/NEWPID)` | EPERM(1) | 进程被杀 | 无权限；app 域为 SIGSYS |
| `mount(tmpfs)` | EACCES(13) | 进程被杀 | 不可用 |
| `chroot` | EPERM(1) | 进程被杀 | 不可用 |
| 打开 `/dev/fuse` | EACCES(13) | EACCES(13) | FUSE 不可用 |

原始输出：`evidence/01-rmx3357-shell-domain-20261122.txt`、`evidence/02-rmx3357-app-domain-20261122.txt`、`evidence/03-rmx3357-execution-20261122.txt`。

## 3. 关键结论

### 3.1 直接原生执行 glibc 程序不可行（FACT）
`bash` 直接 exec 返回 ENOENT：内核按 `PT_INTERP` 的绝对路径在 Android 根下找 `/lib/ld-linux-aarch64.so.1`，找不到。用自带 `ld.so --library-path` 可以启动（FACT，runas_app 域实测输出 `GLIBC_LOADER_OK`），但：

- 所有绝对路径（`/etc`、`/usr`、`/var`）都落到 Android 根，`/etc/os-release` 不存在，`/` 不可读（FACT）。
- 子进程 exec 时不会自动带 loader，任何 `make`/`gcc`/脚本派生都会回到 ENOENT。

结论：在没有 chroot/mount 的前提下，"零拦截原生执行"只对静态链接程序成立；当前 Ubuntu rootfs 与 GNOME 桌面基本都是动态 ELF。因此路径重写与 exec 拦截不可回避。

### 3.2 应用数据目录不可执行（FACT）
`untrusted_app` 下 exec `files/exe-test` 与 `files/glibc/.../ld-linux-aarch64.so.1` 都是 EACCES。`nativeLibraryDir/lib*.so` 可以执行（探针本身即证据）。

架构含义：**所有 guest 可执行入口必须由 APK 的 `nativeLibraryDir` 中的加载器/包装器承担**，不能直接 exec rootfs 里的二进制。这也是现有 PRoot 使用 `libproot-loader.so` 的原因之一。

### 3.3 该设备不支持 proot-next 的核心机制（FACT + INFERENCE）
`SECCOMP_RET_USER_NOTIF` 自 Linux 5.0 引入；RMX3357 为 4.19，`NEW_LISTENER` 返回 EINVAL，且 proot-next 源码注释也把 EINVAL 定义为"kernel without user notification"（FACT：`/tmp/nr-research/proot-next/src/seccomp/filter.rs:399`）。

INFERENCE：proot-next 的"绝大多数路径 syscall 走 notify、只有 4 个走 ptrace"的加速结构，在 4.19 设备上退化为纯 ptrace，甚至不可用。Rust 重写本身不能绕过内核能力缺失。

### 3.4 app seccomp 对越权 syscall 是"杀进程"而非"返回 EPERM"（FACT）
`mount`、`chroot`、`unshare`、`openat2` 在 app 域触发进程终止（无返回值），logcat 出现 CrashBox 记录。任何运行时后端必须把这些 syscall 当"不可尝试"，而不是"试了再兜底"。

### 3.5 可用底座（FACT）
`ptrace` + `process_vm_readv` + `memfd_create` 在普通应用权限下全部可用，说明现有 PRoot 路线在 4.19 设备上仍是唯一可靠保底；优化空间在于**减少需要拦截的 syscall 数量**，而不是换掉 ptrace。

## 4. 对架构的直接影响（INFERENCE，待 PLE110 对照）

1. 后端必须按设备能力选择：`seccomp-notify`（内核 ≥ 5.0 且可用）→ `ptrace`（保底）。探测必须在 app 域做，不能用 shell 域结果代替。
2. 执行入口固定为 `nativeLibraryDir` 加载器；rootfs 二进制只能被加载（mmap + 跳转）或被 ptrace 改写 exec，不能被直接 exec。
3. 路径虚拟化需要覆盖 `execve`；LD_PRELOAD 可覆盖 libc 层文件调用，但覆盖不到直接 syscall 的程序，也不能覆盖静态 ELF。
4. GPU/桌面复用现有 LinuxHub 能力，不在第一阶段改动。

## 5. 未验证 / 下一步

- UNKNOWN：PLE110（骁龙 Android 16，内核版本待测）是否具备 `openat2`、`SECCOMP_RET_USER_NOTIF`、user namespace。需要实机对照。
- UNKNOWN：Android 8–12 各厂商内核的实际能力矩阵（本机仅覆盖 Android 13 / 4.19）。
- UNKNOWN：`/dev/fuse` 在 app 域为 EACCES 是 SELinux 还是内核配置；FUSE 路线暂不纳入。
- 待做：PRoot 与本机原生的基准对照（进程启动、stat、readdir、小文件创建、编译）。
- 待做：LD_PRELOAD 路径翻译原型 + `nativeLibraryDir` 加载器原型，验证能否在不改 ptrace 的情况下跑通 bash/coreutils。
