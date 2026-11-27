# 329 · 第 6 轮 GPU：四路 300 秒长稳通过，桌面仍卡在宿主 X server

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / arm64 / 4 KiB 页。应用：`com.linuxhub.husk`。

## 0 · 结论

- **FACT**：同一正式 rootfs 上 4 路并发运行 Rust `gl-probe soak 2400x1080 300`，
  4 路全部是 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，每路 `errors=0`，
  fd 从开始到结束均为 7，RSS 变化均为 `+244 kB`，门禁
  `GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0`。
- **FACT**：`make gpu-audit` 的 19 项读回段通过；随后的桌面段失败为
  `PASS=0 FAIL=3 SKIP=0`，原因是当前 App 没有自己的 X server，`glxinfo` 报
  `Error: unable to open display :0`。
- **FACT**：当前 APK 只有 `libhuskapp.so`、`libhuskguard.so`、`libld.so`；
  发行 rootfs 没有 `Xorg` / `Xvfb`，也没有自建 X11 socket。独立 GNOME/X11 桌面
  仍是 **UNVERIFIED**，不能把 `com.linuxhub.myth.husk` 的 X server 借来充当 Husk 结果。
- **INFERENCE**：第 6 轮的运行时 GPU 部分已完成；桌面 + GUI 应用部分不是 path-shim
  或 loader 能补的，仍需要应用内 X server / 输入 / Surface 呈现这一层宿主组件。
- **UNKNOWN**：Adreno/KGSL 没有真机；16/64 KiB 真机未验证。

## 1 · 四路 300 秒长稳（FACT）

命令：

```sh
DEV=UKHMMB8PYTGECMNV SECONDS=300 JOBS=4 make gpu-soak
```

门禁原文：

```text
STATE p1 renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS  job 1 used zink/Mali
PASS  job 1 completed with errors=0
PASS  job 1 fd count stable (7)
PASS  job 1 RSS bounded (delta=244kB)
STATE p2 renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS  job 2 used zink/Mali
PASS  job 2 completed with errors=0
PASS  job 2 fd count stable (7)
PASS  job 2 RSS bounded (delta=244kB)
STATE p3 renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS  job 3 used zink/Mali
PASS  job 3 completed with errors=0
PASS  job 3 fd count stable (7)
PASS  job 3 RSS bounded (delta=244kB)
STATE p4 renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS  job 4 used zink/Mali
PASS  job 4 completed with errors=0
PASS  job 4 fd count stable (7)
PASS  job 4 RSS bounded (delta=244kB)
GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0
```

四份原始日志的关键行：

```text
TAG p1 GL_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
START p1 2400x1080 for 300s rss=368672kB fd=7 mali_fd=2
DONE p1 frames=171389 avg_fps=571.3 peak_fps=600 rss=368916kB (start 368672kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0

TAG p2 GL_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
START p2 2400x1080 for 300s rss=368492kB fd=7 mali_fd=2
DONE p2 frames=171100 avg_fps=570.3 peak_fps=601 rss=368736kB (start 368492kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0

TAG p3 GL_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
START p3 2400x1080 for 300s rss=368604kB fd=7 mali_fd=2
DONE p3 frames=171165 avg_fps=570.5 peak_fps=604 rss=368848kB (start 368604kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0

TAG p4 GL_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
START p4 2400x1080 for 300s rss=368476kB fd=7 mali_fd=2
DONE p4 frames=171467 avg_fps=571.5 peak_fps=599 rss=368720kB (start 368476kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0
```

这不是 GUI 应用并发的证据。它验证的是：四个独立 guest 进程同时创建 zink/PanVK
上下文，连续运行 300 秒，没有串上下文、fd 泄漏或 GL 错误。整数 RSS 波动不作性能结论。

## 2 · 门禁自身的一次负向修正（FACT）

第一次正式 300 秒运行的 4 个工作负载都完成了，但旧 RSS 判据把 `-1828 kB` 的 RSS 下降
误判成“增长”，结果 `FAIL=1`。这是测试门禁的正负号 bug，不是 runtime 泄漏。
修成绝对变化量后重新跑完整 300 秒，得到上面的 `PASS=19 FAIL=0`。

