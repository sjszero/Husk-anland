# Anland 深读：Android 上跑 GNOME 的现成答案（2026-11-23）

> 用户要求：动手前先看别人做好的。这是对 `SuperTurtleDev/anland` 的源码级阅读记录。
> 固定版本：浅克隆 `d60a2a5`（2026-11-23 取），GPL-3.0，★362。
> 本文件只有仓库里能读到的东西；推断与判断单独标。

## 0. 结论先说

**anland 是目前最接近 Husk 目标的成熟实现**：它让 Android 上的普通应用把
Wayland 合成器（KWin / Mutter / Hyprland / Niri / mangowc / gamescope / weston 系）
的桌面，零拷贝送到 Android Surface，并且把**输入、相机、音频、剪贴板**都做成了
同一套 socket 协议。

三个直接改变我们设计的结论：

1. **输入不该走 `/dev/input` + libinput + udev**。anland 把鼠标/键盘/触摸直接注入
   合成器的 seat（Clutter 虚拟输入设备），端到端不需要内核输入节点。
   ⇒ 我们原计划的"evdev FIFO + libinput udev 数据"整条可以删掉。
2. **帧缓冲的所有权必须在 Android 侧（consumer）**：dmabuf、ready eventfd、shm 索引页、
   两个 socketpair 全部由 consumer 分配，producer 只往里画。这正是我们 evidence 73–79
   已经验证的形状，但 anland 把 ABI 定死并跑了几千帧。
3. **相机/音频用同一根 socket 传 fd 与控制消息**（`INPUT_TYPE_RESOURCE` + `SERVICE_TYPE_CAMERA`
   + SCM_RIGHTS），不需要第二个服务、不需要额外守护进程。

## 1. 角色与拓扑（`README.md`、`common/protocol.h`）

| 角色 | 二进制 | 职责 |
|---|---|---|
| daemon | `daemon` | rendezvous broker：最多 1 consumer + 1 producer，存屏幕信息，用 `SCM_RIGHTS` 传 fd |
| consumer | Android app / `test_sdl_consumer` | **资源所有者**：分配 dmabuf、buffer-ready eventfd、shm 索引页、data/fence 两个 socketpair，负责上屏 |
| producer | 各合成器后端 | 往 consumer 的 buffer 里渲染；收发输入/剪贴板 |

与 DroidNative 的差别：DroidNative 把合成器搬进 App（Kotlin + 自有 compositor）；
anland 让**guest 里的真合成器**渲染进 App 的 buffer。我们（GNOME/mutter）属于后者。

## 2. 协议 ABI（`common/protocol.h`，逐字）

```c
struct buf_info { uint32_t stride, width, height, format; uint64_t modifier; uint32_t offset; }

#define INPUT_TYPE_TOUCH 1 / KEY 2 / POINTER_MOTION 3 / POINTER_BUTTON 4 / POINTER_AXIS 5
#define INPUT_TYPE_TOUCH_FRAME 6 / DISPLAY_REFRESH 7 / CLIPBOARD 8 / TEXT_INPUT 9
#define INPUT_TYPE_ACTION 10 / RESOURCE 11 / RESOURCE_INVALID 12 / TABLET_* 13..16
#define SERVICE_TYPE_CAMERA 1
struct InputEvent {   /* 20 字节定长；剪贴板带变长尾部 */
  uint32_t type;
  union {
    struct { int32_t action; float x, y; int32_t pointer_id; } touch;
    struct { int32_t action; int32_t keycode; } key;              /* 内核 evdev 键码 */
    struct { float x, y, dx, dy; } pointer_motion;
    struct { uint32_t button; int32_t pressed; } pointer_button;  /* 内核 BTN_* 码 */
    struct { uint32_t axis; float value; int32_t discrete; } pointer_axis;
    struct { uint32_t refresh_mhz; } display;
    ...
  };
};
```
要点：坐标是 **float**（buffer 像素，producer 侧按逻辑分辨率缩放）；按键/按钮仍是
**Linux evdev 码**（不是 Android keycode），所以手势在 App 侧翻译时就要用 evdev 码。

## 3. Mutter 后端（`producers/gnome/anland_backend/mutter/src/backends/anland/`）

| 文件 | 行 | 内容 |
|---|---:|---|
| `meta-backend-anland.c` | 1991 | 整个 MetaBackend 实现：虚拟 DRM 设备（`libdisplay_producer/anland_device.h`）、输出/CRTC/虚拟监视器、渲染循环、事件源 |
| `meta-anland-input.c` | 1406 | **seat 注入**：指针/按键/滚轮/触摸/平板 + text-input（IME） |
| `meta-anland-camera.c` | 1036 | 相机：guest 侧 V4L2 请求 → consumer 回 fd |
| `meta-anland-audio.c` | 664 | PulseAudio sink/source 桥 |
| `meta-anland-clipboard.c` | 313 | 双向剪贴板 |
| `meta-anland-color.c` | 140 | 色彩/HDR 元数据 |

