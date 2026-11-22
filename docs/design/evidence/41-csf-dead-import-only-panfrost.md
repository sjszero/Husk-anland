# Evidence 41 — CSF 是死的；UAPI 只有 IMPORT 没有 EXPORT；手搓 atom 到此为止

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-23 00:4x–01:3x（CST）
工具：`runtime/crates/gpu-probe`（纯 Rust，guest 内运行，经 Husk loader 启动）

> 本轮三个假设，全部有结论。结论决定第 2 条的实现形态从"可能走 panthor"收敛成
> **只能走 panfrost（JM + Valhall）**，也决定第 3 条的零拷贝方向要反过来。

## 1. CSF 是死的（FACT）

```
gpu|csf/glb_iface|FAIL|errno=25 (ENOTTY)  → 0xc0188033 不被接受
gpu|csf/kinstr_jm|PASS|fd=5 record_size=16 version=2
```

`KBASE_IOCTL_CS_GET_GLB_IFACE = _IOWR(0x80, 51, union{in 24B})` = `0xc0188033`。
同一个 nr 的 JM 版 `KBASE_IOCTL_KINSTR_JM_FD = _IOWR(0x80, 51, 8B)` = `0xc0088033` 正常返回。

⇒ 这棵 MTK 内核**只编进了 JM**，CSF 的 ioctl 面根本不存在（不是"没启用"，是 `ENOTTY`）。

**后果**：Mesa 的 panthor（CSF）后端在本机不可能工作。第 2 条唯一形态 = panfrost（JM）。

## 2. r32 这份 UAPI 没有 EXPORT，只有 IMPORT（FACT）

`/tmp/r32_mali_kbase_ioctl.h`（MTK GPL 树，r32p1）里 `grep -i export` 无命中；
只有 `KBASE_IOCTL_MEM_IMPORT`（nr 22，`union kbase_ioctl_mem_import` 24B）= `0xc0188016`。

实测：

```
gpu|imp/fd=-1|REACHED|errno=12 (ENOMEM)
gpu|imp//dev/null fd=5|REACHED|errno=12 (ENOMEM)
```

**REACHED 而不是 ENOTTY** → 命令存在，参数进了内核（ENOMEM 是"这个 fd 不是可导入的 dma-buf"之外的结果，
反向也说明它没有先做 fd 类型检查就报错了，具体语义下一步再钉）。

⇒ **零拷贝方向必须反过来**：不是「kbase 分配 BO 再导出 dma-buf 给 app」，
而是「**app 侧分配 dma-buf（AHardwareBuffer/gralloc）→ guest 用 MEM_IMPORT 导入 → GPU 直接渲染进去 → app 把同一块 buffer 交 SurfaceFlinger**」。
这才是这台机上唯一成立的零拷贝形状。第 3 条已按此改写。

## 3. atom 权威布局（FACT，r32p1 GPL 头 `base_jd_atom_v2`）

```
0x00 u64 jc
0x08 u64 udata[2]
0x18 u64 extres_list
0x20 u16 nr_extres
0x22 u8  jit_id[2]
0x24 {u16 atom_id; u8 dependency_type; u8 pad} pre_dep[2]   (0x24, 0x28)
0x2c u16 atom_number
0x2e u8  prio
0x2f u8  device_nr
0x30 u8  jobslot
0x31 u8  [3 pad]
0x34 u32 core_req            ← 确认
0x38 u8  renderpass_id
0x39 u8  padding[7]          ← 必须全 0（写 0x38 的 u32 会踩到 0x39.. → EINVAL）
0x40 u32 frame_nr            ← 仅 MTK_GPU_BM_2；sizeof = 72
```

**更正 evidence 40**：当时说「0x38 是个不为 0 就被拒的神秘字段」——那不是神秘字段，
是 **padding 从 0x39 开始**，一次 u32 写 0x38 必然踩进 padding。结论不变，机制说清了。

## 4. 硬件 atom 仍然拿不到 START（FACT），而且 kinstr 分不出成败（FACT）

