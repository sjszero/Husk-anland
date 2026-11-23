# 用 Husk 替换 LinuxHub 2.0 的 proot 运行时

> 目的：2.0 现在每个会话都是一条 `libproot.so …` 命令行。换掉它要回答两个问题：
> ① 那串参数里哪些是 **proot 自己缺能力**（换掉就该删），哪些是 **真实需求**（必须留住）；
> ② 换过去以后，哪些东西靠的是 proot 独有的假象，Husk 必须自己提供。
> 本文只写我们自己的映射与实测，2.0 的代码不复制进仓库。

## 1. 2.0 现在怎么起会话（读代码得到的原样契约）

```kotlin
prootCommand(...) = libproot.so --link2symlink -0 -r <rootfs>
  -b /dev -b <shm>:/dev/shm -b /proc
  -b <compat>:/proc/version -b <compat>:/proc/stat -b <compat>:/proc/bus/pci/devices
  -b <audio>:/run/doro-audio -b <desktop>:/run/doro-desktop -b /sys
  -b <compat>:/sys/class/power_supply
  -b <x11>:/tmp/.X11-unix
  -w /root /usr/bin/env -i HOME=… USER=… PATH=… LANG=C.UTF-8 TERM=xterm-256color …
```
环境里另有 `PROOT_LOADER`、`PROOT_TMP_DIR`、`PROOT_TMP_DIR`，以及
`/etc/profile.d/doro.sh` 里的一行 `LD_PRELOAD=<libgetifaddrs_bridge.so>`；
登录会话再包一层 `su - <user>`，前面还有一段 `ensureAndroidGroupNamesScript`
（把 `/proc/self/status` 的 Groups 追加成 `/etc/group` 里的 `android_gid_N`）。

## 2. 逐项映射

| 2.0 的用法 | 它存在的理由 | Husk 的对应物 | 结论 |
|---|---|---|---|
| `-r <rootfs>` | 重定向 `/` | `NR_ROOTFS` | **换成环境变量** |
| `-0` | fake root | `NR_FAKE_ROOT`（默认开） | **换成环境变量** |
| `su - <user>` | proot 的 `-0` 让所有人都是 root，切到用户只能靠 `su` + fake setuid | `NR_FAKE_UID`/`NR_FAKE_GID` 种入身份，`setuid/setresuid/setreuid/getresuid` 跟着走（新增） | **删除 su 这一步**（见 §4 的 su 现状） |
| `--link2symlink` | proot 不支持 `link(2)` | shim 的 `link/linkat` 在 `EACCES/EINVAL` 时做 copy 回退 | **删除**（实测 `ln` 在 guest 里 rc=0） |
| `-b /dev` `-b /proc` `-b /sys` | 把宿主目录露进去 | `/dev` 按名字直开、`/proc` 与 `/sys` 合成/透传 | **删除** |
| `-b compat:/proc/version`、`/proc/stat` | proot 伪造不了这两个文件 | shim 合成（`Linux version 4.19.191…`、`cpu …`） | **删除** |
| `-b compat:/proc/bus/pci/devices` | 同上 | 未合成；guest 看到宿主结果（本机 ENOENT） | **删除**，记为墙（device-adapters.md） |
| `-b compat:/sys/class/power_supply` | proot 下 `/sys` 不可用 | `/sys` 透传宿主，值就是宿主的 | **删除** |
| `-b shm:/dev/shm` | proot 没有 `/dev/shm` | `/dev/shm` 支持（evidence 118） | **删除** |
| `-b audio:/run/doro-audio` | 和 2.0 的音频守护进程换 socket | Husk 的音频是 app 侧 AAudio + socketpair，**通道不同** | **保留目录约定**，但不再依赖 proot |
| `-b x11:/tmp/.X11-unix` | Termux:X11 / Xwayland 是外部载体 | 与 proot 无关 | **保留** |
| `PROOT_LOADER`、`PROOT_TMP_DIR` | proot 内部 | `NR_LD` | **删除** |
| `LD_PRELOAD=libgetifaddrs_bridge.so` | proot 下 `getifaddrs()` 拿不到接口 | netlink interpose + `/sys/class/net`（evidence 116/121） | **删除**（实测 `getent`/`getifaddrs` 不用它） |
| `ensureAndroidGroupNamesScript` | proot 的 `getgroups` 透传 Android gid，`groups`/`ls -l` 打一堆 "cannot find name for group ID" | `getgroups` 现在在 fake 身份下只报主组（新增） | **删除** |

