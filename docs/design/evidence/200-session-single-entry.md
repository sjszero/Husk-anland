# 200 · 会话形态：单一入口、清残留、以及"安全模式/主题丢失"到底是什么

日期：2026-11-25 16:11–16:29（CST）。设备 RMX3357，`:0`（X server 在应用进程里，不动它），
发布版加载器 `libld.so`，`/etc/husk/gpu-env` zink。这一份对应用户反馈的两句话：
"一直进的都是安全模式，主题那些全丢失了" 和 "0.0.6 预备全不稳定"。

## 1 · 现象复现与实测值（FACT）

1. **扩展其实是被启用的。**
   ```
   disable-user-extensions = false
   enabled-extensions      = @as []
   gnome-extensions list --enabled = ubuntu-appindicators / ubuntu-dock / tiling-assistant / ding
   ```
   `enabled-extensions` 是空的，但会话模式的 json 里带了 `enabledExtensions`，四个扩展都真的在跑
   （截图里左侧 dock、桌面 Home/Trash 图标都在）。**"主题全丢"不是扩展没加载。**
2. **用户看到的那条"安全模式"是这条通知**（截图内可读）：
   ```
   Logged in as a privileged user
   Running a session as a privileged user should be avoided for security ...
   ```
   来源是 gnome-shell 检测到 `getuid() == 0`。
3. **`gnome-shell-disable-extensions` 这个标记文件真的是活的**：启动时 shell 会写它，
   若文件已存在就报 `Failed to create file ... File exists`：
   ```
   GNOME Shell-Message: 16:11:36.762: Failed to create file .../gnome-shell-disable-extensions:
       Error opening file ...: File exists
   ```
   手工 touch 它再整会话重启，四个扩展仍然全部启用 —— 在 `linuxhub-ubuntu-min` 这个模式下
   它没有把扩展关掉。所以它是"下次启动的提示"，不是"本次安全模式"的开关。
4. **真正让会话变烂的是残骸**：连续手起会话之后，同时存在多个 `colord` / `accounts-daemon` /
   `upowerd` / `dbus-daemon`，并且 `XDG_RUNTIME_DIR` 里的 `gvfsd` / `at-spi` / `bus` 是上一轮的，
   日志里就是：
   ```
   GVFS-WARNING: The peer-to-peer connection failed: Error binding to address
       (GUnixSocketAddress): Address already in use. Falling back to the session bus.
   ```

## 2 · 假身份这条路走不通（FACT，附原文）

想直接让 shell 以非 root 身份跑（`NR_FAKE_UID=1000`、`HOME=/home/husk`）：

```
$ sh xe-user.sh usr/bin/env DISPLAY=:0 id
uid=1000(husk) gid=1000(husk) groups=1000(husk)      # 假身份生效
$ ... 起 gnome-shell
Failed to connect to session bus: The connection is closed
$ grep 'Could not connect' shell.log
Gjs-CRITICAL: JS ERROR: Gio.IOErrorEnum: Could not connect: No such file or directory
** Message: Execution of main.js threw exception
```

原因是 dbus 的 `EXTERNAL` 认证比对的是**真实** uid：客户端声明 1000、服务器看到的是 Android
的 2xxxx，握手直接被关。所以"让 shell 以为自己是普通用户"这条路在本层不成立，
那条 privileged 提示按**已知提示**处理，不用假身份绕。

## 3 · 落地：`tools/guest/husk-desktop.sh`（单一入口）

之前桌面的启动流程只活在开发机的临时脚本里（就是"预备不稳定"的根）。现在收成一个入口：

1. 按 guest 程序名停残留（先 TERM，2 s 后 KILL）；只碰 guest 程序，绝不碰应用进程；
2. 清陈旧状态：`bus`、`bus.lock`、`gnome-shell-disable-extensions`、`run/dbus/*`、
   `gvfsd/`、`gvfs/`、`at-spi/`；
3. 起系统总线 → 起会话总线（每一步等 socket 真的出现，最多 20 s）；
4. 起 `gnome-shell --x11 --replace`（`linuxhub-ubuntu-min` 模式、zink 由 `gpu-env` 注入）；
5. 等 `_NET_SUPPORTING_WM_CHECK` 被认领（不是"进程在"），再自检并打印：

```
CHECK wm=yes
CHECK ding=yes
CHECK gpu=zink
husk-desktop: 会话已起（wm=yes ding=yes gpu=zink）
```

6. 最后拉起壁纸看护（evidence 195/199）。

真实的两个坑，都在代码注释里写死了：

- **dbus 地址必须用 guest 侧路径**。`XDG_RUNTIME_DIR` 和 `unix:path=` 一开始用的是宿主绝对路径，
  路径层会再加一次 rootfs 前缀，于是 shell 报 `Could not connect: No such file or directory`、
  `main.js` 直接抛异常退出。现在用 `RT_G=/tmp/xdg-husk`、`/run/dbus/system_bus_socket`。
- **后台进程必须收干净继承的 fd**。不这么做，调用方一句 `| tail` 就永远等不到 EOF
  （实测挂死 5 分钟以上）。现在后台启动的子 shell 先 `exec 0</dev/null 1>/dev/null 2>/dev/null`。

## 4 · 验收（FACT）

同一条命令跑两次（一次输出接管道），20 s 内返回：

```
CHECK wm=yes   CHECK ding=yes   CHECK gpu=zink   rc=0
```

跑完的现场计数（对比"手起会话"时动辄几十个）：

```
gnome-shell 进程 = 1
dbus-daemon     = 1
/usr/libexec/gsd-* = 1
```

整屏 2400x1080 取图：壁纸正确（沿用 evidence 199 的 1:1 预缩放图）、左侧 dock 在、
桌面 Home/Trash 图标在、顶部面板在，唯一多余的是第 1 节那条 privileged 提示。

## 5 · 还没做的

- privileged 提示没有消掉。要消它只有两条路：改 gnome-shell 内嵌 gresource 里的 main.js
  （JS 是压缩进二进制的，`grep` 找不到原文，得解包重打包），或者在预载层拦
  `Notifications.Notify` 丢掉这一条。两条都动的是"别人的代码"，0.0.6 先记录下来。
- `gnome-shell-disable-extensions` 由 shell 自己重新写，`rm -f` 只是保证"启动那一刻"没有它；
  实测它在场也不影响扩展加载，所以不追加处理。
