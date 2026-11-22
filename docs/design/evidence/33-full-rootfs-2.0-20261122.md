# evidence 33 — 用 2.0 的完整 rootfs 跑起来（goal B 的第一段）

设备：RMX3357 / Android 13 / 4.19.191 / 4 KiB
上下文：**guest**（`untrusted_app`，uid u0_a302）

## 怎么造的（全部 Rust，`nr-tools rootfs`）

2.0 的资产不是"铺好的 rootfs"，而是 **base tar + 一堆离线 .deb**：

    LinuxHub-2.0/app/src/main/assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar   31 MB
    LinuxHub-2.0/app/src/main/assets/offline/bootstrap-arm64.tar     → 598 个 .deb
    LinuxHub-2.0/app/src/main/assets/offline/gnome-24.04-arm64.tar   → 729 个 .deb（去重后合计 894）

用 `nr-tools rootfs --base … --debs …` 直接铺（不走网络、不跑 dpkg 维护脚本）：

    runtime/target/release/nr-tools rootfs --fresh \
      --out /tmp/rootfs-full \
      --base  …/rootfs/ubuntu-base-24.04.4-base-arm64.tar \
      --debs  /tmp/debs2/packages \
      --tar   /tmp/rootfs-full.tar.gz \
      --loader-out /tmp/libld-full.so \
      --shim binaries/libpath_shim.so --hook tools/loader/init-hook.bin

产物与实测：

| 项 | 值 |
|---|---|
| usr/bin 条目 | **817** |
| usr/share/applications | 95 |
| **libc** | glibc **2.39-0ubuntu8.7**；`svc` 站点扫描出 2 个 `set_robust_list(99)`，已按断言打 `nop` |
| **loader** | 从该镜像**自己的** `ld-linux-aarch64.so.1` 生成；`bss pad: section headers moved 202432 -> 290816` |
| 页对齐体检 | `nr-tools check-elf`：4096 / 16384 / 65536 全部 congruent |
| tar | 776 MB；设备上解开 **1.9 GB** 到 `files/rootfs-full` |

## 上机结果（FACT，全部在 guest 内、`untrusted_app` 域）

| 命令 | 输出 |
|---|---|
| `usr/bin/bash -c 'head -3 /etc/os-release'` | `PRETTY_NAME="Ubuntu 24.04.4 LTS"` |
| `usr/bin/gjs --version` | `gjs 1.80.2` |
| `usr/bin/dbus-daemon --version` | `D-Bus Message Bus Daemon 1.14.10` |
| `usr/bin/Xwayland -version` | `Xwayland Version 23.2.6` |
| `usr/bin/gnome-shell --version`（带额外库目录） | **`GNOME Shell 46.0`** |
| `usr/bin/python3 -V` | `Python 3.12.3` |

`gnome-shell` 的调用方式（库目录要补，见下面缺口 2）：

    F=/data/data/com.linuxhub.husk/files; R=$F/rootfs-full
    cd $R && NR_ROOTFS=$R $F/libld39.so \
      --library-path $R/usr/lib/aarch64-linux-gnu:$R/usr/lib/gnome-shell:$R/usr/lib/aarch64-linux-gnu/mutter-14 \
      usr/bin/gnome-shell --version

注意：**loader 放在 app 私有目录里可以直接执行**（`files/libld39.so --version` 成功，
`ld.so (Ubuntu GLIBC 2.39-0ubuntu8.7)`），不需要塞进 jniLibs、不需要重装 APK。
这与"app 数据目录禁止 exec"的常见说法相反 —— 在本 ROM / 本 targetSdk 下实测可执行。

## 两个真实缺口（本轮发现，未修）

1. **带绝对 shebang 的脚本跑不了**：`usr/bin/gnome-terminal` 第一行是 `#!/usr/bin/python3`
   （rootfs 里确实有 python3.12），但内核在 execve 时按**宿主**路径解析解释器 → ENOENT，
   bash 报 `cannot execute: required file not found`。
   修法在 shim 的 `execve` interpose 里：读到 `#!` 就自己把解释器映射进 rootfs 再执行
   （proot 用 ptrace 做同一件事，我们不拦 syscall，只能在这一层做）。
2. **库目录不止一个**：Ubuntu 的 `ld.so.conf.d` 把 `mutter-14`、`gnome-shell` 等目录加进
   `ld.so.cache`，我们没有 cache → 必须显式把这些目录拼进 `--library-path`。
   计划：`nr-tools rootfs` 在构建期扫出"含 .so 的目录"写进 `etc/husk/libdirs`，运行器读它拼路径。

## 与目标的关系

goal B 的验收是"GNOME 桌面能起，浏览器/文件管理器/终端可用"。本轮证明的是**整条链的底座**：
2.0 的完整用户态（含 GNOME Shell 46、mutter、gjs、dbus、Xwayland）在我们的 loader + 打过补丁的
glibc 2.39 下能启动并报版本。还差：会话总线 + 合成器 + 窗口（C 段的显示桥），以及上面两个缺口。
