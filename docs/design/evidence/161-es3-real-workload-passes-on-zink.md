# evidence 161 — 真 ES3 workload 在 zink + PanVK 上跑对了（不是只清屏）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB。
安装：`/data/local/tmp/husk-c4`（不装 APK），会话由 `husk-session.sh` 起。
时间：2026-11-24（CST）。探针：`tools/device-tests/es3-workload.c`（guest 内 `gcc` 现编）。

## 1. 探针在测什么（FACT：代码如此）

不是清屏，是一个把 ES3 关键路径都用上的 workload：

| 特性 | 用法 |
|---|---|
| VAO | 显式建 VAO 再绑（ES3 强制） |
| 实例化 | 顶点属性 divisor=1，4 个实例各画一个四边形，铺满 16×16 目标 |
| std140 UBO | 片元着色器从 UBO 取 `vec4 color` 当乘数 |
| 纹理采样 | 2×2 `RGBA8`、`NEAREST`、`CLAMP_TO_EDGE` |
| MRT | FBO 两个颜色附件，`glDrawBuffers(2)`，`o1 = 1 - o0` |

判定：att0 里四种纹理色（红/绿/蓝/白各乘 0.5）**每种各出现 64 次**（4 实例 × 16 像素），
异常像素 0；att1 每个像素都等于 `1 - att0`；`glGetError()` 全程 0。

读回是**逐像素 1×1**——整块 `glReadPixels` 在这棵树上只写第一行+第一列（evidence 162），
那是读回缺陷，不能当成渲染错误。

## 2. 结果

| 配置 | ES3 config | 上下文 | 着色器 | MRT FBO | att0 | att1 |
|---|---|---|---|---|---|---|
| 默认（zink，无 override） | **没有** | 建不出（rc=2） | — | — | — | — |
| `MESA_GLES_VERSION_OVERRIDE=3.2`（zink/PanVK） | 有 | ES 3.2 / GLSL ES 3.20 | OK | 完整 | **0 异常，四色 64/64/64/64** | **256/256 等于 1−att0** |
| `GALLIUM_DRIVER=llvmpipe`（对照） | 有 | ES 3.2 / GLSL ES 3.20 | OK | 完整 | 同上 | 同上 |

- **FACT**：zink on PanVK 不只是「能建上下文、能清屏」——实例化、UBO、纹理采样、MRT
  在这台设备上**画对了，逐像素回读也一致**。
- **FACT**：默认仍然没有 ES3 的 config 位，所以不设 override 时连上下文都建不出来。
  上限确实在「声明」这一层，而不是「能力」这一层。
- **INFERENCE**：这排除了「抬上去之后画错」这一类最坏情况的一半；但**不等于** GTK4 的 ngl
  渲染器或 mutter 的合成路径能跑——它们要的特性面比这个 workload 大得多。

## 3. 所以现在能说什么、不能说什么

- 能说：GPU 计算 + 离屏 GL 绘制（含 ES3 特性）在这条链上是通的，且可复现（一条命令重跑）。
- 不能说：「桌面已经原生吃到 GPU」。还缺两次验收：
  1. `GSK_RENDERER=ngl` 的 GTK4 应用在 override 下真的画对（不是退到 cairo）；
  2. mutter 用 ES3 建上下文并稳定出帧 30 分钟。
- override 仍**不进发布配置**，只作实验开关（LIMITATIONS §7 同步改）。
