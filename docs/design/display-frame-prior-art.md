# 帧怎么从 guest 到 Android 屏幕：三种前人做法，我们选哪一种

> 触发：evidence 101 之后，GNOME Shell 已经在 guest 里跑起来，但帧还在 guest 里
> （`--headless` 的 surfaceless 渲染器）。这是显示链路最后一段，先把前人的三种做法摆出来。

## 三种做法（都是源码级读过，不是道听途说）

### A. 合成器放进 APK（wldroid / Piperella / Termux:X11 一派）

`wldroid/docs/compositor-architecture.md` 写得很直白：APK 里跑一个 **wlroots 合成器**
（`compositor_server.c`）+ `android_backend` + `android_output`：

- 输出端：`AHardwareBuffer` 分配 → 渲染 → `ANativeWindow_lock/unlockAndPost`；
- 输入端：guest 应用直接连 APK 里的 Wayland socket，走 `zwp_linux_dmabuf_v1`；
- 缓冲区：`ahb_registry` 用 **inode → AHardwareBuffer\*** 的 256 项表做零拷贝登记。

代价：**桌面 session 也是它自己的**。guest 里没有 mutter/gnome-shell 的位置
（gnome-shell 本身就要当合成器）。Piperella 同理（合成器做成 Gradle 库嵌进 APK）。
Termux:X11 是 X11 版本的同一条路（共享内存 + Surface）。

### B. 给 mutter 写一个 Android 后端（anland 一派）

`anland`（已深读，见 `anland-deep-read.md`）在 guest 里跑真正的 mutter/GNOME，
用自己的 backend 把帧交给 Android。这是**产品形态和我们最像**的做法：桌面是 GNOME，
不是"另一个合成器"。但它的 mutter 后端只支持 **mutter 49–51**，我们 2.0 的 rootfs 是
**mutter 46**；要么跟着升 rootfs 的 mutter，要么改后端适配 —— 而且那是编译期后端，
等于重编 mutter（GPL-3.0 边界要单独处理）。

### C. 走协议，不碰 mutter 源码（我们现在这条）

mutter 自带 `org.gnome.Mutter.ScreenCast`：给一个虚拟显示器 → 帧以 PipeWire 视频节点
出来（evidence 97/99 已实测：`Stream/Output/Video`、BGRx、state=running）。我们只做
协议层消费者：PipeWire 节点 → dmabuf → AHardwareBuffer → SurfaceControl。
不改 mutter、不重编、不引入第二个合成器。

## 为什么继续走 C（JUDGMENT，附反面理由）

1. **形态要求**：目标是 2.0 的 GNOME 桌面，不是"自研合成器 + 任意桌面"。A 路线等于把
   gnome-shell 换掉，验收项（GNOME、nautilus、浏览器）全变。
2. **不重编 guest 组件**：B 要把 mutter 从 46 升到 49+ 或维护 GPL 后端补丁，等于把
   一个上游组件的生命周期绑到自己身上；C 只依赖公开 D-Bus 协议。
3. **已知缺口在 C 里是工程问题**：还差两件 ——
   ① session manager（wireplumber 0.4 因 logind 依赖退出 70，或自己实现最小策略层自动连线）；
   ② 帧消费者（PipeWire → dmabuf → AHB → SurfaceControl）。
   A/B 的缺口是"整个桌面栈重做"，量级不同。

## 从 A 借来的两条具体做法（直接可用）

- **AHB 登记表按 inode 索引**（wldroid `ahb_registry`）：我们从 PipeWire 拿到的 dmabuf fd
  也可能是同一个 BO 的多次导出，按 inode 去重，避免重复 import；
- **`ANativeWindow_lock/unlockAndPost` 是保底路径**，GPU 路径（EGLImage import）不通时
  退化到 CPU 拷贝并记录开销 —— 与我们 evidence 54 的 3-buffer ring 结论一致。

## 未决（写在这里，别到时候忘）

- mutter 46 的 ScreenCast 流是 BGRx **system memory** 还是 dmabuf？evidence 99 记录的是
  BGRx，但没确认 buffer type。若只有 system memory，第一步就得先解决"PipeWire 出 dmabuf"，
  否则零拷贝无从谈起。这是下一次上机的第一件事。
