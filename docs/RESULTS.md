# 实测结果

所有数字都来自设备原始输出，文件在 `docs/design/evidence/`，编号在每节末尾标出。
**FACT** = 命令输出直接可见；**INFERENCE** = 由输出推出；**UNKNOWN** = 没测。

**设备口径（重要）**：本文件混有两台机器的数据，读的时候先看标注。

| 设备 | 系统 | 出现在 |
|---|---|---|
| PLE110（OPPO） | Android 16 / kernel 6.6.118 / 4 KiB 页 | 第 1–5 节的性能与功能矩阵（历史测值） |
| RMX3357（MT6893 / Mali-G77） | Android 13 / kernel 4.19.191 / 4 KiB 页 | 第 6 节起、以及 evidence 31–36（当前目标机） |

Guest：Ubuntu 24.04 / glibc 2.39（RMX3357 上完整 rootfs 为 24.04.4 / glibc 2.39-0ubuntu8.7）
PLE110 被测 loader：`sha256 1600a850…1cbee`

---

## 1. 功能矩阵：31 / 31

`tools/device-tests/01-functional-matrix.sh`，最终 loader 上复跑（evidence 17）：

```
PASS mkdirp          PASS mkdirp-place    PASS deepwrite      PASS redirect
PASS cp              PASS mv              PASS chmod         PASS readlink
PASS symlink-follow  PASS symlink-abs     PASS symlink-dotdot PASS find-files
PASS find-links      PASS readdir         PASS ls-long       PASS pwd
PASS pwd-root        PASS dotdot          PASS dotdot-read   PASS escape-link
PASS escape-write    PASS rm-rf           PASS sort          PASS sha256
PASS glob            PASS stdin           PASS subshell      PASS exec-abs
PASS exec-rel        PASS bigio           PASS write-read
#checks=31 fail=0
```

覆盖：目录创建/写入/重定向、cp/mv/chmod、符号链接（相对/绝对/`..`）、find、readdir、
`ls -l`、cwd（含 `/`）、逃逸拦截、`rm -rf`、sort/sha256、glob、stdin、子 shell、
绝对与相对 exec、大 IO、读写回环。（evidence 09、17）

## 2. 越界：0 escapes

`03-containment.sh`——6 项攻击全部拦下，5 项边界行为正确（evidence 18）：

```
blocked read-abs-symlink     symlink -> /system/build.prop
blocked read-dotdot-symlink  symlink -> ../../../../../../system/build.prop
blocked list-hostdir-symlink symlink -> /data/local/tmp
blocked abs-up               cat /../../../../system/build.prop
blocked abs-up2              cat /../system/build.prop
blocked open-escape          exec 9</../../../system/build.prop
ok      clamp-cd-root        cd /..; pwd -P == /
ok      clamp-mkdir          mkdir -p /../esc  -> /esc *inside* the root
ok      rootfs-file          test -e /etc/os-release
ok      host-proc            head -1 /proc/self/status（挂载点按内核原样透传）
ok      host-dev-null        : > /dev/null
#escapes=0
```

## 3. 路径层成本（每 op，20,000 次平均）

| 项 | 数值 | 说明 |
|---|---|---|
| `stat` 真实路径 | **2,916 ns** | 路径本来就在 root 下，无解析 |
| `stat` guest 单组件 | **5,399 ns** | +2.5 us（解析 + 介入） |
| `stat` guest 四组件 | **11,070 ns** | 约 2 us / 组件 |
| `open` 真实路径 | 4,407 ns | — |
| `open` guest | 7,536 ns | +3.1 us |
| `getpid` | **196 ns** | 无路径，纯裸调用 |
| `fork+execve+wait` | **2,489,924 ns**（2.49 ms） | 每个子进程一次 loader 启动 |

对照（同进程、同 uid 的原生 bionic 二进制）：exec **6.8–8.0 ms**。
也就是说 guest 的 exec 比原生 bionic 还快——因为 bionic 那条路每次要过一层动态链接检查。
（evidence 10、13、14、18）

## 4. 与 PRoot 同条件对照

| 项 | Husk | PRoot |
|---|---|---|
| 机制 | 内核直接执行 | 每个 syscall 一次 ptrace 往返 |
| 5000 次 `stat` | **0.06 s** | **0.41–0.76 s** |
| 本机能否 exec guest ELF | 能 | **不能**（ENOENT→EACCES，四种配置、自带 loader、宿主 bionic 对照都试过） |
| dma-buf / DRM 映射 | 原生 | 极脆弱 |

PRoot 的 5000×stat 数字来自 RMX3357（Android 13）上的同一 harness；PLE110 上 PRoot 连
宿主 bionic 二进制都起不来，所以那张表在双机条件下分开列。（evidence 11、19；proot-comparison.md）

## 5. 真实构建：gcc 13.3 + make

`accept-gcc-make.sh`（exit 0）：

