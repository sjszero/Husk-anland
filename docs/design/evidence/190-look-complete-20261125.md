# 190 · 外观收口：主题/字体之外，把壁纸和「特权用户」提示也钉住

日期：2026-11-25 12:55–13:00（CST）。承接 evidence 184（GTK 主题 Adwaita → Yaru）。这一份补完
"看着不对"的另外两半：**壁纸**和**那句话**。

## 1 · 壁纸：也是 `ubuntu-settings` 的活

改之前（FACT，evidence 186 §6）：

```
org.gnome.desktop.background picture-uri   = file:///usr/share/backgrounds/gnome/adwaita-l.jpg
org.gnome.desktop.background picture-options = zoom
```

那是 GNOME 原厂那张蓝紫碎片图，不是 Ubuntu 的。修法同 184：写进同一份
`99-husk-look.gschema.override`，不用新机制。

新增的段（安装器会写进 rootfs 并 `glib-compile-schemas`）：

```ini
[org.gnome.desktop.background]
picture-uri='file:///usr/share/backgrounds/warty-final-ubuntu.png'
picture-uri-dark='file:///usr/share/backgrounds/warty-final-ubuntu.png'
picture-options='zoom'

[org.gnome.desktop.screensaver]
picture-uri='file:///usr/share/backgrounds/warty-final-ubuntu.png'
```

`warty-final-ubuntu.png` 是这棵树里自带的 Ubuntu 24.04 默认壁纸（酒红底 + 王冠纹章，
`190-warty-final-ubuntu.png`），不引入外部资源。

## 2 · 实测

```
glib-compile-schemas /usr/share/glib-2.0/schemas   → rc=0
picture-uri   = 'file:///usr/share/backgrounds/warty-final-ubuntu.png'
picture-options = 'zoom'
picture-uri-dark = 'file:///usr/share/backgrounds/warty-final-ubuntu.png'
gtk-theme     = 'Yaru'
```

注意：**已经 `gsettings set` 过的用户值不会被默认值覆盖**（override 只是默认值）。本轮先把
之前手改的几个键 `gsettings reset` 掉，值才回落到新默认。全新安装的树里没有这些 user 值，
不需要这一步。

图：`190-look-complete-wallpaper.png`（壁纸已经是王冠那张）。

## 3 · 只换 shell 不够：会出现"半张新壁纸"

用发布版加载器（`libld.so`，带 gpu-env 注入）跑 `gnome-shell --x11 --replace` 之后抓图
（`190-session-after-replace.png`）：屏幕上半是新的王冠壁纸，下半还是旧的蓝紫碎片。原因不是
壁纸没生效，而是 `--replace` 只换了 shell，会话里别的 actor（桌面图标窗口 `Desktop Icons 1`
这类）还是上一轮留下的进程，仍旧按老配置出画。

**FACT**：这套环境里 `--replace` 不等于"重开会话"。0.0.6 的验收必须做**整会话干净重启**
（把所有会话进程一起停掉再按应用的真实链路拉起来），否则抓到的永远是混合态。

## 4 · 顺带定性一件事：用户说的"安全模式"

整会话重启后，右上角立刻弹出一条系统通知：

```
System
Logged in as a privileged user.
Running a session as a privileged user should be avoided for security.
```

**FACT**：这条提示来自 gnome-shell 自己的"以 root 运行会话"检查，因为当前会话的 `HOME=/root`、
uid 是 0。它跟 GNOME 的"恢复模式/安全模式"不是一回事，但**用户看到的"一直进安全模式"很可能
就是这条提示 + Adwaita 白板**（184 修掉了后者）。

**JUDGMENT**：真正的修法是让会话跑在非 root 的 guest 用户上（树里已有
`provision-accounts.sh` / `husk-usermgr` 那套账户工具）。这是 0.0.6 之后"会话形态"这条线
的下一步，不是外观 override 能解决的。

## 5 · 0.0.6 该写进文档的口径

- 主题/字体/壁纸三项默认值：**已钉、已验**（184 + 190）。
- "安全模式"观感：**根因是会话以 root 跑**，提示来自 shell；彻底解决要切非 root 会话。
- `--replace` 不算重开会话；验收要整会话重启。
