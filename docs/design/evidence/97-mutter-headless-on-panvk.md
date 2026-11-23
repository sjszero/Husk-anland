# evidence 97 — mutter 46 在 guest 里跑成 Wayland 合成器（GPU 走 PanVK）；ScreenCast / RemoteDesktop 都在

设备 RMX3357 / MT6893 / Mali-G77 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 环境（一次设定，后面都用这套）

```sh
R=/data/data/com.linuxhub.husk/files/rootfs-full
export XDG_RUNTIME_DIR=$R/tmp/xdg
export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus   # dbus-daemon --session 自建
export VK_ICD_FILENAMES=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
export VK_DRIVER_FILES=$VK_ICD_FILENAMES
export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink PANVK_KBASE_DVFS=none
export __EGL_VENDOR_LIBRARY_FILENAMES=$R/usr/share/glvnd/egl_vendor.d/50_mesa.json
```

## 结果（FACT）

```
$ sh gexec3.sh rootfs-full usr/bin/mutter --headless --no-x11
MESA: info: Found kbase device '/dev/mali0'.
MESA: info: panvk (Mali-G77 MC9) FristOneRR 1.2.0 (Mesa 26.3.0-devel)
MESA: info: panvk: gpu_id=0x90800011 variant=0x0 texture_features0=0xf7fffffe afbc=1 afrc=0
libmutter-Message: Created surfaceless renderer without GPU
libmutter-Message: Using Wayland display name 'wayland-0'
mutter-WARNING: Failed to acquire org.freedesktop.locale1 proxy
mutter-WARNING: Failed to connect to colord daemon
（进程持续存活，Wayland socket `wayland-0` 就绪）
```

三个必须知道的细节：

1. **`--headless --display-server` 是非法组合**（`meta-context-main.c:138` 拒绝），
   headless 只能跑成"nested"。
2. **必须有 session bus**，否则 `meta-wayland.c:621 set_gnome_env` 断言失败直接 `Bail out!`。
   `dbus-daemon --session --address=unix:path=$XDG_RUNTIME_DIR/bus` 可解（默认 `/tmp/dbus-*`
   在我们路径重写下失败，必须显式给 path）。
3. **`unable to initialize the Clutter backend: no available drivers found` 的解**就是上面的
   zink + PanVK 环境：mutter 拿到 EGL 后继续往下走。`without GPU` 这行是 mutter 在说
   "没有 DRM device 可用，走 surfaceless"，渲染仍是 zink→PanVK 的 Vulkan 路径（INFERENCE：
   日志里 panvk 已建 instance/device；GL renderer 字符串未单独取证）。

## D-Bus API（FACT）

```
$ gdbus introspect --session --dest org.gnome.Mutter.ScreenCast --object-path /org/gnome/Mutter/ScreenCast
（存在，完整 Properties/Introspectable + ScreenCast 接口）

$ gdbus introspect --session --dest org.gnome.Mutter.RemoteDesktop --object-path /org/gnome/Mutter/RemoteDesktop
  interface org.gnome.Mutter.RemoteDesktop {
      CreateSession(out o session_path);
      readonly i Version = 1;
  }

$ gdbus call ... --method org.gnome.Mutter.RemoteDesktop.CreateSession
(objectpath '/org/gnome/Mutter/RemoteDesktop/Session/u1',)
```

会话对象是**跟随 D-Bus 连接**的：gdbus 进程退出后再去 introspect 该路径已经没有对象
（INFERENCE，符合 mutter 的 session 生命周期；我们的客户端要在**同一连接**里
CreateSession → Start → ConnectToEIS/Notify*）。

## 对目标的含义

- 显示路线 B 的"合成器能不能在 guest 里跑起来"这一问：**能**，而且 GPU 是 Mali-G77 硬件。
- 输入路线 B 的"有没有标准注入接口"这一问：**有**（RemoteDesktop，mutter 46 原生）。
- 剩下的不是可行性问题，是工程量：ScreenCast 虚拟监视器 → dma-buf 帧交付；RemoteDesktop
  会话 → libei/EIS 输入；gnome-shell 作为客户端接上去。

## 复现时注意

- 每次实验后按名字精确清理：`pkill -u $(id -u) -f 'usr/bin/mutter'`、`pkill -u $(id -u) -f dbus-daemon`；
  不要 `rm -f` 任何东西。
- `mutter` 的库在 `usr/lib/aarch64-linux-gnu/mutter-14/`，用 `tools/device-tests/gexec.sh`。
