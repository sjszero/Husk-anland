# 336：活跃门禁中的 Python 编排改为 Rust

日期：2026-11-27（CST）
基线：`35ab14a`（运行树仍为设备上的 `files/rootfs-full`）
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页

## 1. 修前问题（FACT）

仓库中受 Git 跟踪的 `.c` / `.py` / `.js` 文件已经是 0，但活跃门禁仍在使用
Python 作为编排语言：

```text
tools/probe/coverage-sweep.sh                 python3 解析 strace
tools/device-tests/audit-supplementary-groups.sh
                                              python3 heredoc 调 libc
tools/bench/run-all.sh                        python3 计时和统计
tools/bench/run-interleaved.sh                python3 计时和统计
```

这会导致开发机或最小 guest 没有 Python 时门禁无法独立复现。它不说明核心运行时
依赖 Python；核心运行时仍然由 Rust、loader 和 glibc 组成。

同时确认了一个真实的 rootfs 选择问题：`coverage-sweep.sh` 和
`run-coverage-probe.sh` 仍默认指向旧的
`files/linux/distributions/ubuntu-24.04/rootfs`，而当前运行时使用
`files/rootfs-full`。如果传入相对 rootfs，`coverage-sweep` 还会把路径过滤器
比较成相对串，导致已翻译的路径被误列为候选。

## 2. 改动（FACT）

- `husk-tools` 新增 `strace-path-candidates <out-dir> <rootfs>`：解析 `cov-*.log`，
  忽略 `execve` 的 argv 数组，收集未带 rootfs 前缀的绝对路径；排序去重。
- `husk-tools` 新增 `now-ms`，以及覆盖 `## cold_start_ms` / `#rep` 两段格式的
  `bench-summarize` 解析。
- `runtime-probe` 新增 `supplementary-groups`：通过 libc 调
  `getgroups` / `setgroups` / `initgroups` / `access`，输出真实组列表，
  失败时返回非零。
- `coverage-sweep.sh`、`run-all.sh`、`run-interleaved.sh` 改用 Rust 工具。
- `audit-supplementary-groups.sh` 由 Python heredoc 改为执行 Rust guest probe；
  新增 `run-supplementary-groups-probe.sh`，负责把 probe 放进临时 guest 路径。
- `coverage-sweep.sh` / `run-coverage-probe.sh` 默认 rootfs 统一为
  `rootfs-full`，并将相对设备路径规范化为 `/data/data/<pkg>/files/...`。
- `make gate-a` 改走新的 supplementary-group 包装器；不改变 38 项既有门禁语义。

## 3. 修后正向结果（FACT）

```text
cargo test -p husk-tools
9/9 PASS（含 strace 路径解析和 cold_start 摘要）

NDK=<ndk>/bin make check
PASS（整 workspace 检查、shell 语法、release/APK/资产审计、文档链接）

make gate-a（临时匹配 loader）
PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: PASS（1089 条候选，无未翻译路径）
fd-identity-probe: PASS
supplementary-groups: PASS=1 FAIL=0
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
rootfd-close-range: PASS
path-race: PASS
```

第一次 `make gate-a` 并不绿：结果为 `PASS=34 FAIL=2`。其中
`apt-get -s` 因当前设备树缺少 `libpam-modules-bin` 和 `libsmartcols1` 返回
`rc=100`，`/data/user` 则是此前旧会话留下的 Mesa shader cache。为避免把旧状态
当成运行时通过：

- 只在这台测试设备的 `files/rootfs-full` 中安装了这两个已由 apt 索引确认的包；
- 只删除了该 rootfs 下旧的 `data/user/0/com.linuxhub.husk/.cache/mesa_shader_cache`；
- 然后重新跑门禁，得到上面的 `38/0`；这不是源码仓库已自动修复这两个环境状态。

supplementary-group 的真机关键输出：

```text
AFTER_START [1500, 4242]
UID_GID 1500 1500
ID_GROUPS 1500 4242
AFTER_SETGROUPS [4242]
ACCESS_GROUP_R true
ACCESS_GROUP_W true
AFTER_INITGROUPS [1500, 4242]
PASS supplementary-groups setgroups/initgroups/getgroups/access
```

`gate-a` 中使用的是现场构建的临时 loader，而不是设备上旧安装槽位的
`files/linux/husk/libld.so`；临时 loader 的 SHA-256 为
`67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`。
测试结束后已移除临时 loader，不把测试路径当作发行资产。

## 4. 复跑时发现的嵌套路径问题（FACT）

修完 H18 后第一次完整复跑 `gate-a`，前面的验收项和 race 探针都通过，但
`run-path-race-probe.sh` 在最后返回失败：

```text
/data/local/tmp/husk-session.sh[259]: files/rootfs-full/root/libld-test.so: inaccessible or not found
--- path-race-probe: execution failed
make: *** [gate-a] Error 1
```

原因是门禁脚本的 `device_path()` 只认识裸的 `files/rootfs-full`，不认识
`files/rootfs-full/root/libld-test.so` 这类 rootfs 子路径。补充 10 个脚本的映射后，
同一命令 `run-path-race-probe.sh` 通过：`STATS toggles=60000 operations=212050`、
`PASS host-sentinel-not-visible`。随后再次完整执行 `make gate-a`，全部通过；临时
loader 和 `/data/local/tmp` 副本均已删除。

## 5. 修前负向与仍未消除的边界（FACT / UNVERIFIED）

- `make check` 在不设置 `NDK` 时会失败：
  `audit-xlorie-asset.sh: readelf/llvm-readelf not found`。设置正确的 NDK
  `.../bin` 后通过；这是本机环境变量问题，不是本次 Rust 改动。
- `rt-server-parity.sh`、`05-mount-audit.sh`、`13-egress.sh` 等诊断脚本仍有
  Python heredoc；本轮只清理活跃门禁和 benchmark 编排，不宣布全仓库无 Python。
- benchmark 的 `guest/measure.sh` 仍故意调用 `python3` 作为被测负载；那不是编排
  依赖，不能删除。
- 设备上原有安装树的 loader 与当前源码构建的 loader 不一致；本轮用临时路径完成
  回归，没有覆盖安装槽位，也没有 bump `VERSION`、打 tag 或发 Release。
- 桌面 acceptance 仍受设备上 `u0_a300` 的高进程/线程数限制，未在本轮重新执行。
