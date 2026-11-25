# 234 · 会话看门狗与关停闭环

日期：2026-11-26 01:46–01:53（CST）。设备：RMX3357 / Android 13 / aarch64。
本轮只处理 Husk 的会话生命周期，不修改桌面合成器，也不改变 X11 路线。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| Rust 会话看门狗被打进 APK | aarch64 PIE，APK 内路径 `lib/arm64-v8a/libhuskguard.so` | FACT |
| App 启动 guest 前先等待看门狗 ready | 日志 `guard pid=27484 ready`，随后 `spawned ... armed=true` | FACT，真机 |
| lease EOF 能发现 App 进程死亡 | `am crash` 后看门狗日志 `lease EOF: App process is gone` | FACT，真机 |
| 看门狗能清理已脱离会话组的进程 | 独立测试中 pid 27209 自建 session/pgid，lease 关闭后收到 SIGTERM，最终 `remaining=[]` | FACT，真机 |
| 当前 ROM 的 `am force-stop` | `com.linuxhub.myth.husk` 的 39 个同 UID 进程在 4 秒内变为 0 | FACT，真机 |
| 所有 ROM 的 force-stop/LMK 行为一致 | **UNKNOWN**；不同 ROM 的 AMS/SELinux 策略可能不同 | 未验证 |
| 清空环境变量后自行 setsid 的异常子进程 | 不再带 `NR_SESSION_ID`，只能靠进程组兜底；若同时脱离进程组则 **UNKNOWN** | 未验证 |

## 2 · 实现

新增 `app-rs/src/bin/husk-session-guard.rs`，构建后作为
`jniLibs/arm64-v8a/libhuskguard.so` 打进 APK。它是 PIE 可执行文件，不是被 dlopen 的共享库；
使用 `.so` 后缀是为了让 Android 的 native library 提取路径把它解包出来。

启动顺序：

1. App 建 lease pipe，fork 看门狗，并等待 `ready-fd` 回包；
2. 看门狗 `setsid()`，与 App 和 guest 都脱离会话关系；
3. guest 会话首进程 `setsid()`，环境带唯一 `NR_SESSION_ID`；
4. App 把 `leader_pid + start_ticks` 写入 lease；
5. App 进程死亡或正常关停时 lease 关闭，看门狗扫描 `/proc/*/environ`；
6. 对带同一 marker 的进程先 SIGTERM，300 ms 后仍存在则 SIGKILL；同时对会话组做同样升级。

`NR_SESSION_ID` 已补进 `husk-session.sh`、桌面 DING/壁纸脚本里所有 `env -i` 重建环境的路径，
否则登录会话或 `env -i` 会把 marker 抹掉。

同时修正了 `teardown()` / `ensure_term()` 原先手工 `close(fd)` 后 `Term::drop()` 再关一次的
双关闭路径。现在统一走 `close_master()`，关闭前先把 fd 置 `-1`。

## 3 · 真机输出

App 启动：

```text
session: guard pid=27484 ready
spawned pid=27490 fd=103 tee=126 guard_pid=27484 guard_fd=108 armed=true
session: running pid=27490 (pgid=sid=pid by setsid) 80x24 fds=125
```

手工造一个脱离原会话组的 daemon，确认 marker 跟着环境走：

```text
PID=26529 state=S ppid=26074 pgid=26529 sid=26529
NR_SESSION_ID=husk-26074-1
PID=26530 state=S ppid=26529 pgid=26529 sid=26529
NR_SESSION_ID=husk-26074-1
```

独立 lease 关闭测试：

```text
daemon=27209
guard[27211]: started lease_fd=3 session=guard-direct-test
guard[27211]: cleanup marker_pids=[27209]
guard[27211]: sent signal=15 pid=27209
guard[27211]: done remaining=[]
```

`am crash` 路径：

```text
guard[26822]: lease EOF: App process is gone
```

该次 `am crash` 后同 UID 进程也为 0，但日志没有出现后续 `cleanup marker_pids` 行。
因此只能确认看门狗看到了 EOF；不能确认最终清理由看门狗还是 AMS 完成。这一点按 UNKNOWN 记录。

`am force-stop` 基线（未安装本轮看门狗的 `com.linuxhub.myth.husk`）：

```text
BEFORE 39
AFTER 0
```

当前 ROM 的 force-stop 会连同脱离会话的同 UID 子进程一起清掉，所以本轮没有复现此前记录里的
“force-stop 后仍残留 libld”。这不构成“看门狗没有价值”的结论：它覆盖的是 App 进程单独死亡、
崩溃、被 LMK 选中，而 AMS 尚未清扫整个 UID 的情况；那种跨 ROM 行为仍是 UNKNOWN。

## 4 · 尚未覆盖

- 未强制触发真实 LMK；没有拿 `am send-trim-memory` 冒充 LMK。
- 未验证所有 Android 版本的 `am force-stop`、AMS 和 SELinux 是否都按 UID 清扫。
- 进程如果主动清空环境且同时 `setsid()` 脱离进程组，marker 和 group 两条路径都可能漏掉；
  当前没有 cgroup/namespace 级兜底。
- 看门狗本身和 App 同 UID；如果某个 ROM 的 force-stop 连未被 AMS 登记的 helper 也一起杀，
  看门狗没有机会执行清理。当前 ROM 上两者都被清掉，未观察到残留。

