# 146 · 删掉 proot 之后，B/D/E 三段的回归套件在 2.0 的树上跑一遍

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`（debug），
rootfs = `files/linux/distributions/ubuntu-24.04/rootfs`（App 自己装的 814 包树）。

## 0 · 结论（FACT）

删 proot 之后，把"已收口"的那几段（挂载 / 设备节点 / 网络表面 / 进程视图 / 功能矩阵 / 围栏）
在**生产树**上重跑，抓到两个真问题，都修了，修完全绿：

| 套件 | 修之前 | 修之后 |
|---|---|---|
| `01-functional-matrix` | 31 checks / 0 fail | 31 / 0 |
| `03-containment` | escapes=0 | escapes=0 |
| `04-device-probe` | PTY / DRM shim / 设备节点如设计 | 同 |
| `05-mount-audit` | 18 PASS / 0 FAIL | 18 / 0 |
| `09-network-surface` | 11 PASS / 0 FAIL | 11 / 0 |
| `10-device-nodes` | **8 PASS / 1 FAIL**（`/dev/shm`） | 9 / 0 |
| `11-process-view` | **5 PASS / 3 FAIL**（`comm`） | 8 / 0 |
| `13-egress` | 4 PASS / 2 FAIL（树里没有 `curl`） | 见 §4 |

修完的全流程重跑（空 checkpoint，App 界面全程点击）：

```text
START EXTRACT_SYSTEM 1795488190849 → INITIALIZATION COMPLETE 1795488743445   552.6 s
  EXTRACT 3.6 / CONTAINER 523.9 / CREATE_USER 1.0 / SUDO 3.1 / ENV 16.3 / SAVE 4.6
814 包 install ok installed / dpkg -C 空 / Bad system call 0
1,181×nr=264 + 12×nr=452 全接住 / 10,208 个 guest 进程
/etc/passwd 0:0  /usr/bin/sudo 0:0  /home/husk 1000:1000
14-gate-a 34 PASS 0 FAIL    12-supervisor 8 PASS 0 FAIL
```

## 1 · `/dev/shm` 跟着 proot 的 bind 一起没了

`10-device-nodes` 第一项就红：

```text
FAIL  /dev/shm is a directory
```

原因：`/dev/shm` 在 Android 上不存在，shim 有意把它映射到 `<rootfs>/dev/shm`（不是透传），
以前 proot 用 `-b <app>/tmp/shm:/dev/shm` 提供了那个目录；删 bind 之后没有人建它，于是
guest 里每个 `shm_open()` 都是 `ENOENT`（PulseAudio、GTK 的共享内存都会走到这里）。

修法：`HuskRuntime.guestDirs` 加 `dev/shm`（App 每次准备会话时 `mkdirs`，实测 App 启动后
`rootfs/dev/shm` 就存在，`assets.version = husk-runtime-spawn-link-13`）。

## 2 · 嵌套 exec 的 `comm` 一直是 loader 的名字

`11-process-view` 的三条红：

```text
FAIL  /proc/self/comm names the program (got 'libld.so')
FAIL  ps lists this shell as bash
FAIL  pgrep -x bash matches
```

（顺带发现这条检查自己是**空转**的：`ps -eo comm | grep -qx libld39.so` 硬编码了旧 loader 名
`libld39.so`，现在的 loader 叫 `libld.so`，所以它永远是 PASS。已改成匹配 `libld*.so`。）

两个原因叠在一起：

1. `name_once()` 只在**第一次走 guest 路径**时被调用（`rootfd()`）。只碰透传路径的程序
   （`cat /proc/self/comm`、`ps`、`sleep`）根本不请求 root fd，于是永远保持内核给的名字。
   → 现在在 `nr_boot` 里调用（每个进程一次，在程序跑起来之前）。顺带确认：shim 是塞进
   ld.so 的一段，**没有自己的 `.init_array` 会被 loader 执行**，所以放在 `nr_init()` 里的
   那份永远不会跑（第一版就是这么写错的）。
2. `name_once()` 用 2 KiB 栈缓冲读 `/proc/self/cmdline`。嵌套 exec 的 cmdline 里
   `--library-path` 一个参数就几 KB，读满 2 KiB 就截断，解析永远走不到程序名。
   → 改成 64 KiB 静态缓冲（`.bss`），解析逻辑不动。

修完实测（App 树，`sh xe.sh usr/bin/bash …`）：

```text
$ cat /proc/self/comm            → cat
$ ps -eo pid,comm | head         → sh / bash / ps / head        （不再出现 libld.so）
$ bash -c '/usr/bin/bash /root/dt/probe2.sh'   → comm=[bash]
旧 loader（07c890f2）同一条命令             → comm=[libld-07c890f2.]
```

代价：loader 从 673,280 B 涨到 738,816 B（+64 KiB 的 `.bss` 缓冲）。

## 3 · 复现命令

```sh
# 套件放在 rootfs 里（01 会 rm -rf /tmp，别放 /tmp）
run-as com.linuxhub.myth.husk sh -c 'cd files/linux/distributions/ubuntu-24.04/rootfs && mkdir -p root/dt'
for f in 01 03 04 05 09 10 11 13; do adb push tools/device-tests/$f-*.sh /data/local/tmp/; done

# 跑（宿主侧）
X='sh -c "cd files/linux && sh xe.sh usr/bin/bash /root/dt/10-device-nodes.sh"'
run-as com.linuxhub.myth.husk $X
```

## 4 · 没闭环 / 不算进红

- `13-egress` 的两条 `curl` 检查在 2.0 树里是**没有 curl**（只有 `wget`），不是运行时问题：
  DNS 与裸 socket `HTTP/1.0 GET` 都 PASS，`wget -q -O /dev/null http://ports.ubuntu.com/`
  → `rc=0`。要保留这两条检查就得先给 2.0 的包清单加 curl，或在脚本里认 `wget`。
- `02-bench` 没跑（本轮不测性能）。
- 桌面首帧（G）仍是 UNKNOWN，与脱机无关的那部分继续按目标延后。
- 这些套件用 `run-as` 跑（`runas_app` 域），历史基线也是这么跑的；需要 App 域的结论
  （例如 `/sdcard`、门禁 A）另外在 App 自带终端里量（evidence 145）。
