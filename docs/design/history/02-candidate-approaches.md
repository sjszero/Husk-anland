# 历史：候选方案的分层枚举（2026-11-22）

> **存档文件。** 这是定方案之前的候选枚举，当时的 T0–T4 只是层级标签。结论已被
> [../runtime.md](../runtime.md) 取代：现在只保留「运行期路径层 + 构建期改写」这一条路，
> 下面的标签不再作为术语使用。

## 结论先说

在无 root、不解 BL、不用虚拟化的前提下，**"零适配层"对整个 Ubuntu 用户态不成立**，这不是实现问题，是内核权限问题：

- `chroot` / `mount` / `unshare(userns|mountns)` 在应用域不可用（`03` 号证据：EPERM 或 SIGSYS）。
- 应用数据目录不可 exec（EACCES），只有 APK `nativeLibraryDir` 可执行。
- 动态 ELF 的 `PT_INTERP=/lib/ld-linux-aarch64.so.1` 是绝对路径，Android 根下不存在。
- Ubuntu 程序把 `/etc`、`/usr` 写死在代码和 glibc 里。

所以问题不是"要不要适配"，而是**把适配放在哪一层**。越靠近链接期，越接近原生。

## 层级（从最原生到最重）

| 层级 | 机制 | 每次操作额外开销 | 覆盖范围 | 本机可行性 |
|---|---|---|---|---|
| 无适配 | 内核直接执行，无任何适配 | 0 | 静态、无绝对路径依赖的程序 | 可行 |
| 链接期 | 安装时改写 ELF（`PT_INTERP`、RPATH、prefix） | 0（运行时无介入） | 被改写的二进制 | 可行但需改 rootfs |
| libc 层 | `LD_PRELOAD` 重写路径/exec | 纳秒级（实测 5000 次 stat 无可测差异） | 走 libc 的动态程序（Ubuntu 绝大多数） | **已实现** |
| syscall 层 | seccomp user notification，supervisor 代做 syscall | 微秒级 | 全部 syscall | 4.19 不支持 |
| ptrace 层 | 每个 syscall 停两次（PRoot） | 十微秒级 + 全局停顿 | 全部 | 可用 |

## 实测对照（同一 rootfs、同一设备、同一时段）

| 工作负载 | PRoot 5.x（ptrace 层） | 纯 Rust shim（libc 层） | 差距 |
|---|---|---|---|
| 100 次 glibc `cat` 进程 exec | 0.76 s | 0.18–0.25 s | 3–4× |
| 5000 次 `stat` | 0.41–0.54 s | 0.05–0.06 s（无 shim 亦 0.05–0.06 s） | 8–10× |
| 5000 次 `stat` 的 shim 额外开销 | — | 低于计时分辨率（< 10 µs/次，实际为百纳秒级） | ≈ 0 |

证据：`evidence/05-proot-vs-shim-20261122.txt`。

## 因此的最优架构

1. **默认层 = libc 层**（已跑通）：不挂 tracer、不逐 syscall 停，覆盖 Ubuntu 绝大部分程序。
2. **链接期改写作为可选优化**：安装 rootfs 时批量改写 `PT_INTERP`，消除 `execve` 解释器补全这一步；运行时零介入。
3. **无适配层给我们自己的组件**：用 Rust 写的运行时工具一律静态、无 `/etc` 依赖，直接用内核跑，零适配。
4. **ptrace 层 / syscall 层只做兜底**：静态 ELF、Go、直接 `svc` 的程序才下沉，且用 Rust 实现，不引入 C。
5. 桌面/GPU 沿用 LinuxHub 2.0 已验证的链路，不在运行时第一阶段重做。

## 仍需证明的点

- libc 层在 `untrusted_app` + `nativeLibraryDir` 下的覆盖率（现在用 runas_app 验证机制）。
- 链接期改写后 `ld.so` 缓存、`$ORIGIN`、SUID、硬链接的行为。
- `cwd` 虚拟化与 `..` 逃逸防护（安全边界，必须先于性能优化完成）。
