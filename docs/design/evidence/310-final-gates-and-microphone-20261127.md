# 310 · 最终树门禁 A、A-hw 与麦克风真实 PCM（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`
最终树：`files/release-tree-final/root`
Loader：`files/husk-dist-final2/libld.so`

## 0 · 结论

- **FACT**：最终树 `TREE_INTEGRITY PASS=23 FAIL=0`。
- **FACT**：最终树 Gate A `PASS=34 FAIL=0 SKIP=1`。唯一 SKIP 是离线树没有 apt index，
  不是运行时失败。
- **FACT**：GPU loader 默认值 `GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0`。不带 GPU 环境变量
  的进程报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；两个反向控制都落到 llvmpipe。
- **FACT**：活动树 A-hw `PASS=11 FAIL=0`。Camera2 三帧 `1,382,400 bytes/frame`，麦克风
  返回 `196,608` PCM bytes，`peak=394`，`silent_chunks=1`。
- **FACT（路径统一后）**：先把 `files/rootfs-full` 从 `release-tree-clean2/root` 切到
  `release-tree-final/root`，再冷启动 App；同一最终树上的 A-hw 为 `PASS=11 FAIL=0`。
  修前直接对尚未接入 App bridge 的旁路目录跑 A-hw 得到 `10/1`，原因是 socket 路径未统一，
  不是音频链路缺陷。现在正式入口、bridge socket、A-hw 指向同一棵最终树。

## 1 · Gate A

```text
PASS  guest uid is root (0)
PASS  bash version (5.2.21(1)-release)
PASS  locale zh_CN weekday (星期一)
PASS  user management (8/8)
PASS  dpkg installed packages (807 >= 90)
PASS  dpkg -C clean after (0)
PASS  /proc/self/exe resolves inside guest (ok)
PASS  real SIGSYS traps (0)

PASS=34 FAIL=0 SKIP=1
```

## 2 · GPU 默认值

```text
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1 -> llvmpipe (LLVM 20.1.2, 128 bits)
HUSK_GPU=llvmpipe -> llvmpipe (LLVM 20.1.2, 128 bits)
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

## 3 · A-hw

```text
PASS  CAMERA granted=true
PASS  CAMERA is not USER_FIXED
PASS  RECORD_AUDIO granted=true
PASS  RECORD_AUDIO is not USER_FIXED
PASS  captured 3 frame(s) through Camera2 (1382400 bytes each)
PASS  app opened a Camera2 session
PASS  capability matrix readable
PASS  audio capture device present
NOTE  capture-probe|PASS|chunks=48|frames=49152|bytes=196608|silent_chunks=1|peak=394
PASS  microphone returned 196608 PCM bytes (peak=394)
PASS  no crash records for the app process
PASS=11 FAIL=0
```

## 4 · 最终树接入 App 正式入口

可逆地把 App 的稳定入口切到刚安装的树，并冷启动 App：

```text
PREV=release-tree-clean2/root
files/rootfs-full -> release-tree-final/root
```

随后逐项复验：

```text
TREE_INTEGRITY PASS=23 FAIL=0
Gate A          PASS=34 FAIL=0 SKIP=1
A-hw            PASS=11 FAIL=0
GPU defaults    PASS=9 FAIL=0 SKIP=0
```

A-hw 的关键输出：

```text
camera: captured 3 frame(s) through Camera2 (1382400 bytes each)
capture-probe|PASS|chunks=48|frames=49152|bytes=196608|silent_chunks=4|peak=285|elapsed_ms=1074
microphone returned 196608 PCM bytes (peak=285)
```

这一轮只验证 Linux 运行环境、硬件桥和发行树接入。桌面与 X11 的宿主载体不在 Husk 运行资产内，
不能用其他应用或旧树里的 X socket 代替；详见 evidence 312。

## 5 · 最终构建复验

从源码重建最终 APK 后，重新安装到同一设备并复跑 A-hw：

```text
APK version contract OK: 0.0.36 (36)
OK   lib/arm64-v8a/libhuskapp.so
OK   lib/arm64-v8a/libhuskguard.so
OK   lib/arm64-v8a/libld.so
capture-probe|PASS|chunks=48|frames=49152|bytes=196608|silent_chunks=5|peak=254
PASS=11 FAIL=0
```

APK 不作为本仓库 release 资产；本段固定的是最终 APK 的版本、三个 native 产物和安装后硬件链路。
PRoot 对照、X11 宿主边界仍分别以 evidence 311、312 为准。
