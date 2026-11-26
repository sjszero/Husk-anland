# 290 · DRI3 定论（结构性不可达）与 2400×1080 场景的呈现代价全拆

日期：2026-11-26 20:40（CST）。设备 UKHMMB8PYTGECMNV（RMX3357 / MT6893 / Mali-G77 MC9 /
Android 13 / 4.19.191 / 4 KiB 页），应用 `com.linuxhub.husk`，guest 树
`files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126`。

第 5 轮要求把 DRI3 / dma-buf 在「X server 跑在应用内、本机无 render node」这个前提下
**说死**，不再留「以后再说」。这一份给结论、给判定链、给每一段的实测毫秒。

## 1 · 现场事实（本轮实测）

### 1.1 真实视角（bionic 进程看到的，也就是 X server 看到的）

```
/dev/dri           只有 card0   226:0  0660 system:system
/dev/dri/renderD128  不存在
/dev/dma_heap      不存在
/dev/udmabuf       不存在
/dev/ion           10:60  0666  （run-as 域 read() 返回 EINVAL = open 成功）
/dev/mali0         10:33  0666  （GPU 走的是 kbase，不是 DRM）
```

`run-as com.linuxhub.husk cat /dev/dri/card0` → **`Permission denied`**（FACT）。
所以 X server 拿不到任何 DRM fd：没有 render node、`card0` 无权限、`dma_heap`/`udmabuf`
都不在，gbm 也无从谈起。

### 1.2 guest 视角不能拿来当证据

经 husk loader 起来以后（`run-as ... sh <rootfs>/tmp/husk-exec.sh ... python3`）再 open 一遍：

```
/dev/ion            OPEN OK
/dev/dri/card0      OPEN OK
/dev/dri/renderD128 OPEN OK      ← 这个节点在真实视角里根本不存在
/dev/mali0          OPEN OK
```

`renderD128` 能 open，说明这一层是 **path-shim 的假设备**（README/SECURITY 里写过的
「假 DRM / V4L2 / netlink fd」）。它能让 Mesa 的设备探测过关，**不代表**存在可以被
DRI3 使用的真设备、更不代表有 dma-buf 的导入导出语义。
**以后凡是要判断「设备到底在不在」，必须用不经过 shim 的视角。**

### 1.3 这棵树没有 Vulkan ICD

```
usr/share/vulkan            不存在
usr/lib/aarch64-linux-gnu/  只有 libvulkan.so.1（加载器），没有 libvulkan_*panvk*
dri/                        有 zink_dri.so、panfrost_dri.so、kms_swrast_dri.so
```

没有 ICD，`zink_dri.so` 就没有 Vulkan 实现可指 → **这棵树的 GL 只能是 llvmpipe**（FACT）。
`install-husk.sh` 生成 `usr/share/vulkan/icd.d/husk_panvk_guest.json` 的前置条件
（树里已经有 PanVK 驱动）在这里不成立，脚本自身也会打印
`no PanVK driver in the tree`。所以下面凡是 zink 的数字都不是在本树上测的，来源单独标注。

## 2 · DRI3 定论

**结论：在本布局里 DRI3 结构性不可达，不是参数问题，也不是 path-shim 能补的。**

判定链（每一环都是上面量过的事实）：

1. DRI3 要求 X server 先持有**真 DRM fd**：`DRI3Open` 要把这个 fd 交给客户端，
   `DRI3PixmapFromBuffer` 之后客户端用 gbm 在这颗 fd 上分配/导入 dma-buf。
2. server 是本应用里的 bionic Xorg 进程（X.Org 21.1.99），不走 shim → 它看到的是 1.1 的视角。
3. 三条可能的来源全断：**没有 `renderD128`**、**`card0` 是 `0660 system:system` 打不开**、
   **`/dev/dma_heap` 与 `/dev/udmabuf` 都不存在**。
4. 退一步说，就算拿到 `card0`（226:0），那也不是 Mali 的节点 —— Mali 走 kbase 的
   `/dev/mali0`（10:33），kbase 不是 DRM 驱动，没有 PRIME/dma-buf 语义。
5. 于是扩展广告在、设备没有：`DRI3 error: Could not get DRI3 device`。

**MIT-SHM 同理不可达**（evidence 228）：guest 侧 SysV IPC 是 shim 仿真的，server 是 bionic、
走真内核，而真内核把 SysV IPC 编掉了 → `XShmAttach` 必报 `BadAccess`。
后果是**每个 X 客户端都退回 `XPutImage`**，这一跳无论如何要付。