输入注入的真正落点（`meta-anland-input.c:666-760`）：

```c
meta_virtual_input_device_native_notify_absolute_relative_motion (
    META_VIRTUAL_INPUT_DEVICE_NATIVE (input->pointer), g_get_monotonic_time(), x, y, dx, dy);
clutter_virtual_input_device_notify_button (input->pointer, t, meta_evdev_button_to_clutter(button), state);
clutter_virtual_input_device_notify_scroll_continuous (...);
```
坐标换算见 `map_input_coordinates()`（buffer 像素 → 逻辑布局，含 layout.x/y 偏移）。

构建方式（`producers/gnome/Ubuntu2604_v5/build.sh` + `mutter.patch`）：
`debian/rules` 加 `-Danland=enabled -Dc_args=-DANLAND_MUTTER_50`，backend 源码整目录拷进
mutter 树；`startup.sh` 设 `ANLAND=1 ANLAND_SOCKET=… ANLAND_DRM_DEVICE=/dev/dri/renderD128
XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wayland-anland`，然后 `exec gnome-session`。

**版本事实**：backend 的 ifdef 只有 `ANLAND_MUTTER_49/50/51`；发行版配方是 Debian 13
（mutter 48/49）与 Ubuntu 26.04（mutter 50）。我们 2.0 的 24.04 rootfs 是 **mutter 46**
（`libmutter-14.so.0`，guest 内已核实）。⇒ 不能直接套。

**许可事实**：anland 是 GPL-3.0；Husk 仓库不是 GPL。把 anland 的 C 代码拷进 Husk 会
传染许可。⇒ **只在协议层对接**：guest 侧可以是 GPL 的 mutter 后端（单独分发、给源码），
Husk 的 Rust 侧只按协议说话。

## 4. 我们已有的东西对照

| 能力 | anland | Husk 现状 |
|---|---|---|
| consumer 持有 dmabuf ring | ✅ | ✅ 3-slot AHB→SurfaceControl，10,000 帧（evidence 59） |
| 显式 ready/acquire fence | eventfd + 真 `SYNC_FD` | ✅ 真 `SYNC_FD` 10,000/10,000（evidence 58） |
| guest 导入 App dmabuf | `MEM_IMPORT` + sticky | ✅（evidence 73） |
| 相机进 guest | 自定义 `SERVICE_TYPE_CAMERA` | ✅ 更强：guest 里就是真 `/dev/videoN` V4L2（evidence 91–94） |
| 音频 | PulseAudio sink/source 模块 | ✅ `module-pipe-sink/source` + AAudio（evidence 62/88） |
| 输入 | **Clutter 虚拟设备注入** | ❌ 卡在这里（原计划 evdev FIFO） |
| 合成器后端 | 7 家现成 | ❌ 无 |

## 5. 输入路线的重新定案

anland 证明了"合成器级注入"是可行且成熟的；DroidNative 证明了"App 内合成器"也可以。
两者都**没有**走 `/dev/input`。我们原方案里的 FIFO + udev 数据是自找麻烦
（libinput 要靠 udev 枚举设备、要 `/sys/class/input` 目录、要 EVIOCG* 查询）。

按桌面路线分两种落地，都是纯 Rust 客户端 + 标准接口：

| 桌面路线 | 输入注入 | 依据 |
|---|---|---|
| **GNOME on X11**（2.0 现役做法：App 内 X server + GNOME Shell 46） | **XTEST 协议**（`FakeInput`） | 2.0 `InputEventSender.java` 直调 server 侧 injector；XTEST 是同一机制的外部版本 |
| **mutter headless/Wayland**（≥46 支持 `--headless` + RemoteDesktop D-Bus + EIS） | **libei / EIS**（`org.gnome.Mutter.RemoteDesktop` 拿 ei fd） | mutter 46 源码 `src/backends/meta-eis*.c`，`meta-remote-desktop-session.c:1933` `meta_eis_new()` |

两条都不需要改 mutter、不需要 root、不需要 udev。`evdev` 只在 guest 侧需要"给老程序
看"的时候才做（例如某个程序直接读 `/dev/input/event*`），不属于主路径。

## 6. 下一步（本文件产出的行动项）

1. `hu-sk-input` 的 guest 侧输出从"24 字节 evdev 记录"改为"XTEST 请求"（先做，因为 X11 路线
   零补丁、与 2.0 同代），`--raw` 保留作诊断。
2. 桌面路线做一次 bounded spike：guest 内 `mutter --headless` 能否在 PanVK surfaceless 上起来，
   以及 RemoteDesktop D-Bus 能否拿到 ei fd。结论写 evidence。
3. 显示若走 mutter，需要 ScreenCast→dmabuf 或 backport anland 后端；这一项标 UNKNOWN，
   不先动手，等 spike 结论。
