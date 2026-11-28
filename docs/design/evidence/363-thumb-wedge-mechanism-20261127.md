# 363 · 缩略图黑楔子：机制被切成两半——采样侧（可绕）与内容侧（现场真凶）

日期：2026-11-27 22:10–22:35（CST）。设备 RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
2400×1080 横屏，应用 `com.linuxhub.myth.husk`，runtime `0.0.37`，X server `:0`，zink 会话。

承接 361（现场几何、与 ICD 无关、两次逐像素相同）。本份只回答一个问题：
**381 块黑楔子到底是"采样到了坏 mip"，还是"内容里就有一块黑"?** 结论是后者，而且是
用两个**可回滚的二进制补丁**做单变量实验证明的。

## 1 · 基线（FACT）

现场复现照 361 §1 的 6 步（`tools/device-tests/t-grid.sh` + `adb exec-out screencap -p`）。
本次同一台机器、同一会话重新抓：

| 采样框 `x 1330–1660, y 110–300` | 近黑像素 / 总数 | 占比 |
|---|---|---|
| 应用盘（楔子在场） | 9345 / 62700 | **0.1490** |
| 361 记录（同一台机） | 2352 / 15675 | 0.1500 |

逐行扫楔子左边界（每 12 行一个点，框 `x 872–1220, y 122–278`，归一化到缩略图宽）：

| y | 122 | 134 | 146 | 158 | 170 | 182 | 194 | 206 | 218 | 230 | 242 | 254 | 266 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 楔左边界 x | 1214 | 1200 | 1184 | 1168 | 1153 | 1139 | 1123 | **1108** | 1092 | 1113 | 1140 | 1166 | 1194 |
| 归一化 | .983 | .943 | .897 | .851 | .807 | .767 | .721 | **.678** | .632 | .693 | .770 | .845 | .925 |

与 361 §2 的表**逐点相同**（±1 px 是采样步长差）⇒ 确定性、可重复。

## 2 · 探针：`gl-probe scale-wedge` 能把楔子造出来（FACT）

`runtime/crates/gl-probe/src/main.rs::run_scale_wedge`：2400×1080 白底黑网格纹理，
按 16×8 单元统计暗格，`#`=单元平均亮度<40。

| 变体 | dark_cells | near_black | 附注 |
|---|---|---|---|
| `mip`（MIN=LINEAR_MIPMAP_LINEAR + `glGenerateMipmap`） | **18/128（全在右侧）** | 11721/54288 | ASCII 图呈**尖端向左的楔子**，顶点≈(11/16, 4/8)=(0.69,0.50) |
| `nomip`（MIN=LINEAR，不生成 mip） | 0 | 6259 | 只有网格线是黑的 |
| `nearest` | 0 | 7670 | — |
| `cpumips`（自己按 2×2 盒式算 mip 再 upload） | **0** | 2378 | 与 llvmpipe 一致 |
| `list`（GL_TRIANGLES，6 顶点） | 128（全黑） | 54288 | **探针自身问题**：list 变体 `mip=false` 却把 MIN 设成 MIPMAP 滤波器 → 纹理不完整 → GL 规定采样为黑。不是驱动 bug，别当证据用 |

⇒ 采样路径（LOD/导数/UV）没问题（`cpumips` 干净）；坏的是 **`glGenerateMipmap` 生成的各级内容**。

## 3 · 生成的 mip 各级本身就带楔子（FACT）

`scale-wedge dumpmips`（逐级挂 FBO 读回）：

| level | PanVK mean / near_black |
|---|---|
| 0 | 222 / 326016 of 2592000（网格线，干净） |
| 1 | **181 / 185758 of 648000** |
| 2 | 181 / 46314 of 162000 |
| 3 | 181 / 11610 of 40500 |
| 4–7 | 187–193，仍有黑 |

把 level 1（1200×540）和 level 2（600×270）画成 ASCII（`#`=平均亮度<40）：

