# evidence 119 — guest 进程在 `ps`/`top`/`pgrep` 里用真实名字

设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`。2026-11-23。
脚本：`tools/device-tests/11-process-view.sh`（8 项，全通过）。

## 1. 问题（FACT）

guest 进程的 `comm` 由内核在 exec 时取自被执行的**文件**，而容器里被执行的永远是 loader：

```
$ ps -e
 29502 ?  00:00:00 sh
 29505 ?  00:00:00 libld39.so
 29506 ?  00:00:00 head
 29509 ?  00:00:00 libld39.so
$ pgrep -x bash     -> 空
```

后果：`ps -e`/`top`/`htop` 的 COMMAND 一列大半是 `libld39.so`，`pgrep`/`pidof`/`pkill`
按名字找不到任何 guest 程序。这是可用度问题，不是正确性问题。

## 2. 改法

`rootfd()` 是每个 guest 进程第一次做路径操作时必走的初始化，那里加一次 `name_once()`：

* 读 `/proc/self/cmdline`（裸 syscall，不经自己的 open）；
* 跳过 argv[0]（loader）、跳过 `--library-path` 及其取值、跳过其他 `--` 选项；
* 第一个剩下的参数就是 guest 程序，取 basename（最多 15 字节）；
* `prctl(PR_SET_NAME)`。

进程自己 fork 出来的子进程继承 comm；exec 出去的新进程在新进程里各设一次。
静态原子量保证只做一次。

## 3. 实测（FACT）

```
$ ps -e
 30968 ?  00:00:00 bash
 30969 ?  00:00:00 tail
 30970 ?  00:00:00 ps
 30971 ?  00:00:00 head
$ pgrep -x bash        -> 30968
$ cat /proc/self/comm  -> cat
$ ps -eo comm | sort | uniq -c
      1 bash   1 head   1 ps   1 sh   1 sort   1 tail   1 uniq
$ top -b -n1 | sed -n 2p
Tasks:   6 total,   1 running,   5 sleeping,   0 stopped,   0 zombie
```

`top` 与 `ps` 的任务数一致；`/proc/1/comm`、`/proc/2/comm`（宿主进程）仍然读不到。

## 4. 回归

```
tools/device-tests/05-mount-audit.sh     PASS=18 FAIL=0
09-network-surface.sh                    PASS=11 FAIL=0
10-device-nodes.sh                       PASS=9  FAIL=0
11-process-view.sh                       PASS=8  FAIL=0
06/07/08 IPC 脚本                        RESULT PASS ×3
```

设备侧的清理脚本 `killall.sh` 原来按 comm 找 `libld39.so`，改名后失效 —— 已改成
`readlink /proc/<pid>/exe`（exe 始终是 loader，改名不影响它）。这一点写在这里，
免得下次又踩。
