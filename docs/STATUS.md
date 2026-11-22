# 当前状态（2026-11-22 18:20）

## 已完成

运行时本体与构建工具都已验证（见 README 与 `docs/RESULTS.md`）。
今天新增两项实测与一项集成：

| 项 | 结果 | 证据 |
|---|---|---|
| PTY 可用性 | `/dev/ptmx` 可打开 | evidence 24 |
| GPU 可达性 | `/dev/kgsl-3d0` 可打开（Adreno825），`/dev/dri/*` 拒绝，guest 看不到 `/vendor` | evidence 24 |
| 终端壳集成 | 既有 `com.terminal` 应用启动 Husk 会话的代码已编译、已安装、rootfs 已就位 | `integrations/terminal-app` |

## 未解决（阻塞"能打字"）

**现象**：forkpty 子进程 exec 我们的 loader 后，bash 不产生任何输出，PTY 立即 EOF；`waitpid` 取不到状态。
宿主 UI 与自建壳表现一致，因此不是 UI 问题。

**已排除**：
- `/dev/ptmx` 打不开（实测可打开）；
- exec 失败（失败时会在子进程 stderr 打印，日志里没有）；
- rootfs 缺失（loader 与 bash 均存在，`ls` 正常）；
- 非交互模式（同 loader + 同 rootfs，用 `bash /script` 在探针里 31/31 通过）。

**下一步实验（按代价排序）**：
1. 在应用内用同一 argv 跑 `bash -c 'echo hi'`（非交互），确认 loader 在 forkpty 下是否本身就失败；
2. 若非交互正常 → 问题在交互式 bash（job control / `/dev/tty` / terminfo），改用 `bash --noprofile --norc -i` 或换成 `dash -i` 对比；
3. 用 `waitpid(WNOHANG)` 轮询拿到真实退出码/信号，替代阻塞等待；
4. 若仍无输出，检查 shim 在 `chdir`/`getcwd` 上的假设（evidence 17 的"起始 cwd=`/`"逻辑）。

## 未开始

- PTY 尺寸变化（`TIOCSWINSZ`）已由宿主处理，未验证全屏程序（`vim`/`htop`）；
- 会话生命周期（切后台、息屏、重建）；
- 图形桥、音频桥（见 `docs/FUTURE-WORK.md`）；
- 16K 页内核、多设备验证。