用真实 GPU VA 当 jc（`mem_alloc` + `mmap`，`va=0x41000`，内容清零）：

```
gpu|hwbo/chain|INFO|gpu_va=0x41000 first word=0x0 core_req=0x1
gpu|hwbo/job_submit|ACCEPTED|errno=0 (ok)
gpu|hwbo/rec0|INFO|ts=… state=QUEUE(0) atom=0 flags=0x0 data=[00,00,00,00]
gpu|hwbo/rec0|INFO|ts=… state=COMPLETE(3) atom=0 flags=0x0 data=[00,00,00,00]
```

QUEUE → COMPLETE，间隔 ~1.8 µs，**没有 START**。

源码（`/tmp/r32_jd.c`）：

- `kbase_kinstr_jm_atom_complete()` 只从 `jd_mark_atom_complete()` 出，
  而 `jd_mark_atom_complete()` 同时被 `jd_run_atom()`（DEP/软作业）和 `jd_done_nolock()`（**所有**完成路径，含 `JOB_INVALID`）调用；
- `kbase_kinstr_jm_atom_hw_submit()` 只在真进硬件（`mali_kbase_jm_hw.c:317`）时调用。

⇒ **COMPLETE 这个状态对"成功"和"被判无效"是同义的**，`data[4]` 只有 START 时才有意义（`data.start.slot`）。
⇒ 手搓 atom 已经到顶：除非出现 START，否则拿不到任何新信息。

顺带记一条现场教训：第一轮提交的 atom 没被回收，第二次用同一 `atom_number` 提交会卡在
`wait_event_killable(katom->completed)`（`r32_jd.c:1352`），表现为进程挂住、无输出——不是驱动崩了。

## 5. 内核日志这条路是封死的（FACT）

```
ls /proc/mali            → No such file or directory
ls /sys/kernel/debug     → No such file or directory
dmesg                    → klogctl: Permission denied
```

⇒ 不能靠内核侧日志定位，只能靠"可观测的用户态结果"（kinstr 的 START、显存内容、GPU 频率）。

## 6. 顺带：26 条里的 5+2 条未知命令命名（FACT，GPL 头）

| nr | 命令 | 结构 |
|---|---|---|
| 29 | `KBASE_IOCTL_STICKY_RESOURCE_MAP` | 16B |
| 30 | `KBASE_IOCTL_STICKY_RESOURCE_UNMAP` | 16B |
| 38 | `KBASE_IOCTL_MEM_EXEC_INIT` | `{u64 va_pages}` |
| 50 | `KBASE_IOCTL_GET_CPU_GPU_TIMEINFO` | RW 32B |
| 51 | `KINSTR_JM_FD`（JM，8B）/ `CS_GET_GLB_IFACE`（CSF，24B） | 同 nr，不同 size，靠 size 区分 |
| 54 | `KBASE_IOCTL_CONTEXT_PRIORITY_CHECK` | `{u32}` |

## 7. 决定路线：rootfs 里已经有 panfrost（FACT）

```
files/rootfs-full/usr/lib/aarch64-linux-gnu/dri:  panfrost_dri.so  panthor_dri.so  zink_dri.so  swrast_dri.so …
files/rootfs-full/usr/lib/aarch64-linux-gnu/:      libgallium-25.2.8  libEGL_mesa.so.0  libgbm.so.1  libvulkan.so.1
```

Mesa **25.2.8**。panthor 用不上（CSF 死），**panfrost 正好是 JM + Valhall**。

⇒ job chain 不用我们搓，Mesa 会生成；我们要做的是把 `PANFROST_*` 的 DRM ioctl 翻成 kbase。
设计见 [gpu-drm-shim.md](../gpu-drm-shim.md)。

## 8. 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
export NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot \
 -C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
 -C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
cargo build --release --target aarch64-unknown-linux-gnu -p gpu-probe

adb push target/aarch64-unknown-linux-gnu/release/gpu-probe /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/gpu-probe files/rootfs/usr/bin/'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe csf'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe imp'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe kinstr_hwbo 0x1 0x0'
```
