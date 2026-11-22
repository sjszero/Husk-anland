# 会话生命周期——完整规格

> 适用：`app-rs`（纯 Rust NativeActivity 壳）。本文是**规格**，代码是它的实现，`docs/design/evidence/`
> 里的日志是它的证据。三者不一致时以本文为准，先修代码再改本文。
>
> 一句话：**会话属于进程，不属于窗口；全进程只有一个回收者；关停必须升级。**
>
> 10 分钟长稳不是目标本身，它是本文 §4 不变量全部成立时的一条推论。

---

## 1. 参照物：机制级对照（不是"抄个概念"）

| 参照物 | 它要解决的失败模式 | 它的具体机制 | Husk 取什么 / 不取什么 |
|---|---|---|---|
| **proot** | 多个 waiter 抢 `waitpid` 的状态 → 僵尸、状态丢失 | tracee 挂单链表；`waitpid(tracee_pid)` 只在 tracer 主循环一处；信号由 tracer 转发给 tracee（因为 ptrace 拦走了） | **取**：全进程唯一 waiter。**不取**：信号转发 —— 我们不 ptrace，信号由内核直接送到 PTY 前台进程组，更准 |
| **Termux** | 窗口重建 = 会话被销毁（早期实现） | 会话住在 `TermuxService`（前台服务）里，`TerminalSession` 持有 PTY fd；`terminal-emulator` 只负责渲染；会话结束保留屏幕内容，用户点一下才新建；退出用退出码写屏 | **取**：会话住在进程里、屏幕保留、退出原因写屏、点击才重生。**不取**：前台服务 + 通知（我们用 Activity 进程本身，进程死=会话死；Android 保证 fd 关闭→guest 收到 SIGHUP） |
| **tini / runc-shim** | 孤儿进程没人回收；关停太粗暴丢收尾 | `PR_SET_CHILD_SUBREAPER` 当 PID 1；`SIGCHLD` 循环回收；信号转发给子进程；关停按 SIGTERM→(超时)→SIGKILL 升级；退出码回传 | **取**：subreaper、唯一回收、升级关停、退出码写屏。**不取**：把 guest 当 PID 1 影子（我们不是 PID 1） |
| **Android NativeActivity** | 假设"窗口一直在"、"onDestroy 一定会来" | 回调序列 onStart/Resume/Pause/Stop/Destroy + onWindow*/onInputQueue*；Surface 可以随时销毁重建；进程可能被系统直接杀，`onDestroy` 不保证 | **取**：把会话与 Surface 解耦；teardown 同时挂在 `onDestroy` **和** PTY-close 两条路上（后者覆盖"没收到 onDestroy"） |
| **tmux**（对照，不实现） | 断线后会话要保留 | 服务端进程 + socket 重连 | **不取**：我们没有服务端进程，也不在做跨进程持久化 |

---

## 2. 模型：四层对象与归属

```
进程(app)  ──拥有──►  会话(session: 1 个 PTY master + 1 个会话首进程)
   │                        │
   │                        └── pgid == sid == 会话首进程 pid（setsid 保证）
   ├──拥有──►  Surface（可随时销毁重建，只影响渲染）
   └──拥有──►  InputQueue（attach/detach 到 looper）
```

铁律：

1. **只有进程拥有会话**；Surface / InputQueue / 窗口焦点都不拥有会话。
2. 会话首进程是**进程组长 + 会话首进程**，所以 `kill(-pid, sig)` 能一次打死整组（bash + 它的作业）。
3. 一个进程里**同时最多一个活会话**。
4. 回收者只有 reaper 线程一个，任何人不得再调 `waitpid`。

---

## 3. 状态机与事件矩阵

状态只有三个（第四个是"进程结束"，不是可停留状态）：

* `Idle`：无会话（没有窗口尺寸之前、或没有 rootfs、或 teardown 之后）
* `Running`：有 PTY master + 活的会话首进程
* `Exited`：子进程已退出，屏幕保留并写明原因

| 事件 | Idle | Running | Exited |
|---|---|---|---|
| 已知窗口尺寸（首次布局） | **spawn** → Running | 忽略 | 忽略 |
| `onWindowDestroyed` | 无操作 | 只丢 GL 对象，**状态不变** | 只丢 GL 对象 |
| `onPause` / `onStop` | 无操作 | **继续读 PTY**（不读会让子进程阻塞） | 无操作 |
| `onStart` / `onResume` | 无操作 | 重建 GL，**不重启会话** | 重建 GL |
| PTY 读 EIO | —— | `close_master()` → 等 reaper 报 Exited | 无操作 |
| reaper 报到本会话 pid 退出 | —— | → `Exited`（写 `exit N` / `signal N`） | 无操作 |
| reaper 报到别的 pid（孙进程） | 记日志 | 记日志 | 记日志 |
| 用户点击屏幕 | 无操作 | 无操作（点击已有会话只滚回/聚焦） | **restart**（400ms 限流） |
| `KEYCODE_BACK` | finish | `ANativeActivity_finish`（→ onPause/Stop/Destroy） | 同左 |
| `onDestroy` | 无操作 | **teardown** → 进程结束 | 无操作 |
| 进程被杀（无 onDestroy） | —— | 内核关闭 master → SIGHUP → guest 退出（reaper 收） | —— |

