# 192 · 走 APP 那条链开会话：非 root、Yaru 在、GTK4 干净、Nautilus 有内容、窗口不再摆到屏外

日期：2026-11-25 13:33–13:41（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4 KiB 页），应用 `com.linuxhub.myth.husk`（`versionName=Myth-Prometheus`，APK 构建于 11-24 16:00）。
前面 185–191 全部是在**手工会话**里做的（`husk-session.sh` + `xe-plain.sh`），这份第一次走
**APP 自己的启动链**（`LinuxHubDesktopSession.start()` → `LinuxContainerManager.startDesktop()`），
用来确认"产品路径"到底是什么状态。

## 0 · 起点：APP 停在错误页（FACT）

`adb screencap` 拿到的是「GNOME 已退出，退出码 139」。139 = 128+11 = SIGSEGV。字符串来自
`LinuxHub-2.0/.../desktop/LinuxHubDesktopSession.kt:126`：

```kotlin
lastError = "GNOME 已退出，退出码 $code"
```

那一次是在**手工 session 用 `gnome-shell --x11 --replace` 抢了 APP 起的那个 shell**（`--replace`
会让原 shell 退出）之后出现的，所以最可能的解释是"被替换时死在退出路径上"（INFERENCE，未证）。
本轮重新从 APP 面板启动了一次，之后 12 分钟以上没有复现；这条继续按"待复现"挂着，不当已修。

顺带两个操作坑（对后面复现很关键）：
- APP 卡在错误页时，触摸被 X 的输入面截走，**点「返回 LinuxHub」没反应**；要 `am force-stop`
  再 `am start -n com.linuxhub.myth.husk/com.java.myapplication.MainActivity` 才能回到面板。
- 桌面没起来时屏幕是横的（2400x1080），APP 的竖排布局只占左边 1080 px，右下角那个"显示器"浮动
  按钮（显示器图标，实际坐标 ≈ `857,2160`）只在竖屏布局里找得到。

## 1 · APP 链起来的会话：桌面完整、无"特权"提示（FACT）

点浮动按钮后约 50 s 出桌面：Ubuntu 王冠壁纸、Home/Trash 图标、左侧 dash、顶栏时钟齐全，
**右上角没有 "Logged in as a privileged user."**。图：`192-app-session-desktop.png`。

进程侧证据（`/proc/<pid>/environ`，`run-as` 读出）：

```
USER=husk          HOME=/home/husk        LOGNAME=husk
XDG_RUNTIME_DIR=/tmp/runtime-husk
DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk
DISPLAY=:0         XDG_CURRENT_DESKTOP=ubuntu:GNOME
MESA_LOADER_DRIVER_OVERRIDE=swrast   GALLIUM_DRIVER=llvmpipe
```

三个结论：
1. **会话是 husk 跑的，不是 root** —— 190 里"以 root 跑会话"的提示在这条链上不会出现，
   那条提示只是手工 root 会话的产物（和 186 的"错位"一样，属于调试路径的副作用）。
2. 只有**一**个 `gnome-shell`（`usr/bin/gnome-shell --x11 --replace --mode=linuxhub-ubuntu-min`）
   和**一**个 DING（`-E -P /usr/share/gnome-shell/extensions/ding@rastersoft.com/app`，`-P` 非空），
   191 §4 里"shell 传空 `-P`"没有在这条链上复现 → 兜底脚本不是产品路径的必需品，但留着无害。
3. 这条链把 **shell 自己**钉在 llvmpipe/swrast（软件渲染）。桌面是能用的，但这与"桌面原生吃 GPU"
   不是一回事，得单独处理（不在这份证据的范围里）。

## 2 · 外观：Yaru 真的生效了，不再是"主题全丢"（FACT）

在**同一个会话总线**上读（`HOME=/home/husk`，`DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk`）：

| 键 | 值 |
|---|---|
| `gtk-theme` | `Yaru` |
| `icon-theme` | `Yaru` |
| `font-name` | `Ubuntu 11` |
| `color-scheme` | `prefer-light` |
| `background picture-uri` | `file:///usr/share/backgrounds/warty-final-ubuntu.png` |

override 文件确实落地：`usr/share/glib-2.0/schemas/99-husk-look.gschema.override`（1801 B，
2026-11-25 13:15），`usr/share/icons/Yaru/icon-theme.cache` 存在（187 KB），`Yaru/index.theme`
`Inherits=Humanity,hicolor`。

图标是**Yaru**不是 Adwaita，用配色比对确认（不是"看着像"）：

| 来源 | 取样 |
|---|---|
| `Yaru/48x48/places/folder-documents.png` 不透明像素均值 | `(125,109,111)`，顶部是洋红→橙色条 |
| 桌面截图上 `Documents` 磁贴 | `(217,113,119)` 那一带同色系 |
| `Desktop` 磁贴 | Yaru 的紫→橙渐变（`192-yaru-icon-tiles.png`） |

Adwaita 的文件夹是纯紫渐变、没有橙条，所以这一组只能来自 Yaru。

