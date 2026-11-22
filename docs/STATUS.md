# 当前状态（2026-11-22 夜）

## 能用

| 能力 | 状态 | 证据 |
|---|---|---|
| 普通应用权限下跑 glibc 用户态（`untrusted_app`） | 可用 | evidence 12, 14, 17 |
| 功能矩阵（bash + coreutils + 管道/重定向/子 shell） | 31 / 31 | evidence 17 |
| 越界与符号链接逃逸 | 0 escapes | evidence 18 |
| gcc 13.3 全链（`cc1`/`as`/`ld`）+ make | 通过 | evidence 16, 17 |
| 交互式会话（PTY 里跑 `bash -i`） | 可用 | evidence 25 |
| 设备可达性：`/dev/ptmx`、`/dev/kgsl-3d0` | 可打开 | evidence 24 |
| 壳应用（`app/`）单会话终端 | 旧版（Java+JNI）实测可用 | evidence 25 |
| 壳应用改为纯 Rust（`app-rs/`） | 已编译、已打包，**未上机** | UNVERIFIED |

交互式会话的根因修复：glibc 2.35+ 启动时注册 `rseq`，被 Android seccomp 判为 SIGSYS，
发生时路径层还没来得及装处理器——启动器必须带 `GLIBC_TUNABLES=glibc.pthread.rseq=0`。
同时修掉一个真 bug：`fork()` 之后调用 JNI 是未定义行为，现在 argv/envp 在父进程准备好。

## 能用但粗糙（界面）

- 提示符（PS1）不显示：命令能执行，但 shell 不画提示符；
- 输入框需要点一下才出键盘；
- `TIOCSWINSZ` 尺寸变化、全屏程序（`vim`/`htop`）未验证；
- 会话内 `pwd` 是 `/root`，应改为 `/`；
- 切后台 / 息屏 / Surface 重建后的会话生命周期未处理；
- 应用代码里仍留着开发期的自检与 `ProcessBuilder` 对照，产品化时删除。

## 未开始

- rootfs 的导入、选择、删除（现在靠 `adb` 手工 tar）；
- 终端渲染（现在是最小 ANSI 清洗，不是完整终端）；
- 图形桥、音频桥（见 [FUTURE-WORK.md](FUTURE-WORK.md)）；
- 16K 页内核、第二台设备、长稳测试；
- 与 LinuxHub 应用的正式集成。

## 设备与环境（重要）

- 2026-11-22 白天验证在 **PLE110**（Android 16 / kernel 6.6 / 4K 页）上完成，evidence 01–25 全部来自它。
- 当晚那块设备已断开，当前接的是 **RMX3357**（realme / Android 13 / kernel 4.19.191 / arm64）。
  这台机器上**还没有** `com.linuxhub.husk`、也没有 rootfs。
- 因此纯 Rust 壳应用目前只做到「编译 + 打包通过」，上机验证（原生 Activity 起来、EGL 出画面、
  PTY 里有 bash）**尚未执行**，不能算通过。

## 下一步（按依赖关系）

不依赖设备的（可先做）：

1. `nr-tools rootfs`：在主机上解析 Ubuntu noble 的包集、解包成 rootfs 目录、装改过的 libc 与 loader、
   打好 tar —— 现在 rootfs 只能靠现成 tar 包，这是最大的可用性缺口；
2. 滚动回看与更完整的 ANSI（光标保存/恢复、行内擦除、颜色表），配单元测试；
3. APK 打包去 gradle（aapt2 + 签名），需要设备验证，先不动。

依赖设备的（等机器接上）：

1. 纯 Rust 壳应用上机：启动、出画面、bash 交互、旋转/尺寸变化、`vim`/`htop`；
2. 在 RMX3357（4.19 内核 / Android 13）上重跑 31 项矩阵与越界测试 —— 目前全部结论只来自 Android 16；
3. `apt` 冒烟：dpkg/apt 已能跑（缺 `usr/share/dpkg` 的问题已用 provision 补上），
   还差 `apt-get update` 与一次真实安装。

---

# 2026-11-22 深夜追加：壳应用生命周期 + 第二台设备诊断

## 修掉的真 bug（FACT，已上机验证）

**会话生命周期**：旧实现把"启动会话"放在渲染循环里，子进程一死就下一帧重新 `fork`，
而且 `waitpid` 只在 PTY 读到 EIO 时用 `WNOHANG` 试一次、失败就再没人回收。
结果：一次运行 2 秒起了 171 个 loader、堆出 700+ 个僵尸 `[libld.so]`。
你看到的"一直闪退、一直点开退出"就是它。

现在的模型（对照 proot / Termux / tini 的做法，用 Rust 实现）：

- `start_reaper()`：**全进程唯一**的 `waitpid(-1)` 线程，先 `PR_SET_CHILD_SUBREAPER`，
  bash 的孙进程死亡后也重挂回我们、由它收割；`EINTR` 重试，`ECHILD` 时 `usleep` 不空转。
- 状态回传：reaper → `EXITS` 队列 → 渲染线程 `poll_children()` → 只交给 pid 对得上的 session。
- 启动**一次**（`maybe_start`），死了就死着，**点了才重启**（`restart_session`，400ms 限流）。
- 会话死亡原因**写到屏幕上**：`[husk] session ended: signal 31 (SIGSYS)`。
- `read_available` 见到 EIO 只标记一次 + `close`，不再每帧刷日志。

验证（RMX3357）：`onCreate` → `run: thread up` → `spawned pid=11338` → `pty read errno=5; session closed`
→ `session pid=11338 ended: signal 31 (SIGSYS)`。**一次 spawn、零僵尸。**

## 这台机器（RMX3357 / Android 13 / 4.19.191 / 4K）的新事实

| 事实 | 结论 |
|---|---|
| loader 每次都能跑到 `.init_array`（`sigsys.log` 每次 spawn 精确 +33 字节） | 自举期没有 syscall 被拦，**旧结论"自举被 SIGSYS 杀"错误** |
| app 域 `openat2(437)` 实测 = **TRAP**，我们自己装的 handler 跑到了（`rt_sigaction ret=0`） | 这台 ROM 的 app seccomp **可捕获可仿真**，不是 KILL |
| guest 子进程仍然 `signal 31 (SIGSYS)`，而 shim 的 handler 从未产生二进制记录 | 问题在**我们 loader 里的 handler 没装上/没生效**，不是 ROM 限制 |
| `dmesg` 被拒、logcat 无 `type=1326` | 这台 ROM 拿不到 syscall 号，需用 ptrace 旁路观察或让 handler 自己报 |

## 下一步

1. 让 `install_sigsys` 打印 `rt_sigaction` 的返回值与 errno（现在无条件打 "installed"），确认是"没装上"还是"装上了被覆盖"。
2. `nr-tools` 加 `check-elf`（页大小体检），构建期断言 `p_offset ≡ p_vaddr (mod 0x4000)`。
3. 页大小设计见 [design/page-size.md](design/page-size.md)。
