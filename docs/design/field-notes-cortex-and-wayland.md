# 外部意见处理：Cortex 项目与「上 Wayland」两条

来源：仓库发布后，视频评论区两条意见（2026-11-24）。这里只记**看过之后得出的结论**，
以及要不要做、按什么顺序做。区分 FACT / INFERENCE / 判断。

## 一、Cortex（`ssfuisu/cortex`）——同一族，但技术选择不同

FACT（读它的 README 得到，未在本机复现其行为）：

- 它的做法是 **`LD_PRELOAD` 一个约 1 万行 C 的库**（`libcortex-hook.so`），在库里定义
  `open` / `stat` / `execve` 等约 200 个函数，做路径映射；`execve` 被改写成「用 Ubuntu 自己的
  `ld.so` 启动」。没有 `ptrace`、没有 seccomp 陷阱、没有第二个进程。
- 它给的每调用开销（它自己测的，8 核机、9 次中位数）：`stat` +39 ns（+3%）、
  `open`+`read`+`close` +226 ns（+8%）、`opendir…closedir` +426 ns（+7%），无系统调用的代码 0。
- 静态二进制：**首次启动前改文件**（静态 C/C++ 打掉被禁的系统调用；Go 1.19+ 注入一小段
  映射代码）。
- 它自己写的「不工作」清单里包含 **GPU**；桌面是 **X11 + CPU 绘制，走私有 socket**。

INFERENCE（基于以上，未做同机基准）：

- 两家的共同点：Ubuntu 原样二进制、内核直跑、不用 root、不做虚拟化。
- 差异在「路径翻译放在哪里」：它在每个 libc 函数入口加一层 wrapper（每次路径调用付几十到
  几百纳秒）；我们在**装载期改 GOT**（运行期路径调用不额外付费，代价是静态二进制完全够不着）。
  我们的 `stat ×20k` 比 proot 快 5.7×，与它的绝对数字不能直接比，但方向一致：这两家都比翻译层快。
- 它的 GPU 是空白，我们这边 PanVK + Zink 已经能让 GL renderer 报 `Mali-G77`（evidence 156）。

**值得参考的四条**（按价值排序）：

| # | 它怎么做的 | 我们拿过来做什么 | 价值 |
|---|---|---|---|
| 1 | 静态二进制首次启动前打补丁 | 补 `LIMITATIONS` 里「静态 ELF 完全不受控」这一格：至少把被 Android seccomp 拒绝的调用点改掉，让它能跑 | 高：这是我们现在最大的兼容性缺口 |
| 2 | 用信号处理器回答被 seccomp 拒绝的调用 | 我们目前只在 libc 里把 `clone3` 改成 `ENOSYS`；直接发 `clone3` 的程序仍会被杀。加一个 `SIGSYS` 兜底，按调用号回答 | 高：把「程序自杀」变成「退回旧路径」 |
| 3 | 夜间在模拟器上装一批包，逐项测、公开结果页 | 我们的 CI 还没接上（token 缺 `workflow`），接上后先做这条 | 中高：验收从「36 项门禁」扩到「包矩阵」 |
| 4 | 发布物带 `gh attestation` + `SHA256SUMS` | 我们已有 SHA256SUMS；加构建证明 | 中：供应链 |

**不学的两条**：它的 root mode（和我们的不变量冲突）；它的 X11 + CPU 桌面（我们这边
GPU 已经能算能画，回头做软件桌面是退步）。

## 二、「支持 Wayland 实现性能飞升」

**判断：对 syscall / 编译 / 包管理这些数字没有影响；对图形呈现链有真实收益，但不是"飞升"。**

- FACT（我们的基准）：`stat ×20k`、`open+close ×20k`、`make -j4` 这些差距来自 proot 的
  `ptrace` 往返，和 X11/Wayland 无关。
- FACT（evidence 97）：mutter 已经能以 Wayland 合成器身份在我们 guest 里跑起来
  （surfaceless + zink），ScreenCast / RemoteDesktop 接口都在。
- FACT（evidence 59）：`AHardwareBuffer` → dma-buf → `SurfaceControl` 的零拷贝呈现链已经
  10 000 帧长稳过。
- INFERENCE：X11 那条路每帧要多一次 SHM 拷贝和 X server 往返；换 Wayland 后客户端可以把
  dma-buf 直接交给合成器，合成器再把同一块 buffer 交给 `SurfaceControl`，拷贝次数下降。
  收益在**帧延迟与大分辨率下的 CPU 占用**，不在「程序跑得更快」。
- **真正的卡点不是 X11 vs Wayland，而是 GL 等级**：现在 zink 只给到 GLES 2.0 / GL 2.1
  （evidence 156），GNOME 46 的 Clutter/Cogl 要 GLES 3.0+ 或 GL 3.3+。这一级不解决，
  换成 Wayland 之后合成器照样是软件画。

所以顺序是：**先把 GL 等级抬上去 → 再换成 Wayland 合成器 + dma-buf 呈现 → 最后才谈数字**。
反过来做，只能得到「架构更干净，帧率没变」。

## 三、据此调整的下一步

1. 查 zink 为什么在 PanVK 上只报 GLES 2.0（缺哪个 VK feature / EGL 平台选择），能做就抬到 3.x。
2. 静态二进制补丁（学 Cortex 第 1 条），先做「打掉被 seccomp 拒绝的调用点」这一半。
3. `SIGSYS` 兜底处理器（学第 2 条）。
4. Wayland 合成器接 dma-buf 呈现（复用 evidence 59 的环 + evidence 97 的 mutter）。
5. CI 接上后加夜间包矩阵（学第 3 条）。

前两条和图形无关，能直接减少「装得上但跑不起来」的情况；第 3、4 条属于桌面那条线，
要等 GL 等级解决之后才有意义。
