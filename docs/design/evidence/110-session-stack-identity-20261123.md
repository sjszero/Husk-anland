# 110 · 会话栈（dbus / pipewire / gnome-shell）的身份问题：fake root 与真实 uid 冲突

日期：2026-11-23　设备：RMX3357 / Android 13 / 4.19.191 / 4 KiB / `untrusted_app`

这条不是新功能，是**踩到的坑**，写下来是因为它会浪费一整天，而且结论影响产品默认值。

## 1. 会话总线拒绝 fake root（FACT）

```
dbus[20240]: Unable to set up transient service directory:
  XDG_RUNTIME_DIR "/tmp/xdg-frames" is owned by uid 10302, not our uid 0
```

`NR_FAKE_ROOT` 让 `getuid()` 报 0，但运行时目录在磁盘上属于 10302。dbus-daemon 会比对
这两者，不一致就拒绝建 transient service 目录，于是 bus 能连上却**永远不返回**任何回复。

所以**跑会话栈的进程必须用真实 id**：`NR_FAKE_ROOT=0`。
（这也顺带解释了为什么"同一个地址起两个 dbus-daemon"会让客户端挂死——evidence 101 记过，
这次又踩了一次：先前的 daemon 还在监听，新的连进去没人应答。）

## 2. GDBus 发不出凭据（FACT）

```
husk-rd: no session bus: Error sending credentials: Error sending message: Operation not permitted
```

`g_unix_connection_send_credentials()` 用 `SCM_CREDENTIALS` 把 uid/gid/pid 一起发出去，
内核会校验声称的 uid 必须等于进程的真实 uid。fake root 下它报 0、真实是 10302 →
**EPERM**。libdbus（gnome-shell 用的）不发 pid 因此不受影响，GDBus 受影响。

两条出路，暂未做：

- 会话栈用真实 id（本条采用）；
- shim 接管 `sendmsg`，把 `SCM_CREDENTIALS` 里的 uid/gid 换成真实值（内核反正要真值，
  这不是编数据，是把谎报的字段改回真值）。

## 3. 环境变量必须是**guest 路径**（FACT，浪费了半小时）

`run-as` 脚本里 `export DBUS_SESSION_BUS_ADDRESS=unix:path=$F/rootfs-full/tmp/xdg/bus`
把**宿主路径**交给了 guest 程序，shim 会把它当 guest 绝对路径再翻译一次 →
`<root>/data/data/.../bus`，然后 ENOENT。正确写法是 guest 路径：

```
DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/xdg/bus
XDG_RUNTIME_DIR=/tmp/xdg
```

宿主脚本自己用的路径（mkdir、读日志）才用 `$F/...`。两套路径必须分开写。

## 4. 现在的阻塞点：gnome-shell 46 在这次会话里崩了（FACT，未定位）

按上面三条修完，bus 与 pipewire 都健康（dbus 日志里 `Successfully activated service
'org.freedesktop.impl.portal.PermissionStore'`，pipewire 只有 rtk 警告），
gnome-shell 也能起到 `Using Wayland display name 'wayland-0'`，但随后：

```
(gnome-shell:20902): Gjs-CRITICAL **: Attempting to call back into JSAPI during the
  sweeping phase of GC ... The offending signal was destroy on Gjs_ui_iconGrid_BaseIcon
free(): invalid pointer
```

`free(): invalid pointer` —— 不是我们的 shim 打的，是进程内堆被破坏后 glibc abort。
证据 101 里它能起来，所以这次要么是环境（zink/PanVK + 本次的 runtime 目录）差异，
要么是 Gjs 的 GC 竞态在这个组合下必现。**下一个动作**：用同一个会话连跑 3 次确认是否可复现，
再取 `G_DEBUG=gc-friendly`、`JS_GC_ZEAL` 与 core 位置；起不来就退回"先不跑 shell、
只用 mutter headless 出 ScreenCast"（evidence 103 那条路已验证可行）。

## 5. 顺带做完的部署

- `husk-rd` 重新编译并部署到 guest（`usr/bin/husk-rd`），`sc <secs>` 模式会建会话并持有。
- `tools/guest-config/pipewire.conf` 落到 `/etc/pipewire/pipewire.conf`
  （`access.socket = unrestricted`，evidence 104 的修法）。
