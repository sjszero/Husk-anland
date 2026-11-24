# evidence 159 — SIGSYS 兜底改成「按调用表回答」；ES2 上限的两条新事实

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB。
安装：`/data/local/tmp/husk-c4`（不装 APK，evidence 157），会话由 `husk-session.sh` 起，
运行在 **adb shell uid** 下（不是 APK 的 `untrusted_app`）。
时间：2026-11-24（CST）。

## 1. 改了什么（代码，FACT）

以前 `NR_SIGSYS` 只有两种行为：不设就「记一行然后照原样死 159」，设 `enosys` 就**所有**
被 seccomp 拦下的调用一律回 `-ENOSYS`。后者能用，但是一刀切：一个我们并不了解语义的调用
也会被伪装成「内核不支持」，这在原理上就是在撒谎。

现在默认是第三档：**按表回答**（`sigsys_answer()`，`runtime/crates/path-shim/src/lib.rs`）。
表里只放「文档化的失败方式就是 ENOSYS，调用方本来就带老路径」的调用：

| 家族 | 编号（aarch64） | 为什么可以回 ENOSYS |
|---|---|---|
| `clone3` | 435 | glibc / pthread 见 ENOSYS 就退回 `clone`，这是它的既定分支 |
| `rseq` / `membarrier` / `set_robust_list` | 293 / 283 / 99 | 返回失败即可继续，glibc 不把它当致命错误 |
| io_uring | 425 / 426 / 427 | 老内核就是 ENOSYS，程序都有非 io_uring 路径 |
| `openat2` / `faccessat2` / `fchmodat2` / `close_range` | 437 / 439 / 452 / 436 | 各自都有 at 系列老调用兜底 |
| 新 mount API | 428 – 433 / 442 | 老 `mount(2)` 是 fallback |
| landlock | 444 / 445 / 446 | 调用方一律把 ENOSYS 当「没有该子系统」 |
| pidfd / process_madvise / memfd_secret 等 | 424 / 434 / 438 / 440 / 447 / 448 | 同上 |
| futex 新接口 | 449 / 454 / 455 / 456 | 老 futex 兜底 |
| statmount / listmount / lsm_* / mseal / cachestat / map_shadow_stack | 451 / 453 / 457 – 462 | 同上 |
| `statx` | 291 | glibc 退回 `fstat` |

表外的调用**保持内核原来的语义**（记一行，照死，退出码仍是 128+31=159）。这样「我们不知道
怎么安全回答」的情况不会被静默改写成成功或某种 errno。

`NR_SIGSYS=log`（只记录、照旧死）保留给观察；`NR_SIGSYS=enosys`（一刀切）降级为实验开关。

## 2. 实测（FACT，raw `svc #0`，完全不经过 libc）

探针用内联汇编直接发系统调用，绕开 glibc 包装，因此在 adb shell uid 下：

```
set_robust_list  nr= 99 -> -22 (kernel answered)
membarrier       nr=283 -> 127 (kernel answered)
rseq             nr=293 -> -22 (kernel answered)
io_uring_setup   nr=425 -> -38 =ENOSYS
clone3           nr=435 -> -38 =ENOSYS
landlock         nr=444 -> -38 =ENOSYS
pidfd_open       nr=434 -> -22 (kernel answered)
getpid           nr=172 -> 26201 (kernel answered)
```

**这张表不能当处理器生效的证据**：4.19 内核本来就没有 `clone3` / `landlock` / io_uring，
那三个 `-38` 是内核自己回的，不是我们回的。`NR_SIGSYS=log` 在同一批调用上也没有造成 159，
说明 adb shell uid 的 seccomp 策略**没有 trap 这些调用**——trap 只发生在 APK 的
`untrusted_app` 域（evidence 14 的现场）。

所以：**UNKNOWN（关键）** —— 新的按表回答只有代码与冒烟证据，缺 `untrusted_app` uid 下的
真 trap 复验。复验方法已明确：把 0.02 的包装进测试 APK，用同一份 raw-svc 探针跑一遍，
期望 `clone3 → -38` 且进程存活、`NR_SIGSYS=log` 下仍是 159。

冒烟（FACT）：`./husk run ubuntu -c 'id'` → `uid=0(root)`；`/etc/os-release` 正常；
`make check` 全绿（fmt / cargo check / clippy / 53 个 shell 脚本 / 180 个 markdown 链接 / 版本）。

## 3. GPU：ES2 上限的两条新事实

### 3.1 事实（`eglinfo` 全量输出，surfaceless 平台）

```
EGL driver name: swrast
OpenGL compatibility profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL compatibility profile version: 2.1 Mesa 27.0.0-devel (git-b84f02a451)
OpenGL compatibility profile shading language version: 1.20
OpenGL ES profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile version: OpenGL ES 2.0 Mesa 27.0.0-devel (git-b84f02a451)
OpenGL ES profile shading language version: OpenGL ES GLSL ES 1.0.16
```

注意两件事：扩展列表是**完整的 zink 集合**（`GL_ARB_gpu_shader5`、`GL_ARB_texture_storage`
等都在），但版本号只有 2.1 / ES 2.0、GLSL 1.20 / ES GLSL 1.00 —— 版本号和扩展列表对不上，
说明挡住的不是「驱动没能力」，而是版本声明这一层。

顺带纠正一条：这个 rootfs 里 `libEGL_mesa.so` 内嵌的默认 gallium 名字就是
`libgallium-27.0.0-devel.so`，也就是实际生效的那份；同期还存在一份 `25.2.8-0ubuntu…` 的旧件，
不参与。**zink 是这棵树里 GL 的默认选择**，不设 `MESA_LOADER_DRIVER_OVERRIDE` 也是 zink
（实测：`HUSK_GPU=llvmpipe` 只是不注入 env，renderer 仍然是 zink），所以「退回 llvmpipe 对照」
这条路在当前树里走不通。

### 3.2 读到源码层的一条线索（INFERENCE，未证）

Mesa 主干版本号就是 `27.0.0-devel`，与 rootfs 里的编译版本一致。主干里：

- `zink_screen.c` 只设 `caps->glsl_feature_level` 与 `glsl_feature_level_compatibility = 460`，
  **不设 `caps->essl_feature_level`**；
- `st_extensions.c:1229` 用 `screen->caps.essl_feature_level` 作为 `ESSLVersion`，
  后面所有 ES3 门禁（`ESSLVersion >= 300/310/320`）都看它。

推论：这棵树里 zink 报 ES2 / GLSL ES 1.00 的机制，很可能就是 `essl_feature_level` 为 0，
而 `MESA_GLES_VERSION_OVERRIDE` 恰好在版本声明这一步把上下文抬上去，所以「override 能用」
和「默认只有 ES2」两件事可以同时成立。

这仍是推论，缺一步：拿到这棵树里 zink 填 caps 的确切位置（或换一份明确设 `essl=320` 的
Mesa），再用真 ES3 workload 验证。**在此之前桌面仍按「GPU 能算、版本声明低」记录，不写
「已硬件加速」。**