```
level 1                                            level 2
.........................................................--+     -..-..-..-..-...-..-..-..-..-...-..-..-..-..-...-..-..-.-++#
-..-..-..-..-...-..-..-..-..-...-..-..-..-..-...-..-..--+###     -..-..-..-..-...-..-..-..-..-...-..-..-..-..-...-..--+######
.....................................................-+#####     -..-..-..-..-...-..-..-..-..-...-..-..-..-..-.-++###########
-..-..-..-..-...-..-..-..-..-...-..-..-..-..-...-..+########     ...（同形，顶点在 ~0.65 宽处，两臂直达右边缘上下角）
..............................................+#############
-..-..-..-..-...-..-..-..-..-...-..-..-..-.-+###############     形状与 361 现场**同一个楔子**
```

⇒ `glGenerateMipmap` 的每一级里都有一块**同形楔子**，缩略图恰好采样到它，所以看上去
"楔子是斜的"（它其实是 mip 内容里的一块，映射回屏幕就是斜边）。

## 4 · 实验 A：把 zink 的采样 LOD 钳到 0（可回滚二进制补丁）

**想法**：既然坏的是"采样到坏 mip"，那就让采样永远只用 level 0。

**补丁**（`libgallium-27.0.0-devel.so`，有完整符号表，`llvm-nm` 可直接定位）：

```
c2a7c0 <zink_create_sampler_state>
  c2aae8  str s0,[sp,#0x60]   ; sci.minLod            -> str wzr,[sp,#0x60]   0xB90063FF
  c2aaf4  str s0,[sp,#0x64]   ; sci.maxLod (min>=max) -> str wzr,[sp,#0x64]   0xB90067FF
  c2ad80  str s1,[sp,#0x60]   ; sci.minLod (NONE 分支)-> str wzr,[sp,#0x60]
  c2ad9c  str s0,[sp,#0x64]   ; sci.maxLod (min<max)  -> str wzr,[sp,#0x64]
```

`.text` vaddr == 文件偏移（readelf 核对），四处都是等长 4 字节替换，`llvm-objdump` 复核为
`str wzr, [sp, #0x60/0x64]`。原文件 md5 `3b1700a3611392d5839c566b52f9f7b6`，
补后 `d80ebb178b987d7d13b6854f1a86a46f`。

**探针结果（FACT）**：`scale-wedge 348x156 mip` 从 `dark_cells=18, near_black=11721`
变成 **`dark_cells=0, near_black=6259`**——与 `nomip` 完全一致。补丁确实生效。

**现场结果（FACT）**：应用盘楔子**一点没变**——近黑占比仍是 **0.1490**，逐行边界与 §1
的表逐点相同（图：`363-thumb-wedge-two-patches-20261127.png`）。

⇒ **现场的楔子不是"采样到坏 mip"**。采样侧这条线在这里到此为止。

## 5 · 实验 B：把 mutter 的窗口 mipmap 链关掉（同一个思路的第二个候选）

`meta_shaped_texture_set_create_mipmaps`（`libmutter-14.so.0.0.0` 的导出符号 `0xb2a80`）
整函数改成 `ret`（`0xD65F03C0`），窗口纹理就不再建 mip 链。
原 md5 `7bfaf344bdaf70b12bcfa126d4c8de68`，补后 `9b29d062e950dd0a3efea30bc22e61ae`。

**现场结果（FACT）**：还是 **0.1490**、几何逐点相同（与 §4 同一张图）。
⇒ 也不是"窗口 mipmap 链"。

两个补丁已全部**回滚**，设备 `/usr/lib/aarch64-linux-gnu/` 下两个文件的 md5 分别回到
`3b1700…`（libgallium）和 `7bfaf3…`（libmutter），备份副本已删除。

## 6 · 顺手查清的三件事（FACT）

1. **`COGL_DEBUG` 对 gnome-shell 无效**：会话 `maps` 里是
   `mutter-14/libmutter-cogl-14.so.0.0.0`（mutter 自带 Cogl），不是 `libcogl.so.20`；
   `libmutter*.so` 里没有 `COGL_DEBUG` / `no-auto-mipmap` 字符串。§7 之前设想的
   "`COGL_DEBUG=no-auto-mipmap` 绕法"**作废**，不用再试。