---

## 4. 不变量全集

分五组，共 15 条。**每条都必须有"代码位置"和"怎么验"**，缺一条就是缺口。

### A. 进程与回收

| # | 不变量 | 代码 | 怎么验 |
|---|---|---|---|
| A1 | 全进程只有一个 `waitpid(-1)` 主人 | `start_reaper` (lib.rs:557) | 任何时刻 `ps -A -o STAT \| grep -c Z` 不增长 |
| A2 | 本进程是 subreaper，孙进程重挂到这里 | `prctl(PR_SET_CHILD_SUBREAPER)` (lib.rs:563) | `bash -c 'sh -c true'` 后无孤儿、无僵尸 |
| A3 | reaper 遇 EINTR 重试、遇 ECHILD 休眠不空转 | 同 A1 循环体 | 日志无 `reaped orphan` 风暴；CPU 空闲时无抖动 |

### B. 会话与 PTY

| # | 不变量 | 代码 | 怎么验 |
|---|---|---|---|
| B1 | 一个进程只 spawn 一次，重生只能由用户动作 | `maybe_start` (lib.rs:110)、`restart_session` (lib.rs:129) | 会话死后 pid 不再增长（不会再出现 700+ loader） |
| B2 | 重生 400ms 限流 | `restart_session` | 连点两次只起一个会话 |
| B3 | 会话首进程 `setsid()` + `TIOCSCTTY`，pgid==sid==pid | `Term::spawn` (term.rs:126/137) | 日志 `session: running pid=…`；`ps -o PID,PPID,PGID,SID` 三者相等 |
| B4 | fork→execve 之间只用 async-signal-safe 调用；信号掩码清零；除 stdio 外的继承 fd 全部关闭 | `Term::spawn` (term.rs:126–160) | 代码审查 + guest 内 `ls /proc/self/fd` 只有 0/1/2 |
| B5 | 关 PTY master 必须先 `ALooper_removeFd` 再 `close` | `close_master` (term.rs:261)、`teardown` (lib.rs:172)、`ensure_term` (lib.rs:202) | 重启会话后无"幽灵输入"；`/proc/<pid>/fd` 无重复注册 |

### C. 表面、输入、尺寸

| # | 不变量 | 代码 | 怎么验 |
|---|---|---|---|
| C1 | Surface 销毁只丢 GL，不动会话 | `sync_surface` (lib.rs:249) | 切后台/旋转后会话 pid 不变，输出还在 |
| C2 | 后台继续读 PTY | `run()` 里 `id==1000` 分支 (lib.rs:584) | 后台 1 分钟再回来，输出没丢 |
| C3 | 尺寸变化走 `TIOCSWINSZ`，交给内核发 SIGWINCH | `Term::resize` (term.rs:185) | guest 内 `tput cols; tput lines` 与屏幕一致；`trap` 收到 WINCH |
| C4 | BACK 交给平台结束 Activity（不被当成输入吞掉） | `drain_input` (lib.rs:683) | 按 BACK 触发 onPause→onDestroy |

### D. 关停

| # | 不变量 | 代码 | 怎么验 |
|---|---|---|---|
| D1 | 关停顺序：先挂断（关 master），再 SIGTERM，再 SIGKILL | `teardown` (lib.rs:172) + `escalate` (lib.rs:531) | 日志有 `teardown(...)` → `group gone after SIGHUP`（或 SIGTERM/SIGKILL 两级） |
| D2 | teardown 同时挂在 onDestroy 与 PTY-close 两条路上 | `on_destroy` (lib.rs:385)、`Term::close_master` | `am force-stop` 后无残留 libld |
| D3 | 进程退出前 APP 置空并 teardown | `run()` 收尾 (lib.rs:584 之后) | `loop exit` 后日志有 `teardown(loop exit)` |

### E. 观测与退出信息

| # | 不变量 | 代码 | 怎么验 |
|---|---|---|---|
| E1 | 每次状态迁移一行 `session:` 日志 | lib.rs 各处 | `logcat -s Husk \| grep 'session:'` 能拼出完整时间线 |
| E2 | 退出原因写屏：`exit N` / `signal N (NAME)` | `Term::note_exit` (term.rs:279) | 屏幕出现 `[husk] session ended: …` + `tap to start again` |

---

## 5. 本轮实测（2026-11-22 21:43–21:47，RMX3357）