- gcc 驱动 → `cc1` → `as` → `collect2` → `ld.bfd` 全链通
- 编译 C 程序并运行，输出正确
- `make -j4` 并行构建；`make` 增量（`Nothing to be done`）；`make clean`
- 全程 `NR_SIGSYS=enosys` 下也通过（seccomp 兜底可用，但不作为主机制）

（evidence 16、17、20）

## 6. 构建工具自证

| 工具 | 语言 | 校验方式 | 结果 |
|---|---|---|---|
| `nr-tools scan-syscalls` | Rust | 迁移时与 Python 参考实现逐行 diff + 产物 sha256 | 一致（evidence 22） |
| `nr-tools embed` | Rust | 迁移时与 Python 参考实现产物 sha256 | 一致 `1600a850…1cbee`（evidence 22） |
| `nr-tools provision` | Rust | 与 sh+python 版阶段目录逐文件 sha256 | 2866 项，内容差异 0（evidence 23） |

在本仓库里直接跑 `tools/loader/build-embedded-loader.sh`，产出的 loader
就是被测的那个 sha256（2026-11-22 复验）。

## 6.5 rootfs 生成（2026-11-22 夜，主机侧）

`nr-tools rootfs` 从 noble 索引解析并解包 61 个包，产出 253 个 `usr/bin` 程序、usr-merge 链接、
61 条 dpkg 状态、`nop` 掉两处可选加速器调用的 libc，tar 25 MB。

**注意指纹随镜像走**：本次生成的 libc 是 `cc44c6a9…`、loader `fb632800…`，
与白天 PLE110 镜像（`18acad63…` / `1600a850…`）不同——它们是同一 suite 的不同补丁级。
loader 必须与镜像同源，所以 `--loader-out` 是从镜像自己的 `ld.so` 现做的。

## 7. 没测 / 不知道

- **未实测**：16K 页内核。机制与实测对齐数据见 [design/page-size.md](design/page-size.md)（Ubuntu arm64 用户态本身 64K 对齐，16K 内核不需要改造 guest）。
- **已补测**：10 分钟长稳（600 s / 568 tick / 11 360 次 fork-exec / 同 pid / 零僵尸，evidence 32）；更长时间仍 UNKNOWN。
- **UNKNOWN**：多 husk 并发实例。
- **未实测**：GPU 真正执行一个 job（atom 结构还没构造，见 evidence 36）。
- **FACT**：`execl` 家族拦不到（原因见 [ARCHITECTURE §6](ARCHITECTURE.md)）。

---

## 8. 生命周期（RMX3357，2026-11-22 21:43–21:47 与 22:05）

| 检查 | 命令 | 结果 |
|---|---|---|
| 启动 | `am start -n com.linuxhub.husk/android.app.NativeActivity` | `session: running pid=… 80x24 fds=98` |
| 打字不退出 | `input text "echo hi"; input keyevent 66` | `cb: onDestroy` 计数 0 |
| 切后台 | `input keyevent 3` | `cb: onPause`，无 teardown，guest 仍在 |
| 回前台 | `am start …` | 同一个 pid，未重启 |
| `exit` | `input text exit` | `session: exited status=0`，loader 进程 0 |
| 点击重生 | `input tap` | 400 ms 限流后新会话 |
| 旋转 | `cmd window user-rotation lock 1/0` | `67x58 -> 142x26 -> 67x58`，同 pid，guest `stty size` = `58 67` |
| 强杀 | `am force-stop` | 无残留 loader |

对照与不变量：`docs/design/lifecycle.md`（15 条，逐条给代码位置与验证方法）。

## 9. 2.0 完整 rootfs（RMX3357，evidence 33）

| 项 | 值 |
|---|---|
| 来源 | 只用 2.0 的 Ubuntu 24.04 链（离线 deb 894 个，去重后铺 rootfs） |
| 规模 | `usr/bin` 817 个，1.9 GB |
| libc | `2.39-0ubuntu8.7`，扫出 2 处 `set_robust_list` 并打补丁 |
| 实起 | `gjs 1.80.2`、`dbus-daemon 1.14.10`、`Xwayland 23.2.6`、`Python 3.12.3`、**GNOME Shell 46.0** |
| 内核级兼容 | `#!` 脚本、`/proc/self/exe` 返回 guest 路径（evidence 34） |

## 10. GPU 直连（RMX3357，evidence 35 / 36）

| 项 | 结果 |
|---|---|
| `/dev/mali0` | guest rw 打开成功；kbase UAPI 11.38 |
| 命令面 | 26 条，来自厂商 `libGLES_mali.so` 反汇编（不是猜 header） |
| `JOB_SUBMIT 0x40108002` | `nr_atoms=0` → EINVAL(22)：命令可达，参数被拒 |
| `mem_alloc(flags=0xf)` | 返回 0，回填 GPU VA `0x41000` |
| `mmap(prot=3, off=0x41000)` | 成功；CPU 写 `0xA5` 读回一致 |
| 反例 | prot 与 `flags&3` 不符 → EPERM；写 PROT_READ 映射 → SIGSEGV |
| 上游 Mesa | 25.2 与 main 无 kbase 后端；rootfs 无 panvk ICD |
