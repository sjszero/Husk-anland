# 限制

这份文件写的是 Husk 做不到、或者还没做到的事。每条都来自目标机上的实测，或者明确标成
未验证。**没写在这里不等于能用**——那只是还没研究。

参考环境（实测条件）：arm64 Android 13、kernel 4.19.191、4 KiB 页，以 `untrusted_app`
运行，无 root。机型与 SoC 只是那一次实测的现场，不是运行时的要求。

## 1 · 一个 ABI、一个设备族（FACT）

只跑过 arm64 Android、4 KiB 页、glibc 2.39 的发行版。loader 的补丁是版本断言的
（`nr-tools scan-syscalls` 校验 glibc 版本与 `set_robust_list` / `rseq` / `clone3` 的
site 数量），所以换发行版或换 glibc 构建会**直接报错**，而不是半好不坏地跑起来。
16 KiB / 64 KiB 页只在 QEMU 上验证过（evidence 125），没在手机上验证。

## 2 · 没有 root、没有解 Bootloader、没有虚拟化（设计如此，永久）

Husk 跑在普通应用沙箱里。沙箱碰不到的东西就碰不到：内核模块、别的应用的数据、`hidepid`
隐藏的 `/proc` 项、裸块设备、系统级策略开关。

## 3 · Termux 不是交付形态（FACT，已实测）

用户态需要的二进制与库 Termux 的 `$PREFIX` 不提供，在那边跑会直接卡在缺依赖上。支持的
目标目录是能执行且可写的那种（设备上的 `/data/local/tmp` 与应用私有目录都满足，见
`docs/RUNNING.md`）。这不是脚本能补的缺口。

## 4 · `apt upgrade` 升 libc6 会拆掉运行时（FACT，已给修法）

`apt` 升级 `libc6` 会把打过补丁的 libc 换成 stock 版本，丢掉两处补丁：libc 内部调用的 slot
页、以及 `clone3 → ENOSYS`。之后 `getent shadow` 返回空、提权修复脚本失败、会话起不来，
报错是一句误导性的 `/bin/sh: 1: Cannot fork`（evidence 154）。重跑
`tools/guest-config/install-libc-patch.sh` 即可。

## 5 · rootfs 里不能有绝对符号链接（FACT，已给工具）

内核按 `/` 解析符号链接目标，所以 rootfs 里的绝对链接会逃到 Android 文件系统。Ubuntu
24.04 的树里有 552 个，`libblas.so.3` 之类经 `alternatives` 的链接全部受影响。
`tools/guest-config/relative-symlinks.py --apply` 改写成相对路径；这是 rootfs 准备步骤，
不是一次性修补（`docs/RUNNING.md`）。

## 6 · 取样走 `adb shell`，它的 stdout 是 socket（FACT）

guest 里 `/proc/self/fd/1` 是 socket。`make` 往它写完（309 字节就够）之后，同一会话后续
写入全部 `EACCES`。proot 下完全一样，所以不是 shim 的行为；但它限定了基准的取样方式：
构建类步骤统一重定向到 `/dev/null`，只记墙钟（evidence 155）。

## 7 · GPU 只到"能开、能算"，没到"能上线"

GPU 那条路是真的：用户态直开 `/dev/mali0`，PanVK 给出 logical device，compute 与离屏
graphics 通过，Zink surfaceless 也通（evidence 97）。**没有做的是 WSI / swapchain**，也就是
把一个窗口的表面交给 GPU 直接呈现这条链；显示侧目前走的是 `AHardwareBuffer` → dma-buf →
`SurfaceControl` 的零拷贝路径（evidence 59），两条链还没有合起来。

## 8 · 未开始

传感器、GPS、USB、蓝牙、NFC、指纹、硬件视频解码、硬件编码。随附 rootfs 里没有浏览器包，
而且没有跑过任何一个浏览器。
