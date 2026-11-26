# 278 · GNOME 与桌面应用的全局 GPU 现场核对

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / arm64。

## 0 · 结论

FACT：当前 GNOME 会话由 `gnome-shell` PID 28216 持有，其 `/proc/<pid>/environ` 包含：

```text
GALLIUM_DRIVER=zink
MESA_LOADER_DRIVER_OVERRIDE=zink
GSK_RENDERER=cairo
```

FACT：`VK_ICD_FILENAMES` 不在 `/proc/28216/environ` 里，不代表 Vulkan 没有生效。loader 的
`/etc/husk/gpu-env` 填空默认值通过 `getenv` 提供，不会反写进程的 environ（evidence 187）。
真实状态要看 fd 和 maps：

```text
10 -> /dev/mali0
11 -> /dev/mali0
... libvulkan_panfrost_x11.so
... libvulkan.so.1.3.275
```

FACT：当前 Nautilus 进程 PID 29077 也同时满足同样的三项：zink 环境、两个
`/dev/mali0` fd、已加载的 `libvulkan_panfrost_x11.so`。这不是只看 GNOME 主进程的一次性
状态，而是桌面启动的普通应用进程实际继承了同一套硬件运行时。

FACT：新增 `tools/device-tests/15-gpu-global-audit.sh` 并在真机运行，输出：

```text
STATE gnome-shell pid=28216 gpu=zink gsk=cairo
PASS  gnome-shell uses zink + hardware Vulkan + device fd
PASS  glxinfo saw zink Vulkan 1.3(Mali-G77)
STATE nested-gtk pid=11463 gpu=zink gsk=cairo
PASS  nested-gtk uses zink + hardware Vulkan + device fd
GPU_GLOBAL_AUDIT PASS=3 FAIL=0
```

`glxinfo -B` 是不带 GPU 变量的普通 GL 客户端，loader 仍然给出：

```text
Device: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
Accelerated: yes
```

嵌套应用是通过 guest loader 新起的 `gnome-text-editor`，不是读取已有进程的环境变量。

FACT：截图是 2400×1080，截图里的 Nautilus 窗口、dock、桌面图标和 GNOME 顶栏完整出现，
没有安全模式标记。截图见
[`../../assets/evidence/278-husk-desktop-zink-20261126.png`](../../assets/evidence/278-husk-desktop-zink-20261126.png)。

## 1 · 这次改了什么

FACT：`husk-desktop.sh` 的 `CHECK gpu=` 不再只匹配日志里的 `zink`/`PanVK` 字符串，也不再要求
`VK_ICD_FILENAMES` 出现在 `/proc/<pid>/environ`。现在的硬条件是：

- `GALLIUM_DRIVER=zink`；
- 进程 fd 表里持有 `/dev/mali0` 或 `/dev/kgsl-3d0`；
- `/proc/<pid>/maps` 里有已加载的 PanVK/Freedreno 硬件 Vulkan 后端。

只满足其中一部分时输出 `zink-partial`、`llvmpipe` 或 `unknown`，不把日志字符串升级成
“已用 GPU”的结论。

FACT：新增 `make gpu-audit`。它先检查已有 `gnome-shell`，再通过 loader 嵌套启动
`gnome-text-editor`，分别检查两者的环境、设备 fd 和 Vulkan 后端；不依赖 `gl-probe`，也不
把应用自己打印的 renderer 文本当作唯一证据。

## 2 · 验证

FACT：手动读取结果：

```text
pid=28216
GALLIUM_DRIVER=zink
MESA_LOADER_DRIVER_OVERRIDE=zink
GSK_RENDERER=cairo
mali0_fd_count=2
maps=libvulkan_panfrost_x11.so
```

```text
pid=29077 (nautilus)
GALLIUM_DRIVER=zink
MESA_LOADER_DRIVER_OVERRIDE=zink
GSK_RENDERER=cairo
mali0_fd_count=2
maps=libvulkan_panfrost_x11.so
```

FACT：`sh -n tools/guest/husk-desktop.sh`、`sh -n tools/device-tests/15-gpu-global-audit.sh`
和 `git diff --check` 通过；`DEV=UKHMMB8PYTGECMNV make gpu-audit` 返回 `PASS=3 FAIL=0`。

## 3 · 边界

- FACT：这只证明 Mali-G77 上 GNOME 与桌面启动的普通应用进程共享 zink/PanVK 和 `/dev/mali0`。
- FACT：当前 `GSK_RENDERER=cairo` 是已知的 GTK4 圆角裁剪规避，意味着 GTK4 界面绘制仍走 CPU；
  Vulkan/GL 程序和桌面合成器的 GPU 路径不等于 GTK UI 渲染器已经 GPU 化。
- FACT：X11 窗口内容到 X server 仍不是零拷贝；DRI3 在这台设备布局下不可用，回读和
  SHM 拷贝代价仍在（evidence 174/187）。
- UNKNOWN：其他应用是否都命中同一 GPU 路径；应用可以自己覆盖 `HUSK_GPU`、
  `LIBGL_ALWAYS_SOFTWARE` 或 `GSK_RENDERER`，这时应看进程自身证据。
- UNKNOWN：Adreno、其他 Android 版本和 16 KiB/64 KiB 真机上的同一结果。
