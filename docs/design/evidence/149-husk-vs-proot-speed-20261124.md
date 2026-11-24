# 149 · Husk 原生 vs proot 容器：同一 rootfs、同一 App 沙箱的速度差（FACT）

日期 2026-11-24 · RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4KiB）
包 `com.linuxhub.myth.husk`，两边都是 `untrusted_app`。

## 0 · 方法（同一台机、同一 rootfs、同一会话）

- rootfs：`files/linux/distributions/ubuntu-24.04/rootfs`（Ubuntu 24.04.4，GNOME 46）
- **Husk 侧**：`files/linux/xe.sh <cmd>`（`libld.so` + path shim，无 ptrace）
- **proot 侧**：`files/linux/pr.sh <cmd>`，参数照抄 2.0 删掉 proot 之前
  `prootCommandLegacy()` 的原样（`--link2symlink -0 -r <rootfs> -b /dev -b /proc -b /sys`，
  `PROOT_LOADER`/`PROOT_TMP_DIR`/`LD_LIBRARY_PATH` 全带上），
  二进制就是 2.0 最后一次发版里那份（proot `03ec7569…`、loader `9a1c8aae…`，见 §4）
- 两边都跑**同一个脚本**，只换启动器；GNOME 会话本身由 Husk 托管（这样唯一变量是
  被测量进程跑在哪个运行时里）

## 1 · "打开设置"：从 exec 到窗口真的可见（FACT）

测法：`bench_settings.py` 先连 X server（`:0`）数一遍 root 的可见顶层窗口当基线，
`Popen(gnome-control-center)` 起表，之后每 20 ms 用 `QueryTree` + `GetWindowAttributes`
找"新出现的 viewable 顶层窗口"，命中即停（这就是用户看到窗口的那一刻）。

| 运行 | 第 1 次 | 第 2 次 | 第 3 次 |
|---|---|---|---|
| Husk 原生 | 1.580 s | 1.026 s | 0.999 s |
| proot | 1.697 s | 1.610 s | 1.627 s |

差值 **≈ +0.6 s（1.6×）**。proot 三次都落在 1.61–1.70 s，抖动很小；Husk 第一次
1.58 s 是 GTK/icon/font 缓存还冷，之后稳定在 1.0 s 左右。

## 2 · 微基准（同一脚本，只换运行器）（FACT）

| 项目 | Husk | proot | 比值 |
|---|---|---|---|
| `fork+exec` × 40（`/bin/true`） | 131 ms | 227 ms | 1.73× |
| `find /usr -xdev -type f \| wc -l` | 1 808 ms | 3 171 ms | 1.75× |
| `tar -cf - /usr/share/doc \| wc -c` | 1 127 ms | 2 316 ms | 2.06× |
| `ls -laR /usr/lib` | 2 384 ms | 3 928 ms | 1.65× |
| `dpkg -l` | 46 ms | 51 ms | 1.11× |
| `python3 -c 'sum(range(4e6))'`（含解释器启动） | 127 ms | 152 ms | 1.20× |
| `sha256sum` 139 MB（`libLLVM.so.20.1`） | 229 ms | 未测到（§3） | — |
| `fork+exec` × 200 | 714 ms | 未测到（§3） | — |

读法：
- **每系统调用都被 ptrace 停两次的代价，落在 syscall 密集的地方**：
  exec 1.7×、目录遍历 1.65–1.75×、大量小文件读 2×。
- **顺序 I/O 与大块内存计算几乎不吃亏**：`dpkg -l` 1.1×、纯 CPU 1.2×（差异主要是
  python 启动的 syscall）。
- proot 的开销与"进程做了多少系统调用"成正比，不随"算了多少 CPU"增长——
  这就是为什么"打开设置"只慢 0.6 s，而"容器里装软件/dpkg 配置"会成倍变慢。

## 3 · 没量到的（UNKNOWN，不猜）

- `sha256sum` 139 MB 与 `fork+exec ×200` 两项在 proot 下**没拿到结果**：今天两次
  运行都出现"被跟踪的子进程已经全退出了，`libproot.so` 这个 tracer 还挂在那里、
  stdout 不关"，于是管道永远读不到 EOF。原因 UNKNOWN（现象记录在此，未排查）。
- **整段 proot 会话**（gnome-shell/mutter/dbus 都跑在 proot 里）的开机时间没测：
  今天量的是"同一个 Husk 会话里，只把被启动的那个进程换成 proot"。
  真按容器版 2.0 的形态，开机路径上几十万个 syscall 都吃 ptrace，
  按上表的比例外推会明显更慢——这是 INFERENCE，不是测量值。

## 4 · 指纹与复现

```
proot   03ec75690868832ad6317877d3da0f0238c283cdc50aad5911cf54e5315405b6  217,040 B
loader  9a1c8aae223f5e8de76b8d457086530ba2780261d3582cc2e1bbe4ccf19df5c9   17,800 B
```

```sh
# 桌面已由 Husk 起在 :0
adb shell run-as com.linuxhub.myth.husk sh files/linux/xe.sh \
  bin/sh /root/dt/bench_micro.sh                      # Husk 侧微基准
adb shell run-as com.linuxhub.myth.husk sh files/linux/pr.sh \
  /bin/sh /root/dt/bench_micro.sh                     # proot 侧微基准（同脚本）
adb shell run-as com.linuxhub.myth.husk sh files/linux/xe.sh \
  bin/sh -c 'python3 /root/dt/bench_settings.py'       # Husk 侧开设置
adb shell run-as com.linuxhub.myth.husk sh files/linux/pr.sh \
  /usr/bin/python3 /root/dt/bench_settings.py          # proot 侧开设置
```

脚本在 rootfs 的 `/root/dt/` 下（`bench_micro.sh`、`bench_micro2.sh`、`bench_micro3.sh`、
`bench_settings.py`）；`bench_settings.py` 自带 `DISPLAY/XDG_CURRENT_DESKTOP` 等
环境（少了 `XDG_CURRENT_DESKTOP=ubuntu:GNOME`，`gnome-control-center` 会直接
以 `Running gnome-control-center is only supported under GNOME and Unity` 退出）。

## 5 · 顺带确认的两件事（FACT）

- 手机上同时装着两个 2.0：`com.linuxhub.myth`（2026-11-22 09:57 首次安装、
  10:32 更新，构建时间早于 proot 移出构建的提交 `24851531`／2026-11-24 10:11，
  即 **proot 版**）与 `com.linuxhub.myth.husk`（2026-11-24 00:37，**Husk 版**）；
  另有旧的 `com.linuxhub.probe`、`com.linuxhub.husk` 两个测试包。
- 本次对比**没有读写 `com.linuxhub.myth`**：proot 侧是用 2.0 最后一次发版里的
  proot 二进制，在**我们自己的** App 沙箱里、对**同一个** rootfs 跑的。
