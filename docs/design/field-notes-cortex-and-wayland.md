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

## 二、「支持 Wayland 实现性能飞升」——**不换，继续 X11**

**结论（决定）：不换合成器/协议，继续用 X11。** 理由是算出来的，不是口味问题。

先量一帧拷贝的代价（FACT，2026-11-24，目标机 guest 内，单线程 memcpy，1080p RGBA）：

```
1080p RGBA copy: 8.3 MB per copy, 1.26 ms per copy, 6.60 GB/s
```

再数一下两条链各自要拷几次（INFERENCE，基于我们自己的实现）：

| 链路 | 每帧 CPU 拷贝 | 1080p 代价 |
|---|---|---|
| X11（现状）：客户端 → X server（MIT-SHM 共享段，X server 是我们自己的进程内实现）→ 合成 → 桥接进 AHB → SurfaceControl | 2 次（合成 1 次 + 桥接 1 次） | ≈ 2.5 ms |
| Wayland：客户端直接画进 dma-buf → 合成器当纹理采样（GPU）→ 输出到 AHB | 0～1 次（GPU blit） | ≈ 0～1.3 ms |

**所以 Wayland 的上限收益是每帧 1～2.5 ms，即 60 Hz 预算（16.7 ms）的 6%～15%。**
这是「省下两次全屏拷贝 + 少一次 X 协议往返」，不是「性能飞升」。而且：

- 换 Wayland 之后要跑 X11 应用就得再加 **Xwayland**，等于把刚拆掉的 X server 又装回来，只多不少。
- 真正的瓶颈不在这里：默认 GL 只有 ES2（见下一节），渲染本身落在 llvmpipe 上，一项就比拷贝贵一个量级。
- X11 这条链已经是**我们自己进程内的 X server**（evidence 148），没有额外进程、没有网络端口，本来就不是典型 X11 的重方案。

什么情况下值得回头再看 Wayland：**当帧变成瓶颈、且剖析显示拷贝占比最大时**。要触发这个条件，先把 GL 等级和渲染路径解决，再看 `top` 里到底是拷贝还是渲染。

## 三、GL 等级的实测（决定桌面能不能吃到 GPU 的关键）

FACT（2026-11-24，目标机，`/data/local/tmp` 的独立安装里）：

1. 默认（`MESA_LOADER_DRIVER_OVERRIDE=zink`）：renderer 是
   `zink Vulkan 1.3(Mali-G77 MC9)`，但只给 **GL 2.1 / GLES 2.0**；请求 ES3.0 上下文直接失败
   （`eglCreateContext` → `EGL_BAD_CONFIG 0x3005`）。
2. `MESA_GLES_VERSION_OVERRIDE=3.2`：ES3.2 上下文建得起来，64×64 FBO 清屏 + 回读像素正确
   （`pixel=3399e6ff`，`PASS`）。
3. `MESA_GL_VERSION_OVERRIDE=4.6`：拿到 GL 4.6 core 上下文，同样画得出、读得回
   （`pixel=3399e6ff`，`PASS`）。

也就是说：**GPU 有这个能力，是 Mesa 在默认状态下只报 ES2。** 但 override 是「声明」而不是
「修复」——真正要用，必须证明 ES3 特性在 PanVK 上真的实现正确，否则就是把「跑不起来」
换成「跑起来但画错」。

**下一步顺序**：先查默认为什么只报 ES2（Mesa 侧判定依据），能用真 workload（GLES3 应用）
验证后再决定是否默认开启；在此之前，`MESA_GLES_VERSION_OVERRIDE` 只作为实验开关，不进发布配置。

## 四、据此调整的下一步

1. 查 zink 在 PanVK 上默认只报 ES2 的判定依据（不是设备能力问题，能力已证）。
2. 静态二进制补丁（学 Cortex 第 1 条），先做「打掉被 seccomp 拒绝的调用点」这一半。
3. `SIGSYS` 兜底处理器（学第 2 条）。
4. 桌面继续 X11；只有实测证明「帧受限且拷贝占比最大」时才回头评估 Wayland。
5. CI 接上后加夜间包矩阵（学第 3 条）。
