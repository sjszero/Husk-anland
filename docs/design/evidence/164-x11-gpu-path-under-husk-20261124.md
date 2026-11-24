# 164 · X11 桌面的 GPU 通路：EGL+zink 通、GLX 不通、合成器仍崩一次

日期 2026-11-24 20:20–20:31 · RMX3357（MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB）
包 `com.linuxhub.myth.husk`（debug，`untrusted_app`），X server 是 App 进程内的 Lorie（`DISPLAY=:0`）。
所有 guest 进程都经 Husk loader（`files/linux/xe.sh` → `husk/libld.so`）启动，零 ptrace。

## 0 · 结论（先看这张）

| 问题 | 结果 | 级别 |
|---|---|---|
| X11 display 能不能连 | 能，`xdpyinfo` 正常，X.Org 21.1.99（Lorie） | FACT |
| GLX 客户端能不能用 GPU | **不能**：Lorie 没有 DRI3，GLX 无可用驱动 | FACT |
| EGL 客户端能不能用 GPU | **能**：X11 平台拿到 `zink Vulkan 1.3 (Mali-G77 MC9 (MESA_PANVK))` | FACT |
| 真 workload 在 X11 上能不能跑对 | **能**：ES3 workload 在 `EGL_PLATFORM=x11` 下 RESULT PASS，逐像素校验通过 | FACT |
| GNOME Shell 46 合成器在 Husk 下能不能起 | 软件（swrast）能起；**zink→PanVK 下 8 秒内 SIGSEGV** | FACT |
| 崩在哪一层 | 不是 GL 能力（同环境 ES3 workload 通过），疑在 mutter 的 EGL 呈现/WSI 路径 | INFERENCE，**未验证** |

一句话：**GPU 在 X11 上已经通了（EGL 这条路）；桌面合成器走 Mali 还差最后一段呈现路径。**

## 1 · X11 GLX：Lorie 没有 DRI3，这条路直接死（FACT）

```
$ sh xe.sh env DISPLAY=:0 usr/bin/glxinfo -B
screen 0 does not appear to be DRI3 capable
libGL: Warning in /usr/share/drirc.d/00-msm-defaults.conf ...: unknown device attribute: device_name
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
segv|sig=11|fa=0|pc=525198846016|lr=366856332416|x0=366856863872|x1=0|x2=366856328224|x8=0|ld=525220847616|lc=0
```

`segv|sig=11|` 是 Husk loader 自己的崩溃处理器打的（`runtime/crates/path-shim/src/lib.rs:888`），
所以这是 guest 进程在 GLX 初始化里真的挂掉，不是 shell 猜的。

而 mutter 用 GLX 时的表现同样是「没有驱动」：

```
$ COGL_DRIVER=gl ... usr/bin/gnome-shell --x11
libmutter-Message: Running GNOME Shell (using mutter 46.2) as a X11 window and compositing manager
Failed to setup: Unable to initialize the Clutter backend: no available drivers found.
```

**结论**：Lorie（termux-x11 的 X server）没实现 DRI3，X11 客户端拿不到直接渲染的 GLX 驱动。
本机 `LIBGL_ALWAYS_SOFTWARE=1` 时才走软件 GLX，这是 App 现在默认形态的来源。

## 2 · X11 EGL：能拿到 Mali（FACT）

```
$ sh xe.sh env DISPLAY=:0 usr/bin/eglinfo -B
MESA-EGL: warning: DRI3 error: Could not get DRI3 device
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
X11 platform:
EGL API version: 1.5
EGL vendor string: Mesa Project
OpenGL compatibility profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile version: OpenGL ES 2.0 Mesa 27.0.0-devel
```

DRI3 缺失只影响 GBM/dri2 screen（`egl: failed to create dri2 screen`），**X11 平台仍然起得来**，
而且是 zink→PanVK→Mali-G77 的 Vulkan 路径。

## 3 · 真 workload 在 X11 上跑对（FACT）

`tools/device-tests/es3-workload.c`（无头文件版，EGL/GL 全 `dlopen`）在设备上现编，跑：

