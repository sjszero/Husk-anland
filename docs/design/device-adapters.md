# 跨设备兼容：我们依赖哪些前提，怎么探测，怎么降级

> 目的：今天只有一台 RMX3357 可以测，但代码不能只对这一台成立。
> 本文只写**我们自己的**约定与探测表。同类项目的实现细节记在仓库外
> `husk-notes/prior-art/`（`mali-kbase-prior-art.md`、`reference-audit-matrix.md` 等），
> 仓库里不放别人的方案。

## 1. 页大小（4 KiB / 16 KiB / 64 KiB）

这是唯一一个"假设错了就整个运行时起不来"的前提。

| 环节 | 约定 | 现状 |
|---|---|---|
| 构建 | 所有我们产出的 ELF（shim、reader、工具）用 `-z max-page-size=0x10000`（4K/16K/64K 都合法）和 `separate-loadable-segments` | 已执行（`tools/loader/build-embedded-loader.sh`） |
| 结构 | 每个 `PT_LOAD` 必须满足 `p_offset ≡ p_vaddr (mod 0x4000)`；`nr-tools embed`/`libc-interpose` 写入前断言 | 已断言 |
| 体检 | `nr-tools check-elf` 静态检查一组 ELF 的对齐（4K/16K/64K 三档） | 4K 实机通过；16K/64K **UNKNOWN** |
| 运行期 | 从 auxv 读 `AT_PAGESZ` 存进 `KERNEL_PAGE`，代码里禁止字面量 4096 | 已实现 |
| guest 侧 ELF | rootfs 里的库/程序由发行版构建，可能只对齐 4K；装载由我们的 loader 完成，不内核对齐 | 未测 16K 设备 |

注意：**"4K 构建"不等价于"只能在 4K 内核跑"**。能不能装取决于 `p_align`、段偏移和装载器实现，
上面那条同余约束才是真正的判据。

## 2. 内核与 Android 版本带来的行为差异（都写在探测里，不写死）

| 差异 | 本机表现（4.19 / Android 13） | 代码里的处理 |
|---|---|---|
| `set_robust_list`、`rseq` | app seccomp 回 `SECCOMP_RET_TRAP`（SIGSYS） | 构建期把这两个 `svc` 点换成 `nop`（`nr-tools scan-syscalls`，带版本与计数断言） |
| `memfd_create` 的 `MFD_CLOEXEC` | **实测 EINVAL**（今天踩到），`flags=0` 正常 | 用 `fcntl(F_SETFD, FD_CLOEXEC)` 补，避免依赖这个 flag |
| `/proc/stat`、`/proc/net/*`、`/proc/sys/*` | SELinux 对 `untrusted_app` 拒绝 | 见 `server-surface.md` 的墙表；不合成假数据 |
| `targetSdk` 影响 SELinux 域 | 实测 28 与 33 **没有差别**（`/proc` 仍被拒） | 保持 33，不为它降 targetSdk |
| `openat2`、`SECCOMP_RET_USER_NOTIF` | 4.19 没有（ENOSYS / 被杀） | 不使用；路径解析自己做 |
| 页大小 | 4 KiB | 见第 1 节 |

结论：**能力探测必须发生在 app 域、在真机上**，不能用 `adb shell`（shell 域）的结果推断。

## 3. SoC 适配面：GPU / 显示 / 音频

一张启动时填的表，按"节点 + 属性"判定，不按机型名。

| 家族 | 判据 | 用户态度路径 | 我们的适配器 |
|---|---|---|---|
| ARM Mali（kbase） | `/dev/mali0` 可打开；kbase UAPI 版本可从 `VERSION_CHECK` 取 | 上游 Mesa `panfrost`/`panvk`（kbase 后端），DRM 语义层翻译 | 已在本机跑通（evidence 46/47） |
| Qualcomm Adreno（kgsl） | `/dev/kgsl-3d0` 存在；`/sys/class/kgsl/kgsl-3d0/*` | 上游 Mesa `freedreno`/`turnip`，走 kgsl 或 msm DRM | **未实现**：适配器接口同一套，命令字不同 |
| 通用 DRM 渲染节点 | `/dev/dri/renderD128` 可打开 | Mesa 原生 | 预留 |
| PowerVR / 其它 | 有节点但无上游驱动 | 无 | 明确降级到软件渲染 |
| 无可用节点 | — | `llvmpipe` | 代码内降级 + 日志说明原因 |

要点（本机实测，别的机器要重测，不能外推）：

- 厂商 `libGLES_mali.so` 带 25 个 bionic `DT_NEEDED`，**不能**在 glibc guest 里加载 —— 这条路对所有
  Android 设备都一样，不要重复尝试。
- kbase 的命令字必须从真实客户端反读，不能猜（猜出来的组合会产生假阴性，见 evidence 36）。
- 显示：本机没有 `/dev/dri`、没有 `/dev/dma_heap`、`/dev/ion` 在 guest 侧 EACCES，所以出图只能走
  `AHardwareBuffer` → `ANativeWindow`；有 `/dev/dma_heap` 的机型可以换成 dma-buf 直出。
- 音频：本机 `/dev/snd` 打不开，只能在 app 进程里用 AAudio，再把 PCM 通过独立 socketpair 送进 guest。

## 4. 适配器的形状（代码层）

```rust
// 探测一次，缓存；只返回"可用/不可用 + 原因"，不返回布尔
struct DevicePlan {
    page_size: usize,
    gpu: GpuBackend,        // MaliKbase | AdrenoKgsl | DrmNode(..) | Software(why)
    display: DisplayPath,   // AHardwareBuffer | DmaBuf | Software
    audio: AudioPath,       // AAudioBridge | Alsa(..) | None
}
```

规则：

1. 探测失败必须给出**原因字符串**，并落到明确的降级路径；任何一条都不能崩。
2. 适配器按**芯片家族**写（Mali kbase / Adreno kgsl / DRM），不按机型写一次性特判。
3. 新增一个家族时，先补探测行 + 降级行，再写实现；没有实现的家族必须显式报"不支持"。

## 5. 待补的设备侧验证（真机到手就做）

- 16 KiB 页设备：`nr-tools check-elf` + 起 bash + 起 GNOME Shell，各一项。
- Adreno 设备：读 kgsl UAPI 版本、最小 `KGSL_IOCTL` 探针、`turnip` 能否枚举。
- `/dev/dma_heap` 可用机型：零拷贝显示路径。
- 内核 ≥ 5.10 的设备：`openat2`、`SECCOMP_RET_USER_NOTIF` 是否可用（可用就能省掉一层路径解析）。
