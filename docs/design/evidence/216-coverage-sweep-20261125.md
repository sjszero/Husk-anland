# 216 · 覆盖率普查：用 strace 抓真实负载，找出"本该翻译却没翻译"的路径

日期：2026-11-25 22:20–22:45（CST）。设备 RMX3357，`com.linuxhub.myth.husk`。

214 的探针是**点测**：验"我怀疑漏了的那几个符号"。这份是**普查**：让真实负载跑一遍，
把内核实际收到的每一个路径参数都抓下来，再判断它是不是 guest 里的路径却没有带 rootfs 前缀。

## 1 · 前置：strace 装进 rootfs（FACT）

guest 里没有网（`getent hosts archive.ubuntu.com` 无输出），所以不能 `apt-get install`。
在宿主上取包再推进去：

```sh
curl -s -o strace.deb \
  http://ports.ubuntu.com/ubuntu-ports/pool/main/s/strace/strace_6.8-0ubuntu2_arm64.deb
adb push strace.deb /data/local/tmp/strace.deb
# run-as 复制进 rootfs/root/ 后：
husk-session.sh <rootfs> -- /usr/bin/dpkg -i /root/strace.deb
```

装完 `strace -V` → `strace -- version 6.8`。**guest 里 strace 能用**本身就是个旁证：ptrace
没有被我们占用（PRoot 那边做不到，见 evidence 213 的机制表）。

## 2 · 判据（FACT）

两个独立事实交叉才成立：

1. strace 记下的路径**没有** rootfs 前缀（说明它原样进了内核）；
2. 这个路径在 guest rootfs 里**确实存在**（`[ -e <rootfs>$p ]`，用 run-as 在宿主侧判，
   不经过 shim）。

两条同时成立 = 这个调用本该被翻译却没有。

排除项：execve 的 argv 数组里也会出现路径（那是程序自己的参数，不是路径实参），
所以先按 `[` 把数组切掉再取引号里的内容——第一版没切，把 `/usr/bin/python3`、`/bin/ls`
这类 argv 误报成漏网（9 条假阳性），切掉之后归零。

## 3 · 结果（FACT）

`make gate-a` 现在三段，第三段就是这个普查：

```
PASS=36 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: PASS  (没有发现未翻译的 guest 绝对路径)
```

四个负载、候选路径 1379 条：

| 负载 | trace 行数 |
|---|---:|
| `ls -l /usr/share/doc /usr/share/locale` | 38 407 |
| `find /etc -name '*.conf'` | 4 024 |
| `python3 -c "import json, ssl"` | 1 998 |
| `apt-get -s install acl` | 6 254 |

没有一条"guest 里存在、却没被翻译"的绝对路径。这条结论的适用范围就是这四个负载——
不是"所有程序都没问题"。

## 4 · 复跑

```sh
sh tools/probe/coverage-sweep.sh <device-serial>            # 默认四个负载
sh tools/probe/coverage-sweep.sh <device-serial> "/bin/ls -l /root"   # 指定负载
```

rootfs 里没有 strace 时脚本打 `SKIP` 并退出 0，不会把"没装工具"变成"检查通过"。
装法见 §1。

## 5 · 还没做

- 负载只有四条，且都是命令行工具。Nautilus / GNOME Shell / 浏览器这类 GUI 负载没跑
  （要起会话，成本高），它们的路径面比 CLI 宽得多。
- 判据只覆盖"%file 类系统调用"；io_uring 的 `IORING_OP_OPENAT`、`execveat` 这类
  不走普通路径参数的调用抓不到。
- 静态 ELF 与程序自己发的 raw syscall 仍然绕开翻译（设计取舍），普查也看不见它们。