## 3. 换的时候必须守住的两条契约（踩过）

1. **`NR_ROOTFS` / `NR_LD` 必须穿过每一次环境重置。**
   `env -i`、`su`、`login`、`sg` 都会重建环境，而 shim **只**通过 `NR_ROOTFS` 找 guest root。
   2.0 现在正好用 `env -i`。少了这两个变量，表现是"shim 失效"：父进程路径对、
   子进程读宿主路径（2026-11-23 实测，evidence 125 / 129）。
2. **程序路径必须相对。** loader 只接受 `usr/bin/bash`，不接受 `/usr/bin/bash`。
   经 `env` / `su` 起的进程用绝对路径没问题（那时是普通 exec），但**我们自己**调 loader 时要用相对路径。

## 4. 已知缺口：`su` 现在不能用（不是 proot 造成的）

| 事实 | 证据 |
|---|---|
| `su --help` 正常；`su -c …`（root→root、root→hu1）一律 `su: System error`，rc=1 | 实测 |
| `System error` 是 libpam 的 `PAM_SYSTEM_ERR` 文本（在 `libpam.so.0` 里找到该字符串） | 实测 |
| 换成只有 `pam_permit.so` 的最小 PAM 配置，仍然 `System error` → 不是 pam_unix / NSS 的问题 | 实测 |
| NSS（`getent passwd`、`python pwd`）、`fopen` 类工具（sed/sort/awk/perl）、`dlopen` guest 路径**都正常** | 实测 |
| `strace` 在设备上会把被测进程挂死（app 域 ptrace 受限）→ 没法用它定位 | 实测 |

结论：这是**运行时的缺口**，不是 2.0 的补丁。定位还需要一次能进 PAM 内部的观测手段
（或改成不用 PAM 的 su）。**当前 swap 的做法是绕开它**：会话直接以目标用户身份启动（§5），
不经过 `su`。缺口本身记在 STATUS 的"下一步"里，不假装已经解决。

## 5. 实现：`tools/guest/husk-session.sh`

`tools/guest/husk-session.sh <rootfs> [--user NAME] [--cwd DIR] -- <相对程序路径> [args…]`

- 从 guest 自己的 `/etc/passwd` 取 uid/gid/home/shell（不从 Android uid 推）；
- 把 `HOME USER LOGNAME SHELL PATH LANG TERM`、`NR_ROOTFS`、`NR_LD`、`NR_FAKE_UID`、`NR_FAKE_GID`
  一起交给 `env -i`，所以环境重置也不会把 shim 弄丢；
- 2.0 的 `su - <user>` 由 `NR_FAKE_UID/NR_FAKE_GID` + `bash -l` 直接达成；
- `--cwd` 用 `usr/bin/env -i -C <dir>` 实现（宿主侧 `cd` 与 guest 无关），程序路径在这里规范成绝对路径；
- 装 rootfs 与生成启动器的是 `tools/guest/install-husk.sh`（本地 tarball 或 URL，两遍解包补 SELinux
  拒掉的硬链接），用法与实测见 `docs/design/evidence/130-install-path-20261123.md`。

## 6. 验收（本机做到哪一步）

| 项 | 结果 |
|---|---|
| 用户会话身份 | `id` → `uid=1000(hu1) gid=1000(hu1) groups=1000(hu1)`，`id -un`=hu1 |
| 主目录 | `cd ~` → `/home/hu1`，登录脚本生效 |
| 默认工作目录 | `--cwd /usr/bin` → `pwd` = `/usr/bin`（不经登录 shell 也成立） |
| setuid 跟随 | `os.setuid(1001)` 后 `getuid/geteuid` 都变 1001 |
| 默认会话不变 | 不设 `NR_FAKE_*` 时仍是 `uid=0` / `root` |
| 硬链接 | guest 内 `ln` rc=0（副本回退） |
| `/proc` 合成 | `/proc/version`、`/proc/stat` 可读且有内容 |
| 免 preload 的网络枚举 | `getent`、`getifaddrs` 正常 |
| 回归 | `04-path-reliability` 31/31 + 15/15；bash 矩阵全通 |

**没有验证**：`su`/`sudo`（见 §4）、GUI 会话整体、把 2.0 的 Kotlin 侧真正接上
（那要改 2.0 的工程，不在本仓库范围；本仓库交付的是"替换后该长什么样"的可执行契约）。
