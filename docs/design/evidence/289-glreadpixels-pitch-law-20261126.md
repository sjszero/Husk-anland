> **更正（evidence 314）**：本文的“16 字节行距定律”和“Mesa 打包层缺陷”结论作废。根因是探针常量错配：`GL_PACK_ALIGNMENT` 是 `0x0D05`，`GL_PACK_ROW_LENGTH` 是 `0x0D02`；旧探针把它写反，默认把 RowLength 设为 4。修正后整块读通过，不能再把逐行读取当作运行时缺陷。

# 289 · 整块 `glReadPixels` 的目的行距 = `align_up(每像素字节 × GL_PACK_ALIGNMENT, 4)`

日期：2026-11-26 20:05（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4.19.191 / 4 KiB 页），应用 `com.linuxhub.husk`，guest 树
`files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126`（Ubuntu 24.04，Mesa 25.2.8）。

evidence 162 记现象、evidence 179 量地址学（「目的行距恒 16 字节」）。但 179 的归属结论
只建立在「三处一致」上，而那三处跑的是**同一个 Rust 探针**。这一轮换了语言、换了加载路径
重做，把规律写成公式，并把能甩掉的嫌疑逐个甩掉。

探针：`tools/device-tests/probes/readback.py`（纯 ctypes，不依赖 Rust、不依赖 gl-probe，
`dlopen` 直接取 `libEGL.so.1` / `libGLESv2.so.2`）。

## 做法

1. `EGL_PLATFORM_SURFACELESS_MESA` + ES3 上下文，无窗口；
2. 源图每像素唯一：`R=x*7+3`、`G=y*11+5`、`B=(13x+17y)%251`、`A=255`，挂 FBO；
3. **真值用逐像素 `1×1` 读**（179 说这条路是好的）**并与上传的图案对账**
   —— 256/256 完全一致，所以「纹理上传坏了」不能冒充「读回坏了」；
4. 整块 `W×H` 读进预填 `0xAA` 的缓冲区，把每个被写的线性偏移反解回它装的是源图哪一格；
5. 另加四个对照：PBO 路径、Mesa 自己的 `eglGetProcAddress` 入口（绕开 libglvnd）、
   `GL_RGBA4` 目标（换每像素字节数）、同族另一个 Mesa 构建。

## 结果（FACT）

`16×16`、`GL_RGBA/GL_UNSIGNED_BYTE`、默认 `GL_PACK_ALIGNMENT=4`：

```
0=0,0 4=1,0 8=2,0 12=3,0 16=0,1 20=1,1 24=2,1 28=3,1
32=0,2 36=1,2 40=2,2 44=3,2 ...（每 16 字节换一行，每行只有 4 个像素）
256=4,15 ... 300=15,15（最后一行的剩余 12 个像素贴着写在后面）
```

整块读被写 76/256、值正确 4/256；**逐行 `W×1` 调 H 次 = 256/256 正确**。

把三个变量分开测，规律是死的：

| 变量 | 结果 |
|---|---|
| `GL_PACK_ALIGNMENT` = 1 / 4 / 8 | 目的行距 = **4 / 16 / 32** 字节 |
| `GL_PACK_ROW_LENGTH` = 0 / 16 / 32 | 目的行距恒 16 字节，**该参数完全无效** |
| 宽度 W = 1…40（H=3） | 目的行距**恒 16 字节**，只有 W=4 恰好「对」（16 = 4×4） |
| `GL_RGBA4` 目标（2 字节/像素） | ALIGN=1/4/8 → 行距 **4 / 8 / 16** 字节 |
| 读进 PBO 再 `glMapBufferRange` | 与直接读进客户端内存**完全相同** |
| 经 Mesa 自己的 `eglGetProcAddress` 取入口 | 与经 libglvnd **完全相同** |

于是：

```
目的行距(字节) = align_up(每像素字节数 × GL_PACK_ALIGNMENT, 4)
```

- 宽度不参与；`GL_PACK_ROW_LENGTH` 不参与；四舍五入的模数是常量 4（默认值），
  不是当前的 `GL_PACK_ALIGNMENT`（否则 RGBA4 + ALIGN=1 应得 2 而不是 4）。
- 默认 ALIGN=4、RGBA8 时就是 16 字节 —— 179 测到的那条。
- 等价说法：**打包层把 `GL_PACK_ALIGNMENT` 当成了行长度（`RowLength`），而它自己的
  `Alignment` 一直是默认 4。**

## 归属（FACT，逐条甩掉的嫌疑）

| 嫌疑 | 否掉它的证据 |
|---|---|
| Husk 的 path-shim / loader | 同一棵树里 proot + llvmpipe 结果一致；且缺陷只跟 `PACK_ALIGNMENT` 走 |
| Mali / PanVK / zink 后端 | llvmpipe（纯软件、不碰 `/dev/mali0`）完全一致 |
| libglvnd 分发层 | 经 `eglGetProcAddress` 直接拿 Mesa 自己的入口，行为一样；两个指针地址不同（`0x7f42b48100` vs `0x7f42c16f00`）但结果一样 |
| 树被改坏 | `libgallium-25.2.8-0ubuntu0.24.04.5.so` 的 sha256 `fbf08056…4013` **与 Ubuntu 官方 `mesa-libgallium_25.2.8-0ubuntu0.24.04.5_arm64.deb` 完全一致**（size 47 255 848 也一致） |
| 混进第二份 Mesa | `LD_DEBUG=files`：进程里只加载了 `libgallium-…0.24.04.5.so` 一份（残留的 `.4` 文件没被加载） |
| `/etc/ld.so.preload` 之类 | 不存在；这棵树里也没有 `/etc/husk/gpu-env` |
| 只是 `.5` 这次更新的回归 | 换成官方 `0ubuntu0.24.04.2` 构建（`libEGL_mesa` + `libgallium` 整套，`GL_VERSION` 回显 `…0.24.04.2` 确认生效）后**一模一样** |
| 调用方写错 | Rust 探针与 Python/ctypes 探针两套独立实现，结果逐字节相同 |

## 结论与当前边界

- **FACT**：缺陷在 guest 的 Ubuntu Mesa 读回/打包层，不在 Husk 的任何一层；这版树里
  「整块 `glReadPixels`」不可用，而 `glGetError()` 是 0。
- **FACT**：同一份 Ubuntu 25.2.8 源码（`src/mesa/main/image.c` 的
  `_mesa_image_row_stride()`、`src/mesa/main/readpix.c` 的 `read_rgba_pixels()`、
  `src/mesa/state_tracker/st_cb_readpixels.c`）按代码读，行距只能是 `width × bpp`，
  **不可能**是 `Alignment × bpp`。
- **UNKNOWN**：既然源码不可能、两个独立构建却都能复现，那么「二进制与源码不一致」的
  具体成因（构建/编译问题，还是我读的那条路径不是实际执行路径）**没定到文件和行**。
  下一步只有两条：把 `Alignment` 当 `width` 的调用点用调试构建打出来，或者直接改这几处源码
  重编一版 Mesa 对照。
- **UNVERIFIED**：默认帧缓冲 / X11 窗口表面的整块读没测；16 KiB / 64 KiB 页设备没测。

## 工程口径

- 我们自己所有读回（`gl-probe`、截图、像素比对）**一律逐行 `W×1` 读**，不要用整块读，
  也不要用 `1×H` 单列读（proot 下会写到缓冲区外，179 记的 180 字节）。
- 这条写进 `docs/LIMITATIONS.md`，并且**不许**把「整块读坏了」说成「渲染没出来」。
