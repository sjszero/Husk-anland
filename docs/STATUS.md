# 当前状态（2026-11-22 18:20）

## 已完成

运行时本体与构建工具都已验证（见 README 与 `docs/RESULTS.md`）。
今天新增两项实测与一项集成：

| 项 | 结果 | 证据 |
|---|---|---|
| PTY 可用性 | `/dev/ptmx` 可打开 | evidence 24 |
| GPU 可达性 | `/dev/kgsl-3d0` 可打开（Adreno825），`/dev/dri/*` 拒绝，guest 看不到 `/vendor` | evidence 24 |
| 终端壳集成 | 既有 `com.terminal` 应用启动 Husk 会话的代码已编译、已安装、rootfs 已就位 | `integrations/terminal-app` |

## 已解决（2026-11-22 18:34）

**交互式会话可用。** 根因是 glibc 2.35+ 启动时注册 `rseq`，被 Android seccomp 判为 SIGSYS，
且发生在 path shim 装 SIGSYS 处理器之前——所以既无输出也无日志。
探针启动器一直带 `GLIBC_TUNABLES=glibc.pthread.rseq=0`，新写的启动器漏了。详见 `native-runtime/evidence/25`。
顺带修掉一个真 bug：fork 之后调用 JNI（未定义行为），现在 argv/envp 在 fork 前准备好。

自检 `bash -c 'echo HI; id; pwd; ls /'` 退出码 0；会话内 `uname -a` 输出内核 6.6.118 aarch64；`ps` 可见 `libld.so … bash -i`。

## 仍未解决（不阻塞"能打字"）

- 交互式提示符（PS1）不显示：命令能执行，但 shell 不画提示符（外观问题，下一步）；
- 输入框聚焦、`TIOCSWINSZ` 尺寸变化、切后台/重建：未处理；
- 会话内 `pwd` 是 `/root`（启动器 chdir 到 rootfs/root），改成 chdir(rootfs) 即为 `/`。

## 未开始

- PTY 尺寸变化（`TIOCSWINSZ`）已由宿主处理，未验证全屏程序（`vim`/`htop`）；
- 会话生命周期（切后台、息屏、重建）；
- 图形桥、音频桥（见 `docs/FUTURE-WORK.md`）；
- 16K 页内核、多设备验证。
