# 191 · 整会话重启的四个坑：系统总线、~/Desktop、swrast 只画一条带、DING 的 `-P` 为空

日期：2026-11-25 13:00–13:20（CST）。设备 RMX3357。目的是把"桌面会话正常"这件事真正验收，
结果把"重开会话"这条链上的四个坑逐个踩出来了。**注意：这篇记的是坑，不是完成**。

## 1 · 没有系统总线，shell 直接起不来（FACT）

停掉会话进程后第一次拉起，gnome-shell 立刻抛异常退出：

```
GNOME Shell-CRITICAL: LH_STAGE overview-init
accountsservice-WARNING: Couldn't connect to system bus: Could not connect: No such file or directory
Gjs-CRITICAL: JS ERROR: Gio.IOErrorEnum: Could not connect: No such file or directory
  _init@resource:///org/gnome/shell/ui/endSessionDialog.js:244
Execution of main.js threw exception
```

系统总线要用树里自带的那份配置拉起来，并给会话设 `DBUS_SYSTEM_BUS_ADDRESS`：

```sh
dbus-daemon --config-file=/usr/local/share/linuxhub/linuxhub-system-bus.conf \
            --nofork --address=unix:path=/run/dbus/system_bus_socket
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket
```

**FACT**：`$R/run/dbus/linuxhub-system-bus.pid` 是旧的、socket 不在，说明上一轮的会话是靠更早
那次手工启动的总线活着的；重启会话必须自己把总线拉起来。

## 2 · 没有 `~/Desktop`，桌面图标进程报错退出（FACT，已给修法）

```
DING: JS ERROR: Gio.IOErrorEnum: Error when getting information for file ?/root/Desktop?: No such file or directory
  DesktopManager@app/desktopManager.js:112
```

`mkdir -p ~/Desktop` 之后这条消失。**安装器应该建这个目录**（以及非 root 会话用户的同名目录）。

## 3 · shell 跑在 llvmpipe/swrast：整屏只有上方一条有内容（FACT）

| shell 的 GL | 抓图结果 |
|---|---|
| `GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=swrast` | 只有上方约 2400x470 有画面，其余**全黑** |
| `GALLIUM_DRIVER=zink`（PanVK） | 2400x1080 整屏正常 |

两次的 X 屏幕都是 `2400x1080`（`xrandr` 确认），窗口树里也没有遮挡窗口。
**INFERENCE**：swrast 这条路上 mutter 的背景/合成只提交了部分区域。
**对 0.0.6 的意义**：桌面必须跑 zink；把 shell 钉到 swrast 是"半屏黑"的直接原因。

## 4 · DING（桌面图标 + 壁纸）在会话里崩，手工跑不崩（FACT + 未定）

现象：shell 日志里每 10 秒一条 `Launching DING process`，进程起不来 → 没有图标，**壁纸也没有**
（壁纸是 DING 画的；DING 死了就只剩 `primary-color` 的纯色底）。

手工用同样环境跑同一个脚本，**只要把 `-P` 指对就不崩**：

```sh
gjs /usr/share/gnome-shell/extensions/ding@rastersoft.com/app/ding.js \
    -P /usr/share/gnome-shell/extensions/ding@rastersoft.com/app \
    -D '0:0:2400:1080:1:0:0:0:0:0' -M 0
```

不给 `-P` 时崩在这一行（`desktopManager.js:233`）：

```
Gtk.CssProviderError: <broken file>:1:0Failed to import: Error opening file /stylesheet.css
```

`ding.js` 里 `codePath = '.'`，只有 `-P <dir>` 才会被改；而 `build_filenamev([codePath,'stylesheet.css'])`
在 `codePath` 为空串时正好得到 `/stylesheet.css`。**INFERENCE**：会话拉起 DING 时传的 `-P` 是空
（即 `getCurrentExtension().path` 为空）。

排除过的：把 `GSK_RENDERER` 去掉、换三张壁纸（adwaita/原版 warty/缩放版）、去掉
`G_RESOURCE_OVERLAYS` —— **崩溃形态一模一样**，都跟这些无关。
**UNKNOWN**：为什么会话里 `getCurrentExtension().path` 是空的（扩展明明在磁盘上）。

## 5 · 现在的状态（诚实版）

- 会话能起来，顶栏、dash、窗口都正常；**壁纸和桌面图标没有**，因为 DING 没起来。
- 这不是 0.0.6 可以发布的状态：`桌面会话正常` 这条**不成立**。
- 只要 DING 起来，壁纸和图标就都回来（手工 `-P` 那次已经验证到它跑到 DBus 初始化那一步）。

## 6 · 下一步（顺序固定）

1. 定位 `getCurrentExtension().path` 为空：看 shell 是从磁盘还是从资源加载 DING 扩展（`gresource`
   里有没有同名 extension），必要时在会话脚本里改 `XDG_DATA_DIRS`/extension 目录，让 shell 走磁盘副本。
2. 会话脚本固化：系统总线 + `DBUS_SYSTEM_BUS_ADDRESS` + zink + `~/Desktop` 四件套。
3. 三条都绿了再抓"干净重启"的整屏图，作为 0.0.6 的 `桌面会话正常` 证据。