这条修正对应缺陷账本 `G6`，不把测试器错误写成驱动缺陷。

## 3 · 桌面门禁：失败原文（FACT）

命令：

```sh
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
  ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs \
  LD=files/linux/husk/libld.so \
  ENV=DISPLAY=:0,XDG_RUNTIME_DIR=/tmp/runtime-husk \
  sh tools/device-tests/run-guest.sh 15-gpu-global-audit.sh
```

原文：

```text
== existing desktop shell ==
  FAIL  gnome-shell process not readable (pid=missing)
== loader-default GL client ==
  FAIL  glxinfo did not report zink Vulkan 1.3(Mali-G77)
Error: unable to open display :0
== nested application ==
  FAIL  nested-gtk process not readable (pid=10128)
GPU_GLOBAL_AUDIT PASS=0 FAIL=3 SKIP=0
```

日志 SHA-256：

```text
9ae3fba929ef1fb356c95a2233cea0687f2d437ab670ef7bdba90d4ad956492a  /tmp/husk-gpu-desktop-audit-20261127.log
```

当前 APK 与 rootfs 的事实边界：

```text
APK: libhuskapp.so / libhuskguard.so / libld.so
rootfs: Xorg absent / Xvfb absent / self-created X11 socket absent
display :0: unavailable from the current app process
```

因此本轮不能写“桌面 GPU 已全局通过”。GUI 应用、窗口动画和输入链路必须在应用内 X server
具备后重新验收。

## 4 · PanVK 兼容边界（FACT，沿用已有构建证据）

当前 loader 默认注入：

```text
PANVK_DXVK_COMPAT=0
```

这是对该 Mali/Bifrost PanVK 构建的显式规避。`PANVK_DXVK_COMPAT=1` 仍会打开已知的
geometryShader/DXVK 兼容路径，不能把它说成“已经修好”。本轮因没有 X server，不能用
`glxgears` 重新证明当前 X11 崩溃状态；桌面崩溃边界仍是 **UNVERIFIED**。

相关旧证据保留：evidence 232、244、273、292、300。新证据只补充当前四路 Rust 长稳与
当前桌面门禁实际失败。

## 5 · 原始日志与复现

四份长稳日志：

```text
/tmp/husk-gpu-soak-p1.log
/tmp/husk-gpu-soak-p2.log
/tmp/husk-gpu-soak-p3.log
/tmp/husk-gpu-soak-p4.log
```

SHA-256：

```text
594f70cea0d7778e5b7c9ce73317f0b207de4722446dba305f4ee9fa604d3b6f  /tmp/husk-gpu-soak-p1.log
abf1269bc2dc7040924436dcdc450ca594dede5e349e8b2469b80faa053a7ea7  /tmp/husk-gpu-soak-p2.log
782228cae9f62066e05709c0dec4663033f1779d243bd96a76af8a8157497a28  /tmp/husk-gpu-soak-p3.log
35b3e12bfd3fcbaa525c227e91fe8b43e91322c55cf773af2a4687c7e0905c1a  /tmp/husk-gpu-soak-p4.log
```

复现：

```sh
DEV=UKHMMB8PYTGECMNV SECONDS=10 JOBS=2 make gpu-soak
DEV=UKHMMB8PYTGECMNV SECONDS=300 JOBS=4 make gpu-soak
DEV=UKHMMB8PYTGECMNV make gpu-audit
```

`make gpu-audit` 在当前 APK 上应继续在桌面段失败，直到应用内 X server 完成；不能把失败
改成 SKIP 来伪装发布。

## 6 · 边界

- 4 路长稳是 300 秒，不是数小时；更长时间与真实 GUI 应用仍为 **UNVERIFIED**。
- 目前只验证 Mali-G77 MC9；Adreno/KGSL 为 **UNKNOWN**。
- 目前只验证 Android 13 / 4 KiB 页；16/64 KiB 真机为 **UNVERIFIED**。
- `KBASE_IOCTL_MEM_EXEC_INIT` 的 `Operation not permitted` 仍存在，不能因此推导所有
  executable BO 工作负载通过。
- 本证据不修改 `VERSION`，不创建 tag/Release。
