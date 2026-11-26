# 307 · 发行树复验与相机策略阻塞

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`
树：`files/release-tree-vendor/root`（由发行 bundle 安装到空目录）
Loader：`files/linux-clean/libld.so`

> **更正（evidence 314）**：本文的 `PASS=17` 和“T3 已知负向缺陷”是旧探针常量错配造成的错误结论。修正后整块读通过，同一读回套为 `PASS=19 FAIL=0`；以下原始日志保留为历史记录。
>
> **后续复验（evidence 320）**：2026-11-27 晚些时候，同一台设备上的
> `Camera "0"/"1" disabled by policy` 已消失；A-hw 通过 11/0，前后摄各取 3 帧
> 1280×720 NV12。本文第 4 节保留为当时的阻塞记录，不再代表当前设备状态。

## 0 · 结论

- **FACT**：发行树体检 `TREE_INTEGRITY PASS=14 FAIL=0`。树内有 807 个 dpkg
  installed 包与 807 个 `.list`，`locale-archive` 为 6,222,864 字节，默认
  locale 是 `zh_CN.UTF-8`，zh_CN 语言包有 413 个 `.mo`。
- **FACT**：A-core `PASS=34 FAIL=0 SKIP=1`。唯一 SKIP 是离线树没有
  `/var/lib/apt/lists`，因此 `apt-cache policy` / `apt-get -s` 无法执行。
- **FACT**：GPU loader 默认值 `PASS=9 FAIL=0`。不带任何 GPU 环境变量的进程报
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；
  `NR_GPU_DEFAULT_OFF=1` 与 `HUSK_GPU=llvmpipe` 两个反向控制都落到 `llvmpipe`。
- **历史记录（已被 evidence 314 更正）**：当时旧套为 `PASS=17 FAIL=0`，T3 把探针常量错配
  造成的 `16x16` 整块读错误当成预期；修正后整块读通过，读回套为 `PASS=19 FAIL=0`。
- **UNVERIFIED**：A-hw 的前后摄真实出帧本轮没有通过。App 的 CAMERA /
  RECORD_AUDIO 权限均为 `granted=true`、无 `USER_FIXED`，App 日志也读到
  `permissions|OK|all hardware permissions granted`，但相机服务在打开设备时拒绝。
- **FACT（阻塞原因）**：`dumpsys media.camera` 原文：
  `Camera "0" disabled by policy`、`Camera "1" disabled by policy`。
  这是 Android CameraService 的宿主策略拒绝，不是 Husk socket、V4L2 facade 或权限申请失败。
- **UNKNOWN**：该策略由哪个系统开关触发（系统相机隐私开关、设备管理员策略或 ROM
  自定义控制）未确认；没有通过修改系统状态绕过它。

## 1 · 发行树体检（FACT）

命令：

```sh
DEV=... PKG=com.linuxhub.husk ROOTFS=files/release-tree-vendor/root \
  EXPECT=desktop sh tools/device-tests/20-tree-integrity.sh
```

输出：

```text
PASS  usr/bin/bash present
PASS  etc/os-release: Ubuntu 24.04.5 LTS
PASS  etc/husk/gpu-env present
PASS  driver=.../libvulkan_panfrost_frist370_execva.so
PASS  ICD manifest exists in the tree
PASS  dpkg records: 807 .list for 807 installed package(s)
PASS  locale-archive present (6222864 bytes)
PASS  zh_CN langpack: 413 .mo files
TREE_INTEGRITY PASS=14 FAIL=0
```

## 2 · A-core（FACT）

同一棵树、同一 loader：

```text
PASS=34 FAIL=0 SKIP=1
SKIP  apt-cache policy / apt-get -s (no package index in /var/lib/apt/lists)
```

Bash、locale、用户管理、dpkg、路径隔离、`/proc/self/exe` 和 SIGSYS 检查均通过。
没有把离线 apt SKIP 计为 PASS。

## 3 · GPU（FACT）

`18-gpu-loader-defaults.sh`：

```text
tree carries a PanVK driver
gpu-env family line matches this device (mali-kbase)
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1 -> llvmpipe (LLVM 20.1.2, 128 bits)
HUSK_GPU=llvmpipe -> llvmpipe (LLVM 20.1.2, 128 bits)
GPU_LOADER_DEFAULTS PASS=9 FAIL=0
```

`readback-suite.py`：

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
T1 width-boundary rows: all exact
T2 canary tails: all untouched
T3 known defect: 16x16 wrong 907/1024 bytes; 65x4 wrong 951/1040 bytes; glGetError=0
T4 240x135 screenshot row loop: byte-identical
PASS=17 FAIL=0
```

读回仍按逐行路径执行；整块 `glReadPixels` 不是已验证能力。

## 4 · A-hw：权限通过，设备策略拒绝（阻塞）

冷启动 App 后，权限状态：

```text
android.permission.CAMERA: granted=true
android.permission.RECORD_AUDIO: granted=true
USER_FIXED: absent
```

App 日志：

```text
permissions|OK|all hardware permissions granted
camera|list|count=2
camera|v4l2|tree|devices=2
camera|LISTEN|.../rootfs-full/run/husk/camera.sock
```

直接向 socket 发送 `OPEN w=640 h=480 id=0` 与 `id=1`，App 返回：

```text
ERR openCamera(0) rc=-10012
ERR openCamera(1) rc=-10012
```

CameraService 同时记录：

```text
Camera "0" disabled by policy
Camera "1" disabled by policy
```

因此本次 A-hw 结果为 `PASS=7 FAIL=2`：权限检查、App 状态读取和音频设备枚举通过；
Camera2 三帧采集与 session-open 断言失败。失败来自宿主相机策略，不能据此宣称前后摄在本轮通过，
也不能用旧 evidence 91/93/240/282 的结果替代本次复验。

## 5 · 后续条件

要完成第 8 轮相机门禁，需要在同一设备上先解除宿主相机策略，再重新执行：

```sh
DEV=... sh tools/device-tests/17-gate-a-hw.sh
DEV=... sh tools/device-tests/run-camera-fixture.sh ...
```

在策略解除前，前后摄、V4L2 facade 和 GStreamer 的本轮结果保持 **UNVERIFIED**。
没有修改 App 权限、没有使用 `install -g`、没有绕过 CameraService。