| 测试 | 命令 | 结果 |
|---|---|---|
| T0 启动 | `am start -n com.linuxhub.husk/android.app.NativeActivity` | `session: running pid=15293 (pgid=sid=pid by setsid) 80x24` ✅ |
| T1 打字不退出 | `input text "echo hi"; input keyevent 66` | `cb: onDestroy` 计数 = 0 ✅ |
| T4 切后台 | `input keyevent 3` | `cb: onPause`，**无 teardown**，libld 仍在 ✅ |
| T5 回前台 | `am start …` | `cb: onStart/onResume`，**同一个 pid 15293**，未重启 ✅ |
| T7 会话退出 | `input text "exit"` | `session: exited pid=15293 status=0 (exit 0)`，libld=0 ✅ |
| T7b 点击重生 | `input tap 500 1200` | `session: restart requested by user` → `running pid=15490` ✅（400ms 限流生效） |
| T8 强杀（无 onDestroy） | `am force-stop com.linuxhub.husk` | libld=0，guest 未变孤儿 ✅ |
| T9 旋转/尺寸 | `wm size 1200x2400` | **无法测**：shell 缺 `WRITE_SECURE_SETTINGS`（旋转路径历史上已验，见 STATUS 附录 21:20） |

---

## 6. 本轮因此修掉的 4 个真 bug（FACT）

1. **BACK 被当成普通输入吞掉** → 用户永远退不出 App。现在 `AKEYCODE_BACK` → `ANativeActivity_finish`。
2. **`AKEYCODE_BACK` 拼写错误一度变成 match 通配绑定**（Rust 把未知标识符当变量绑定），
   后果是**任何按键都会结束 Activity**。编译器 `unused variable: AKEYCODE_BACK` 警告抓到了，
   常量补进 `sys.rs` 后消除。
3. **`onDestroy` 不 teardown** → guest 会活过它的读者。现在 onDestroy 里 teardown。
4. **同一进程重建 Activity 后永远不再起会话**：`DESTROYED` 是静态量，onDestroy 置 true，
   新 Activity 的 `run()` 第一轮就退出。现在 `onCreate` 里重置（同时清 WINDOW/INPUTQ/FOCUSED）。

附带修：`ALooper_removeFd` 先摘后关（B5）、`tee` 随 master 一起关、子进程 `sigprocmask` 清零 +
关闭 3..255 继承 fd（B4）、`setsid()` 失败直接 `_exit(126)` 而不是静默继续。

---

## 7. 还没做到（缺口，按影响排序）

| # | 缺口 | 影响 | 计划 |
|---|---|---|---|
| G1 | 旋转/尺寸变化在本轮**未实测**（shell 无 `WRITE_SECURE_SETTINGS`） | C1/C3 只有历史证据（本轮代码改动前） | 用 App 内自带的旋转按钮或 `input` 手势触发，或让 app 提供 `adb` 广播入口 |
| G2 | 未验证"系统 LMK 直接杀进程"路径（只验了 `force-stop`） | D2 的第二种入口假设 | 用 `am kill` / 长时间后台压内存 |
| G3 | 无 `onSaveInstanceState` / `onLowMemory` / `onConfigurationChanged` 处理 | 内存紧张或配置变化时没有降级动作 | 至少实现 `onLowMemory`：释放图集与滚动缓冲 |
| G4 | `escalate` 用 `kill(-pid, 0)` 判活，pgid 复用无防御 | 极端情况下误杀新进程组 | 记录会话启动时间/命令，判活时先核对 `/proc/<pid>/stat` 起点 |
| G5 | 没有 fd/线程泄漏的长时统计 | 长时间使用后可能累积 | 在 `session:` 日志里加 fd 计数（`/proc/self/fd`） |
| G6 | 大输出量下的 PTY 背压未测 | C2 只测了"空闲后台"，没测"输出狂刷时切后台" | soak 增加 `yes` 类高频输出阶段 |
| G7 | 10 分钟长稳尚未在本版代码上跑完 | A1/A2/C2 的合并验收 | 本轮已启动（`files/rootfs/tmp/soak2.sh`，每 30s `sync`），跑完写 evidence 32 |

---

## 8. 刻意不做（不是 bug）

* 不做自动重启（坏 rootfs 会变 fork 风暴，2026-11-22 实测 700+ 僵尸）。
* 不做多会话 / 分屏 / 跨进程持久化（tmux 式），要的话是产品功能，不是运行时正确性。
* 不做信号转发（我们不 ptrace，内核直接投递更准）。
* 不做 `SIGWINCH` 软件模拟（`TIOCSWINSZ` 足够）。
* 不在退出时回传退出码给 Java（NativeActivity 无通道），只写屏 + 日志。

---

## 9. 变更记录

| 日期 | 改动 |
|---|---|
| 2026-11-22 21:20 前 | 会话随进程存活；唯一 reaper + subreaper；点击才重启；Surface 解耦 |
| 2026-11-22 21:44 | teardown/escalate/close_master 上线；BACK 结束 Activity；onCreate 重置状态；修 match 通配 bug |
