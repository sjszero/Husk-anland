# 服务监督器（container 式 init）

> 目标：guest 里的东西能"开机自启、挂了自动拉起、长久驻留"，并且停的时候不留僵尸。
> 前提：不 root、不解 BL、不开虚拟化、零 ptrace。设备 RMX3357 / 4.19.191 / `untrusted_app`。
> 实现：`runtime/crates/husk-init`（`husk-init`），构建 `tools/guest-config/build-init.sh`。

## 0. 先看前人（开工门禁）

| 仓库 | 读到的可复用点 | 不采用的理由 |
|---|---|---|
| `containers/youki`（A 级，见 `husk-notes/prior-art/container-and-page-size-prior-art.md`） | 生命周期分层：init 进程与 main 进程分家、子进程回收集中在 init 侧、"先 TERM、等、再 KILL"的停机顺序 | 它依赖真 PID namespace + cgroup；我们没有。**源码级深读仍未做**，本模块只借了分层与停机顺序，记在这里以免被当成已读完 |
| `ravindu644/Droidspaces-OSS`（A 级，只读需求） | 能力面清单：真 PID 1、容器开机自启、服务长期驻留、日志落地 | 需 root + 定制内核，实现不采用 |
| `containers/crun`（B 级） | 无 cgroup 时哪些回收责任必须落在运行时自己身上 | 同上，作用在特权侧 |

参考代码不进 Husk 仓库，笔记在 `husk-notes/prior-art/`。

## 1. 为什么不是 PID 1

PID 1 需要 PID namespace，而 PID namespace 需要 `CAP_SYS_ADMIN`/`unshare(CLONE_NEWPID)`，
本域拿不到。所以 `husk-init` **不是 PID 1**，它是：

* 一个普通进程（app 的子进程），
* `prctl(PR_SET_CHILD_SUBREAPER, 1)` 之后成为"子收割者"：它启动的服务死掉后，服务留下的
  孤儿（比如 `bash` 死掉后仍在跑的 `sleep`）会被重新挂到它下面，由它 `wait4` 回收。

这就是"零僵尸"的实际机制 —— 与 proot 不同（proot 没有子收割者，孤儿挂到 app 上由 app 收）。

## 2. 为什么是 freestanding 静态二进制

* 停机时容器正在被拆，监督器不能在那一刻依赖 loader / shim / glibc 的存活；
* 它按**宿主路径**工作（自己拼 `$NR_ROOTFS/...`），不需要路径改写；
* 没有 libc 就没有 `__write_nocancel` 之类的隐藏调用面，行为就是那几个 syscall。
* 代价：没有 `std`，`memcpy/memset/memcmp/bcmp` 自带，栈上没有分配器（全静态缓冲）。

`-z max-page-size=16384`，所以同一个二进制在 16K 页设备上也是合法映射。

## 3. 配置

`$NR_ROOTFS/etc/husk/services.conf`，一行一个服务：

```text
# name   policy      program and arguments
ticker   always      /usr/bin/bash /tmp/ticker.sh
once     once        /usr/bin/bash /tmp/quick.sh
failure  on-failure  /usr/bin/bash /tmp/boom.sh
```

* `policy`：`always` 无论如何都重启；`on-failure` 只在非 0 退出/被信号杀时重启；
  `once` 跑完就不管。
* 程序路径是**guest 绝对路径**，监督器自己加 `$NR_ROOTFS` 前缀。
* 参数按空白切分，**不支持引号**（要引号就把逻辑写进脚本）。
* 上限：8 个服务、每个最多 16 个参数、名字 39 字节。

## 4. 行为

* **启动**：全部立刻启动；子进程 `chdir($NR_ROOTFS)`，stdout/stderr 追加到
  `$NR_ROOTFS/var/log/husk/<name>.log`。
* **重启退避**：1s 起，连续失败翻倍到 30s；服务活过 10s 后退避复位为 1s。
* **回收**：每 200ms `wait4(-1, WNOHANG)` 一次，任何归到它名下的进程都被回收。
* **信号**：SIGTERM/SIGINT/SIGQUIT/SIGCHLD 被阻塞后走 `signalfd` + `ppoll`（无信号处理函数，
  没有 async-signal-safety 问题）。收到 TERM/INT/QUIT → 给所有服务发 SIGTERM，
  等它们退出（最多 5s，全退就提前结束），仍未退的 SIGKILL。
* **环境**：子进程继承监督器的环境，但 `PATH`/`HOME` 被替换成容器值
  （`/usr/local/sbin:...:/bin`、`/root`）——否则从 app 继承来的 Android PATH 会让
  guest 里的 `sleep` 都找不到（这是实测踩到的）。
* **信号屏蔽不继承**：子进程在 exec 前把信号掩码清空；否则会继承监督器为 signalfd 阻塞的
  TERM，于是"发 SIGTERM 却杀不掉服务"（也是实测踩到的，见 evidence 120）。

## 5. 已知限制（写清楚）

* 不是 PID 1：要求 `getpid()==1` 的程序（systemd、`--system` 模式的服务）不适用。
* 无依赖排序、无就绪探针、无 cgroup 资源限制。
* 日志不轮转。
* 没有开机自启的"官方挂点"：谁在 app 启动时拉起 `husk-init` 属于 2.0 集成的一部分，
  当前由测试/会话手动启动。

## 6. 验收

`tools/device-tests/12-supervisor.sh`（8 项，全通过）：服务真的在跑、`once` 不重启、
`on-failure` 重启、`kill -9` 后被拉起、SIGTERM 半秒内停、无残留进程、无僵尸。
原始输出见 `docs/design/evidence/120-supervisor-20261123.md`。
