# evidence 163 — 改了 SIGSYS 兜底之后重跑门禁 A：36/36 PASS；顺带查清「这台机到底 trap 什么」

设备：RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB。
环境：**APK 域**（`com.linuxhub.myth.husk`，`run-as`，`untrusted_app`），rootfs
`files/linux/husk-clean/rootfs`（811 个 dpkg 包），loader 换成当前 main 的构建
`sha256 e838c8a2…`（旧的 `d5dd7e63…` 备份为 `libld.so.before-sigsys`）。
时间：2026-11-24 19:39（CST）。

## 1. 门禁 A（`tools/device-tests/14-gate-a-acceptance.sh`）

```
PASS=36 FAIL=0
```

覆盖：身份（guest uid=0、根是 rootfs）、bash 5.2.21（管道/heredoc/子 shell/进程替换/花括号
展开）、locale（zh_CN UTF-8、星期一、C.UTF-8 排序）、用户管理（groupadd/useradd/chpasswd/
id/home/shadow/group 清理）、dpkg（`-C` 干净、811 包、unpacked 0、`dpkg-deb -I`、
force-overwrite 重装、`getfacl`）、apt（2.8.3、policy 认识 acl、`-s` rc=0）、宿主路径泄漏
（只有内部 `NR_*` 变量带 app 目录、`ps` 看不到别的 app、guest 无 `/data/user` 与 `/system`、
`readlink /proc/self/exe` 是宿主 loader）、seccomp 真实 trap 0 条。

**FACT**：SIGSYS 兜底表这次的改动没有破坏任何一条既有验收。

## 2. 顺带查清：这台机到底有没有 seccomp TRAP

evidence 159 留的 UNKNOWN 是「新表在 `untrusted_app` 下没验过」。于是直接在 APK 域用
raw `svc #0` 扫了 48 个候选（chroot/mount/umount2/pivot_root/add_key/perf_event_open/bpf，
以及 425–462 的整段新系统调用），两种模式各跑一遍：

| 模式 | 结果 |
|---|---|
| `NR_SIGSYS=log`（保留内核致命语义） | 48 个调用全部返回、进程存活，**日志 0 行** |
| 默认（按表回答） | 同上，返回 `-38` 的那些是 4.19 内核本来就没有这些调用 |

结论（FACT）：**这台设备 + 这个 APK 上，这 48 个候选一个都没被 TRAP**。之前
evidence/14 与 2.0 时代记录的「`nr-sigsys.log` 刷屏（landlock）」在当前环境**复现不出来**；
可能当时是另一个 targetSdk/另一条进程上下文，本轮没有证据，标 UNKNOWN。

因此这张兜底表的定位要写准：**它是保险，不是这台机上正在生效的路径**。代价为零（只有 trap
发生时才进处理器），但只要换一台策略真的 TRAP 的 ROM，它就是「静态 ELF 与裸 `svc` 不会
被直接打死」的那道防线。要让它从「未验证」变成「已验证」，需要一个真的会 TRAP 的目标环境。

## 3. 运行时空载 30 分钟（新 loader）—— 已完成

用当前 loader 在 APK 域跑 `bench/soak.sh husk 1800`：每 60 s 一个采样点，共 30 个。

| 指标 | 首（t=0） | 末（t=1800） | 最小 | 最大 | 结论 |
|---|---|---|---|---|---|
| RSS (KiB) | 836 | 1 252 | 836 | 1 252 | 60 s 内升到 1 252，之后 30 分钟**完全恒定**（首点低是 loader 还没把 guest 映射完） |
| 线程 | 1 | 1 | 1 | 1 | 恒定 |
| fd | 4 | 4 | 4 | 4 | 恒定 |
| 僵尸 | 0 | 0 | 0 | 0 | 30 分钟零僵尸 |

原始采样：`docs/design/evidence/soak-husk-runtime-20261124.csv`（30 个采样点 + 首尾注释行）。

**这一条不是「桌面长稳」。** 桌面那 30 分钟是 evidence 153（rev 21、GNOME 46 X11 会话、
47 线程恒定、guest 僵尸 0）。本节的结论只到这一步：**换上新 loader 之后，运行时空载不涨
RSS、不涨 fd、不留僵尸**。桌面那一项要在 app 侧会话里重跑，本轮没做，不拿这个顶替。