```
$ sh xe.sh env EGL_PLATFORM=x11 DISPLAY=:0 GALLIUM_DRIVER=zink ... MESA_GLES_VERSION_OVERRIDE=3.2 /tmp/es3x
EGL 1.5
ES3 位 config: 有
ES3 上下文建立 OK
GL_VERSION=OpenGL ES 3.2 Mesa 27.0.0-devel | RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) | GLSL=OpenGL ES GLSL ES 3.20
GLSL ES 3.00 着色器编译+链接 OK
MRT(2 附件) FBO 完整 OK, drawBuffers 已设, err=0
阶段A 清屏回读: att0=64,128,191,255  att1=64,128,191,255 （期望 64,128,191,255）
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
```

同一份探针在 X11 平台上、不带 zink 环境时是 `RENDERER=llvmpipe`，`RESULT PASS`——两条都通过，
说明「X11 + EGL + Husk」这一层本身是好的，GPU 与软件都能跑对。

## 4 · 合成器：软件起得来，Mali 起不来（FACT）

四组，同一个脚本（`files/linux/husk-x11-gpu2.sh`，临时件，未入库），只换环境：

| # | GL 环境 | Cogl | 结果 |
|---|---|---|---|
| A2 | swrast/llvmpipe | 自动 | **起得来**：`DING` 启动、`GNOME nautilus 46.4`、DBus 接口就绪，桌面存活 |
| A | swrast/llvmpipe | 强制 `gles2` | 失败：`Trap`（SIGILL）+ Xlib 致命错误 |
| B | zink→PanVK | 自动 | **SIGSEGV**：`panvk ... FristOneRR 1.2.0` 建好设备后约 8 秒挂掉 |
| C | zink→PanVK | 强制 `gles2` | **SIGSEGV**，同上 |

A2 的日志（起得来那一组）：

```
GL=swrast cogl=auto jit_off=1 override=none
libmutter-Message: Running GNOME Shell (using mutter 46.2) as a X11 window and compositing manager
DING: GLib-GIO-Message: Adding GResources overlay ...
DING: DBus interface for Nautilus (org.freedesktop.DBus) is now available.
DING: GNOME nautilus 46.4
```

B 的日志（死的那一组）：

```
GL=zink cogl=auto jit_off=1 override=none
libmutter-Message: Running GNOME Shell (using mutter 46.2) as a X11 window and compositing manager
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA-EGL: warning: DRI3 error: Could not get DRI3 device
Segmentation fault
```

（`GJS_DISABLE_JIT=1` 全程开着，所以这不是 gjs JIT 与 shim 的信号处理器冲突那一条。）

## 5 · 这条断点在哪（INFERENCE，未验证）

- 同环境、同 loader、同 EGL 平台，**普通 GL 程序（es3-workload）在 zink 下通过**，
  只有 mutter 崩 → 差异在 mutter 特有的用法：对 X11 **窗口**建 EGL surface 并连续 present。
- Lorie 没有 DRI3，Vulkan WSI 拿不到 dmabuf 呈现路径；zink 在 X11 上的 present 会落到
  readback/SHM 兜底。**具体崩在 `eglSwapBuffers` 前的哪一步没有取证**——需要一份 aarch64 的
  运行期栈（本机没有 gdb，Android 侧也没有拿到 tombstone）。
- 因此下一步不是「再调 GL 版本」，而是：
  1. 写一个「EGL X11 窗口 + 每帧 swap」的最小探针，确认窗口化 present 就是崩点；
  2. 若确认，再看 panvk 的 `VK_KHR_xcb_surface` 与 Lorie 的 SHM 呈现能否配对；
  3. 都不行的话，给 Lorie 补 DRI3 才是根治（那条属于 X server 改动，不是运行时改动）。

## 6 · App 侧现状（对照用，不是本仓库的交付）

App 的 GNOME 会话环境是写死的软件路径（`GpuRuntimeEnvironment.gnomeSessionShell`）：

```
export MESA_LOADER_DRIVER_OVERRIDE=swrast GALLIUM_DRIVER=llvmpipe LIBGL_ALWAYS_SOFTWARE=1
export GJS_DISABLE_JIT=1
```

设备上 `files/linux/container.properties` 也是 `gpuProfile=software`。所以现在「桌面能用」是
软件合成的结果；要它吃 GPU，需要把这一段换成 zink 环境，**并且先解决本文件第 4 节的崩溃**。