2. **会话真正用的栈**（`/proc/<gnome-shell>/maps`）：
   `libEGL_mesa.so.0.0.0` + `libGLESv2.so.2.1.0` + **`libgallium-27.0.0-devel.so`** +
   `libvulkan.so.1.3.275` + `libvulkan_panfrost_husk.so`，env 是
   `MESA_LOADER_DRIVER_OVERRIDE=zink` / `GALLIUM_DRIVER=zink` / `PANVK_DXVK_COMPAT=0` /
   `VK_ICD_FILENAMES=…husk_panvk_frist370_execva.json`（宿主钉的，H50）。
   ⇒ 以后要打补丁，打 `libgallium-27.0.0-devel.so` 和 `mutter-14/libmutter-*.so` 是会生效的。
3. **`GALLIUM_DRIVER=panfrost`（原生 gallium panfrost，绕开 Vulkan/zink）这条路走不通**：
   `EGL: failed to create dri2 screen` → `FAIL eglInitialize`（这份 Mesa 的 panfrost 没有
   kbase 后端，只有 DRM 后端）。已还原 `/etc/husk/gpu-env`。

## 7 · 现在的判定表

| 假设 | 单变量实验 | 结果 |
|---|---|---|
| 楔子 = 采样到坏 mip（LOD 选错级） | zink sampler LOD 钳 0 | **否**（现场不变） |
| 楔子 = mutter 窗口 mipmap 链 | `set_create_mipmaps` no-op | **否**（现场不变） |
| 楔子 = 缩略图**内容**在做那块 tile 时就已经缺一角 | —— | **剩下的唯一解释（INFERENCE）** |
| 楔子 = 图像内容 / 隐藏缩略图 | 361 §4 | 否 |
| 楔子 = 某个 ICD 副本 | 361 §7 | 否 |
| `glGenerateMipmap` 生成的 mip 是坏的（独立缺陷） | `scale-wedge` `mip` vs `cpumips` | **是**（探针 18/128 vs 0；任何直接吃 mip 的 GL 程序都会中招，Chrome/GTK4 首当其冲） |

形状上的一致性（INFERENCE，三处同族）：

| 证据 | 现场 | 形状 |
|---|---|---|
| 186 | GTK4 `GSK_RENDERER=ngl` 的圆角/阴影 | 三角形缺口 |
| 195 | 3840→2400 的壁纸 | 右端人字楔子，内容 ×0.32 |
| 361/363 | 工作区缩略图（2400×1080→348×156） | 右端楔子，纯黑 |
| 363 §3 | `glGenerateMipmap` 的每一级 | 同一个楔子 |

共同点：**都是一次"要画一个四边形/一小块表面"的 draw 里，某个三角形的那一条边/那个顶点
被画成了别的东西**（纯黑＝采样/写到边界外、×0.32＝第二次半透明黑盖上去）。

## 8 · 还没做（UNVERIFIED）

- 具体是哪一次 draw（哪个程序、哪套顶点/属性）出的错：**UNKNOWN**。要拿到它只有两条路：
  1. **GL 呼叫追踪**：把 rootfs 里的 `libGLESv2.so.2` 换成一个"全符号转发 + 只记 `glDraw*`/
     `glBlitFramebuffer`/`glViewport`/`glScissor`/顶点缓冲"的壳，跑一次应用盘，把 tile 那次
     draw 的状态冻下来，再在 `gl-probe` 里 1:1 重建。
  2. **PanVK 侧**：`.so` 有完整符号表（9800 个），`panvk_v7_CmdBlitImage2` → `vk_meta_blit_image2`
     的调用链已经在反汇编里看到了；但这次现场证明问题不在 blit，而在普通 draw 的顶点/属性
     路径上，范围更大。
- 产品侧可先做的**绕法**（不动驱动、只动我们自己的 guest 文件）：把应用盘的缩略图从
  "GPU 缩放 Clutter.Clone" 换成"CPU 预缩到 tile 尺寸的一张图"（与 195 壁纸同思路），
  这样 tile 里没有任何缩小采样，楔子无处出现。代价：缩略图变成静态帧。**未实现**。

## 9 · 图

- `363-thumb-wedge-baseline-20261127.png` — 基线（无补丁），近黑 0.1490
- `363-thumb-wedge-two-patches-20261127.png` — 两个补丁都在，**楔子一模一样**
