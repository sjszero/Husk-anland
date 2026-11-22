# PLE110（Android 16 / GKI 6.6.118）能力矩阵（2026-11-22）

对照设备：RMX3357（Android 13 / 4.19 / MediaTek，`CONFIG_USER_NS` 未开启）。

## 实测

| 能力 | RMX3357 shell | RMX3357 app | PLE110 shell | PLE110 app |
|---|---|---|---|---|
| `openat2(RESOLVE_IN_ROOT)` | ENOSYS | 进程被杀 | **PASS** | **进程被 SIGSYS 杀死**（syscall 437 被 app seccomp 禁止） |
| `seccomp` NEW_LISTENER / USER_NOTIF | EINVAL | EINVAL | **PASS** | **PASS** |
| `unshare(NEWUSER)` | EINVAL | EINVAL | EINVAL（`CONFIG_USER_NS` 未开启） | EINVAL |
| `unshare(NEWNS/NEWPID)` | EPERM | 进程被杀 | EPERM | EPERM |
| `mount` / `chroot` | EACCES / EPERM | 进程被杀 | EACCES / EPERM | 进程被杀 |
| `ptrace` / `process_vm_readv` / `memfd` | PASS | PASS | PASS | PASS |
| `/dev/fuse` | EACCES | EACCES | EACCES | EACCES |

原始输出：`evidence/06-ple110-shell-domain-20261122.txt`、`evidence/07-ple110-app-domain-20261122.txt`。

## 结论

1. **namespace + chroot 路线彻底出局**：两台设备、两个内核版本，`CONFIG_USER_NS` 都没有开启；shell 也没有 `CAP_SYS_ADMIN`。Shizuku 改变不了这一点。
2. **`openat2` 只在 shell 域可用**：app 域调用 syscall 437 会被 app seccomp 直接杀进程（logcat：`seccomp prevented call to disallowed arm64 system call 437`）。所以"让内核做路径约束"只有 Shizuku 路径能享受。
3. **seccomp user notification 在 app 域可用**（6.6 内核）：意味着不需要 Shizuku 也能做"只拦必要 syscall、不挂 ptrace"的快速后端。
4. 4.19 设备两者都没有，只能在 libc 层做路径解析 + ptrace 兜底。

## 架构定稿

- **默认（in-app）**：LD_PRELOAD 路径层 + 用户态符号链接感知解析器；exec 用 loader；必要时 seccomp-notify（6.6+）。
- **Shizuku 增强（可选）**：`openat2(RESOLVE_IN_ROOT)` 做内核级防逃逸，路径安全交给内核。
- **老设备兜底**：字符串前缀 + 手动 `..`/symlink 归一化 + ptrace。
- **不再考虑**：chroot、mount namespace、user namespace、FUSE。