## 3 · GTK4 窗口：加载器注入的 `GSK_RENDERER=cairo` 在产品链上生效（FACT）

用**发布版加载器** `husk/libld.so` 在**这个 APP 会话里**起 `gnome-text-editor`，环境里故意不设
`GSK_RENDERER`，靠 `/etc/husk/gpu-env` 的默认值注入。窗口：`822x642+120+0`，
圆角、横幅、输入区全部干净，没有任何三角缺口。图：`192-gtk4-clean-app-session.png`。

187 的 10850 → 9 那个结论因此在**真实启动链**上复验了一次，不再只是手工会话里的数字。

`gpu-env` 现状（`rootfs/etc/husk/gpu-env`）：

```
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
VK_DRIVER_FILES=…同上
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
GSK_RENDERER=cairo
```

## 4 · Nautilus 内容区空白：只在 root 会话下出现（FACT）

同一棵树上，以 **husk** 身份起 `nautilus`：侧栏 + 内容区完整（Desktop/Documents/Downloads/
Music/Pictures/Public/Templates/Videos 八个磁贴都在）。图：`192-nautilus-husk-content.png`。

而以 **root** 起时它自己就说了原因（13:21 会话日志原文）：

```
** (org.gnome.Nautilus:15855): WARNING **: This app cannot work correctly if run as root
   (not even with sudo). Consider running `nautilus admin:/` instead.
```

所以 178 的"内容区空白"**不是路径层/nv12/gvfs 的问题**，是"Nautilus 46 拒绝以 root 正常工作"。
产品链本来就跑 husk，这一条随之消失；手工 root 会话要看文件就用 `admin:/`，别去动 shim。

顺带（同一份日志）两处噪音，都在产品链上自动消失或与本条无关：
`GVFS-WARNING … peer-to-peer connection failed … Falling back to the session bus`（gvfs 点对点回退，
能用）；`dbus-daemon … GeoClue2` 每分钟拉起一次（geoclue 空闲 60 s 自动退出，循环）。

## 5 · 窗口摆到屏幕外：mutter 的级联摆放，不是"错位"（FACT + 已定钉法）

在新会话里连开几个 GTK4 窗口，位置是这样的：

| 窗口 | 几何 | 结果 |
|---|---|---|
| `gnome-text-editor`（第一次） | `822x642+120+-21` | **标题栏在屏幕上方 21 px** |
| `gnome-calculator`（`center-new-windows=false`） | `482x610+120+520` | 底边 1130 > 1080，**下沿出界 50 px** |
| `gnome-calculator`（`center-new-windows=true`） | `482x610+992+271` | 完整在屏内 |
| `xterm`（对照，非 CSD） | frame `+14+49` / client `+66+69` | 正常 |

`xdotool windowmove 0x1000004 120 0` 之后窗口就老老实实待在 `+120+0`，说明**不是**谁在强制
偏移，就是**初始摆放**算错：mutter 默认按"级联"给新窗口挑位置（上一扇窗往右下错开），这个会话里
没有面板 strut、也没有它以为的工作区，错开几次就出界。

钉法（本轮已改）：`husk-look.override` 加

```
[org.gnome.mutter]
center-new-windows=true
```

实测同一台机器上从"出界"变成"`482x610+992+271` 全在屏内"。代价：新窗口不再级联错开，
一律居中——对手机屏幕来说这是想要的。

## 6 · `~/Desktop` 权限：DING 会拒绝"别人可写"的桌面（FACT + 已修）

`rootfs/root/Desktop` 是 `drwxrwxrwx`（`mkdir` 在 umask 0 下的产物），DING 日志原文：

```
DING: desktop-icons: Desktop is writable by others - will not allow launching any desktop files
```

`/home/husk/Desktop` 是 `755`，所以产品链不受影响；但手工会话会踩到。修法写进
`husk-session.sh`：`mkdir` 之后显式 `chmod 755`。

## 7 · 这份证据改变了什么

| 项 | 之前 | 现在 |
|---|---|---|
| 会话形态（①） | 手工 root 会话，带"privileged user"提示，靠猜 | APP 链非 root，Yaru/Ubuntu 字体/壁纸全在（FACT） |
| Nautilus 空白（③） | 复现不出，悬着 | 定根因：仅 root 会话；产品链正常（FACT） |
| 窗口摆放 | "出界/错位"混在一起 | 级联摆放，钉 `center-new-windows=true` 即落屏内（FACT） |
| GTK4 缺口 | 只在手工会话验过 | 产品链复验，干净（FACT） |
| `~/Desktop` | 777，DING 拒开文件 | `chmod 755`（已改脚本） |
| APP 退出码 139 | 不明 | 手工 `--replace` 抢 shell 后出现，重跑 14 min 未复现（UNKNOWN，挂着） |

## 8 · 还没做

- 139 要在 APP 里真复现一次才敢判"非 bug"；现在的口径是"未复现"。
- 会话 shell 仍是 llvmpipe；`zink` 只在手工会话里验过。桌面吃 GPU 是下一件事。
- 键盘/触控延迟（④）没测：探针窗口拿不到焦点的问题还在（189）。
