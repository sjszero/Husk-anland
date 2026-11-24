# evidence 160 — 同一棵树里 llvmpipe 报 ES3.2、zink(Mali) 只报 ES2：上限在 zink 这一层

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB。
安装：`/data/local/tmp/husk-c4`（不装 APK，evidence 157）。
时间：2026-11-24（CST）。探针：`eglinfo`，同一份 rootfs、同一个
`libgallium-27.0.0-devel.so`。

## 1. 事实：同一份库、同一套 EGL，只换 gallium 驱动

| 驱动 | GL core | GL compat | GLSL | OpenGL ES |
|---|---|---|---|---|
| `GALLIUM_DRIVER=llvmpipe` | 4.6 Core | 4.6 Compat | 4.60 | 3.2 |
| 默认（zink on PanVK） | 无 core 上下文 | 2.1 | 1.20 | 2.0（ES GLSL 1.00） |

两行是同一台设备、同一份 `libEGL_mesa.so` + `libgallium-27.0.0-devel.so`、同一个
surfaceless 平台，唯一差别是选了哪个 gallium 驱动。

- **FACT**：这个 rootfs 的环境（EGL、glvnd、driconf、Mesa 构建、版本表）不缺东西——
  llvmpipe 能报 4.6 / 3.2。
- **FACT**：ES2 上限是 **zink + 这版 PanVK** 组合下 zink 的声明结果，不是运行时的限制。
- **FACT**：这棵树里 zink 就是 GL 的默认驱动（不设 `MESA_LOADER_DRIVER_OVERRIDE` 也是 zink），
  所以「不注入 env 就退回 llvmpipe 做对照」不成立；要对照必须显式
  `GALLIUM_DRIVER=llvmpipe`。

## 2. 试过并排除的机制（省得下次重走）

- 原假设「zink 没设 `caps->essl_feature_level`，而 `st_extensions.c:1229` 拿它当
  `ESSLVersion`」**不成立**：取同一版 Mesa 主干源码（`27.0.0-devel`，与 rootfs 里版本号一致）
  对照，`src/gallium/drivers/llvmpipe` 也**不设** `essl_feature_level`，照样报 ES 3.2。
  所以这个字段不是 ES 版本的开关，evidence 159 §3.2 的推论作废。
- ES 版本由 `src/mesa/main/version.c: compute_version_es2()` 决定，条件是一串扩展布尔量加
  `consts->MaxColorAttachments >= 4`（ES3.0），再加 `MaxVertexAttribStride >= 2048` 与 compute
  limits（ES3.1）。zink 侧具体哪一个为假仍然 **UNKNOWN**：`eglinfo` 的 ES 上下文只列 ES 名字的
  扩展，不能直接当 `extensions->ARB_*` 的判据。
- 要拿到确切那一条，需要这版 `libgallium` 的**构建来源**（FristOneRR 的 PanVK/Mesa 树），本机
  没有该源码；重新编一份 aarch64-linux 的 Mesa 属于另一档工作量，本轮没做。

## 3. 对路线的影响

- 「桌面上吃到 GPU」和「GL 等级」是两件事：ES2 足够 mutter 46 起合成器，但 **GTK4 的 ngl
  渲染器要 GL3.3 / ES3.0** → 现在会退回 cairo（CPU 画）。当前真实状态是「GPU 合成 + CPU
  画控件」，不是「整个桌面都在 GPU 上」。
- 下一步顺序：先验证「合成器真的跑在 zink 上」（X11 会话，读 mutter 的 renderer 行），再验证
  「override 到 ES3 后 GTK4 ngl 真的画对」。两条都成立才写「桌面原生吃到 GPU」。
  `MESA_GLES_VERSION_OVERRIDE` 仍然只作实验开关。