## 3 · 代价定量（2400×1080 = 2.592 MP，逐段实测）

本轮新测（`tools/device-tests/probes/readback.py ... cost`，llvmpipe，本树，12 帧）：

| 尺寸 | 渲染（clear+glFinish） | 回读（逐行 `W×1`，已知正确的那条路） |
|---|---:|---:|
| 2400×1080 | 5.92 ms（2.28 ms/MP） | **7.29 ms（2.81 ms/MP）** |
| 1080×1080 | 3.44 ms | 6.07 ms |
| 1280×720 | 2.82 ms | 4.21 ms |

已知分段（同机实测，引用）：

| 段 | 2400×1080 | 来源 |
|---|---:|---|
| X 协议拷贝（`XPutImage`，无 MIT-SHM） | **18.92 ms**（7.30 ms/MP，52.9 fps 上限） | evidence 228 |
| zink 的 GPU→CPU 回读 | **30.27 ms** | evidence 174/176 |
| llvmpipe 回读 | 7.29 ms | 本轮 |

交叉验证（两条独立路径互相对上，差值 3%）：

- zink：回读 30.27 + 拷贝 18.92 = **49.2 ms** ⇔ 实测整窗 clear+swap **50.3 ms（19.9 fps）**。
- llvmpipe：实测整窗 clear+swap **20.1 ms（49.7 fps）**。
- 174 的斜率法：zink 24.8 ms/MP × 2.592 = 64.3 ms、llvmpipe 11 ms/MP × 2.592 = 28.5 ms；
  与上面按段相加的 49.2 / 26.2 同一量级，zink 差得多一些（INFERENCE：174 的斜率是从
  ≤1.166 MP 推到 2.592 MP 的，线性外推在 zink 上偏保守；以 2400×1080 的直测为准）。

**换算成人话**：全屏 2400×1080 的窗口，zink 现在封顶 **19.9 fps**，其中 30.27 ms 是
「GPU 表面回读成 CPU 像素」，18.92 ms 是「把像素塞进 X server」。llvmpipe 封顶 49.7 fps。
**零拷贝（DRI3 那一类）能拿回的全部价值就是那 30.27 ms**：19.9 fps → 约 50 fps。

## 4 · 既然 DRI3 不可达，还有什么路（成本、验收都写死，不留「以后再说」）

| 路线 | 能拿回 | 前置条件 | 工作量 | 本轮判定 |
|---|---|---|---|---|
| A. 自建 AHB present：客户端把自己的 surface 做成 AHB，用 `SCM_RIGHTS` 交给应用内 X server，server 用 EGL image 贴上去 | zink 那条 **30.27 ms** 的大部分 | 同 uid + 同应用（已有）；SCM_RIGHTS 传 fd 已验证（evidence 59，AHB ring 跑过 9593 帧）；服务器是我们自己编的 | 三块：X 扩展、server 侧 import 与合成、guest Mesa 的 WSI 后端 | **不做，但排为 0.0.36 之后第一优先工程项** |
| B. MIT-SHM 后端换成 memfd/AHB 共享 | 只能省 18.92 ms 里的一部分 | 两侧都要改（客户端 shm 仿真要改成 fd 传递） | 中 | 不做：收益小于 A，且要动 X 协议两侧 |
| C. 换合成器 / 上 Wayland | — | — | — | **已否决**（保持 X11） |
| D. 维持现状 + 写清数字 | 0 | — | 0 | 本轮采用 |

A 的验收标准（先写死，将来照这个判）：

1. 全屏 2400×1080 客户端 swap ≤ 20 ms/帧（现在 50.3 ms）；
2. `gl-probe window 2400x1080 600` 稳定不掉帧，门禁 A 全绿、A-hw 全绿；
3. 断掉 AHB 路径时能自动退回 `XPutImage`（不许黑屏、不许崩溃）。

## 5 · 不能说明什么

- zink 的 30.27 ms 与 50.3 ms 来自带 PanVK ICD 的树；**本树没有 ICD，zink 在本树测不了**。
- 只有探针窗口，没有在 GNOME 全桌面下逐窗口量；mutter 再合成一次的成本未计入。
- `/dev/ion` 的 open 是在 `u:r:runas_app` 域测的；`untrusted_app` 域**未测**（UNVERIFIED）。
  这不影响结论：A 路线靠的是 AHB（应用本来就在用），不依赖 ION。
