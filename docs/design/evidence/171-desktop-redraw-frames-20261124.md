# 171 · 桌面在 ES3/zink 下真的在重绘：同一会话的四帧指纹

日期：2026-11-24 23:20（CST）。会话：`gnome-shell 46 --x11` on zink → PanVK，GPU 默认值来自
`/etc/profile.d/husk-gpu.sh`（evidence 167）。设备 RMX3357（MT6893 / Mali-G77 MC9）。

方法：`XGetImage` 取整个根窗口（2400×1080、ZPixmap、32 bpp），对像素缓冲取 sha256 前 16 位。
同一个会话内连续取四帧，中间只做一次输入注入（`xdotool key super` 开 GNOME 概览，
`Escape` 关）。探针是**设备侧一次性脚本**（python3 + ctypes，放 guest 的 `/tmp`，不入库）；
仓库里做同一件事的工具是 `gl_probe root`。

| 帧 | 时刻 | 动作 | 非黑 | 像素 sha256 前 16 位 |
|---|---|---|---:|---|
| A | T+0 | 空载 | 100.0% | `38c003c3de3e5984` |
| B | T+2 s | 空载（没有任何输入） | 100.0% | `38c003c3de3e5984`（与 A 逐字节相同） |
| C | T+2 s | `super` 开概览之后 | 100.0% | `3273f0567847d1f9`（与 A 不同） |
| D | T+2 s | `Escape` 关概览之后 | 100.0% | `e5c9772cef52d6f0`（与 A 不同） |

## 这四帧说明什么

1. **不是一张静态图。** 空载两帧完全相同（A == B），说明这个读回路径是确定性的、没有随机噪声；
   一次真实的桌面状态变化让整帧像素变了（C ≠ A），再变一次又换了一张（D ≠ A）。
   如果桌面是软件渲染的静态位图、或者 `XGetImage` 读的是陈旧缓冲，A/B/C/D 不会是这个关系。
2. **合成器在按状态重画。** 概览是 mutter 自己绘制的全屏 overlay；它能改变根窗口内容，
   说明 mutter 的合成路径（`cogl=gles2` + `MESA_GLES_VERSION_OVERRIDE=3.2` +
   `MESA_LOADER_DRIVER_OVERRIDE=zink`）确实把新帧交了出来，而不仅仅是"首帧能出"。
3. **D ≠ A 是预期的**：关掉概览后回到桌面，顶栏时钟等元素已经不同，所以不是"回到完全相同的一帧"。
   这不是缺陷，别拿 D == A 当验收条件。

## 这四帧**不能**说明什么

- 不是帧率：这里只有两次采样，间隔 2 s，量不出 fps。帧率数字在 evidence 168
  （640×480 `glClear`+swap：zink 131 fps vs llvmpipe 300 fps）。
- 不是"每个 swap 都被 X server 看见"：`gl_probe window` 的 60 次 swap 与这里的像素变化是
  **两条独立证据**，谁也不能替代谁。
- 没有做录屏逐帧对齐，所以"按键 → 屏幕像素"的那段延迟仍未测（evidence 168 §3 记的是
  "回车 → guest 进程读到"）。
