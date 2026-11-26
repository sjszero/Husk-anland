# 267 · 会话 guard 清理结果真实性与启动 fd 重映射

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。对象：`app-rs` 的 Rust 会话 guard、App 侧 guard 启动和
`NR_SESSION_ID` marker 清理。

## 0 · 结论

FACT：旧 `marker_pids()` 在 `read_dir("/proc")` 失败时直接返回空 `Vec`。`terminate()`
随后把空列表写成 `cleanup marker_pids=[]`，最终再写 `done remaining=[]`，进程退出码仍是
0。也就是说，扫描失败会被伪装成“确认没有残留进程”。

FACT：新实现把 `/proc` 扫描失败变成 `io::Result` 错误。三个阶段分别记录
`marker scan failed errno=...`；最终无法确认时写
`done remaining=UNKNOWN ... clean=false`，并以退出码 70 结束。只有扫描成功、列表为空、
没有跳过目录项且会话组已经消失时，才写 `clean=true`。

FACT：guard 启动前的 fd 3/4 重映射还有两个工程缺陷：

1. 旧顺序在两个管道端恰好是 `lease[0] == 4`、`ready[1] == 3` 时会先覆盖 ready 写端，
   guard 随后拿到错误的 fd 4，ready 握手永远失败。
2. App 等待 guard ready 超时后只关闭 lease，没有杀掉可能卡住的 helper。

新实现先用 `F_DUPFD_CLOEXEC` 把两个源 fd 复制到 10 以上，再关闭原 fd，最后映射到
3/4；ready 超时时对 guard 发送 SIGKILL，再关闭 lease。真机 guest 内验证
`F_DUPFD_CLOEXEC 100 1`，说明该 fcntl 命令在当前 `untrusted_app` 域可用。

## 1 · 扫描失败的真机验证

新增 `tools/device-tests/run-session-guard-probe.sh`，并接入 `make gate-a`。脚本使用
guard 的测试参数 `--proc-root <dir>` 和 `--test-lease-eof`，不依赖 App 安装，也不修改
LinuxHub 2.0 数据。

正常 marker 清理：

```text
GUARD_MARKER=gone
GUARD_VALID_RC=0
cleanup marker_pids=[22592] skipped_entries=0
sent signal=15 pid=22592
post-term marker_pids=[] skipped_entries=0
final marker_pids=[] skipped_entries=0
done remaining=[] skipped_entries=0 group_alive=false clean=true
```

不存在的 proc 根目录：

```text
GUARD_MISSING_RC=70
cleanup marker scan failed errno=2
post-term marker scan failed errno=2
final marker scan failed errno=2
done remaining=UNKNOWN group_alive=false clean=false
```

FACT：错误路径不再出现 `remaining=[]`，也不会返回成功。旧实现的静态路径是
`read_dir(...).ok() -> 空 Vec -> remaining=[] -> exit 0`；这一条不是动态压力复现，
而是代码构造确定的错误处理缺陷。

## 2 · fd 重映射与超时清理

旧代码：

```text
if lease[0] != 3: dup2(lease[0], 3)
if ready[1] != 4: dup2(ready[1], 4)
```

当 `lease[0] == 4` 且 `ready[1] == 3` 时，第一条 `dup2` 已经覆盖了 ready 写端。
这一布局在 fd 使用较少或测试进程中可能出现，不能让启动结果依赖 fd 分配顺序。

新代码先复制两个源 fd，再关闭所有原 fd，最后映射：

```text
lease_copy = fcntl(lease[0], F_DUPFD_CLOEXEC, 10)
ready_copy = fcntl(ready[1], F_DUPFD_CLOEXEC, 10)
close(lease[0]); close(ready[0]); close(ready[1])
dup2(lease_copy, 3)
dup2(ready_copy, 4)
close(lease_copy); close(ready_copy)
```

FACT：提交前复核又发现一个顺序错误：若先 `dup2` 到 3/4 再关闭原 fd，而原 fd 本身占着
3 或 4，就会把刚映射好的目标关掉。最终顺序改为“复制 → 关闭原 fd → dup2 → 关闭副本”，
避免源 fd 与目标编号重合时破坏目标。

FACT：真机 guest 内 `F_DUPFD_CLOEXEC` 调用返回 `fd=100`、`FD_CLOEXEC=1`。这验证了
当前 App 域允许该命令，不能把它写成只经过宿主机静态编译。

FACT：探针新增 `ready_fd=3`、`lease_fd=4` 的反向编号布局，guard 返回
`GUARD_FD34_READY=R`，缺失 proc 时 `GUARD_FD34_RC=70`。这验证 guard 能处理目标编号
已被占用的布局；但真实 App 的 `spawn_guard()` 子进程路径仍未在设备上动态注入该布局，
仍是 UNVERIFIED，不是动态复现。

## 3 · 完整回归

```text
make check: PASS
make docs: PASS
session-guard-probe: PASS
gate-a: PASS=38 FAIL=0
IDENTITY_RACE rounds=300 fails=0
GROUPS_RACE rounds=20 fails=0
ROOTFD_RACE stat_ok=379 stat_fail=0 range_ok=500 range_fail=0
path race: PASS host sentinel unchanged
coverage-probe: PASS
coverage-sweep: SKIP (rootfs 没有 strace，不能算通过)
```

边界：本修复只改 Husk App 的会话 guard 和启动链，不是桌面补丁，不改 LinuxHub 2.0。
UNKNOWN：其他 Android ROM 对 `/proc/<pid>/environ` 的同 UID 可见性；若某 ROM 连扫描
都拒绝，当前实现会明确失败并退回组信号，但不能声称 marker daemon 已被清理。

## 4 · 最终 APK 构建

FACT：在最后一次 `husk-session-guard`、`app-rs/src/sys.rs` 和 `app-rs/src/term.rs` 修改后，
用 NDK `29.0.14206865` 重新执行 `make apk`，Gradle `BUILD SUCCESSFUL`。最终产物哈希：

```text
2c7a54ab9e20a3293b465feac44e0e5b591ac172a915e33eb99a10b1a536e397  app-debug.apk
b38e3b5ce68f404ed9552631f7dead1b4a33de9a3aec2bfa0173ea05c1454009  libld.so
240a35af5eb94c9ea03faafecaa075edb08691ac35a974733125577aa2427d47  libhuskapp.so
cc211480268d69627fb255d73ad2c313e0d8c8bc8576d296c11f2cffead5569b  libhuskguard.so
```
