# 174 · 把窗口 swap 拆成「GPU 渲染」和「呈现」两段：呈现占 85%–95%

日期：2026-11-25 07:1x–07:3x（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13），
应用 `com.linuxhub.myth.husk`，X server `:0`（X.Org 21.1.99，跑在应用自己的 `:x11` 进程里）。
这次**没有**跑 gnome-shell：探针自己开一个窗口，所以量到的是「客户端 → X server」这一跳，
不含合成器再合成一次的成本。

## 为什么要做

`docs/LIMITATIONS.md` §7 之前写的是「640×480 上 zink 131 fps、llvmpipe 300 fps，瓶颈是那次
SHM 拷贝」。这句话是**推断**，没做过拆分：窗口 swap 里同时有 GPU 渲染、GPU→CPU 回读、
挂到 X server 的拷贝、server 侧导入。不拆开就不知道要不要为「零拷贝」动手。

## 做法

1. `gl-probe` 加了两个参数（`window WxH N` 之前写死 640×480 / 60 帧）和一个新子命令
   `pbuf WxH N`：同一套 EGL/GL，改成 surfaceless display + 同尺寸 pbuffer，同样的
   clear+swap 循环，**每帧 `glFinish()`**；
2. 两种渲染器各跑一遍：`GALLIUM_DRIVER=zink`+PanVK ICD，和 `GALLIUM_DRIVER=llvmpipe`；
3. 尺寸 160×120 / 320×240 / 640×480 / 960×960 / 1080×1080，每档 60 帧；640×480 与
   1080×1080 各重复一次看抖动；
4. 屏幕保持点亮（`svc power stayon usb`），窗口尺寸都在屏内（1080×2400）。

没有 `glFinish` 的第一版 pbuffer 量到 0.1 ms / 60 帧 —— 那只是把 Vulkan 命令录进队列，
一个像素都没画。这一条记下来：**离屏基线不带同步就是假数字**。

## 数字（ms/帧，60 帧平均）

| 尺寸 | 像素(M) | zink window | zink pbuf | llvmpipe window | llvmpipe pbuf |
|---|---:|---:|---:|---:|---:|
| 160×120 | 0.019 | 4.04 | 1.22 | 1.07 | 0.36 |
| 320×240 | 0.077 | 4.52 | 1.14 | 1.57 | — |
| 640×480 | 0.307 | 8.49 / 9.80 / 10.19 | 1.28 | 4.28 | 0.71 |
| 960×960 | 0.922 | 23.64 | 1.65 | 12.50 | — |
| 1080×1080 | 1.166 | 28.84 / 30.68 / 32.15 | 1.70 | 13.00 | 1.05 |

差值（window − pbuf）= 呈现这一段，按窗口面积折算：

| 渲染器 | 640×480 差值 | 1080×1080 差值 | 每兆像素 |
|---|---:|---:|---:|
| zink (PanVK) | 7.2–8.9 ms | 27.1–30.5 ms | **≈ 24.8 ms/MP** |
| llvmpipe | 3.6 ms | 12.0 ms | **≈ 11 ms/MP** |

## 能说明什么（FACT）

- **GPU 渲染本身跟尺寸几乎无关**：zink 从 160×120 到 1080×1080（60 倍像素）只是 1.22 → 1.70 ms。
  所以「GPU 算力不够」不成立，`LIMITATIONS` §7 那条判断方向是对的。
- **窗口 swap 的成本按窗口面积线性长**，斜率 24.8 ms/MP（zink），折算约 4 B/px → **≈ 161 MB/s**
  的有效带宽。这是「读了像素、搬了像素」的量级，不是 GPU 算力。
- **zink 的呈现成本是 llvmpipe 的 2.4 倍/像素**。所以那段成本不只是「一次拷贝」：llvmpipe 的结果
  本来就在 CPU 内存里，zink 还得先把 tiled 的 GPU 表面回读/转成线性再交给 X。
  也就是说 640×480 那 8.9 ms 里，GPU 只占 1.3 ms，剩下是**回读 + 拷贝**。
- 之前 README/Release 里「131 fps 输给 300 fps」的对比，瓶颈确实在呈现链，不在 GPU —— 这次是
  减法量出来的，不是推断。

## 不能说明什么

- 没跑合成器：真桌面里还有 mutter 再合成一次，成本只会更高，不会更低。
- 没有分离「回读」和「X 拷贝」各自多少：要分离得再做一个把 GPU 表面直接落到线性 map 的变体。
- 不含 Android 侧 `AHardwareBuffer → SurfaceControl`（那段 evidence 59 测过是零拷贝）。

## 对路线的意思（INFERENCE）

1. 零拷贝能拿回的是 zink 那 ≈ 24.8 ms/MP 里的绝大部分：1080×1080 从 ~30 ms 降到个位数是可期的。
2. 但这台机上**DRI3 做不出来**：X server 是应用进程里的 bionic Xorg，`untrusted_app` 打不开
   `/dev/dri`，`/dev/dma_heap` 不存在、`/dev/ion` EACCES → 服务器没有 DRM 设备，DRI3 只能是
   广告出来然而「Could not get DRI3 device」。这属于 shell（X server）那一层的能力，不是
   `path-shim` 能补的。
3. 所以在当前架构下可选的是：给 X server 造一条能 import dma-buf 的路（要 shell 侧配合），
   或者接受这次拷贝并把它写清楚。**不要再用「换个合成器/上 Wayland」来解**——那条已经否决。
