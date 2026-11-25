# 204 · 0.0.6 发版门禁 A：36/36，以及它当场抓到的一个宿主路径残骸

日期：2026-11-25 17:10–17:20（CST）。设备 RMX3357，发布树 HEAD 对应 `VERSION=0.0.6`。

## 1 · 门禁 A 结果（FACT）

```
DEV=UKHMMB8PYTGECMNV make gate-a
...
PASS=36 FAIL=0
```

覆盖：identity / bash / locale（含 zh_CN 三种）/ 用户管理 / dpkg / apt / 宿主路径泄漏 /
seccomp 真实 SIGSYS 计数。两条 NOTE 是预期内的：`apt-get -s` 需要网络索引（离线缓存里只有 deb），
以及 guest 可见的 argv 里有宿主应用目录（加载器自己的 argv，41 个进程，不是泄漏）。

## 2 · 第一次跑是 35/36，抓到的东西（FACT）

失败项：

```
FAIL  guest /data/user does not exist (want 'absent' got 'present')
```

宿主机上查这个"present"：

```
rootfs/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs
```

——**一条 14 层深、32 KB、只有空目录的链，正好等于把 `$ROOTFS` 的宿主绝对路径当成 guest 路径
往里 `mkdir -p` 了一次**（guest 侧看到 `/data/user/...`，shim 再加一次 rootfs 前缀）。
时间戳 `2026-11-25 16:17`，即 evidence 199/200 那一轮会话调试期间。

清掉后重跑：**36/36**（§1）。

## 3 · 成因（UNKNOWN，候选已列）

没能复现，也不打算靠猜。已排除/待查：

- `husk-wallpaper-fit.sh` 的 `g usr/bin/mkdir -p "$CACHE"` 用的是 guest 路径 `/root/.cache/husk`，
  不是宿主路径（FACT，读码）；
- `husk-desktop.sh` 的 `mkdir -p "$RT" "$ROOTFS/run/dbus"` 在宿主侧执行，不在 `g` 里（FACT）；
- 待查：设备上那份 `husk-wallpaper-fit.sh` 是否比仓库版本旧（旧版若用 `$ROOTFS` 拼 CACHE 就会正好造出这条链）。

结论按 UNKNOWN 记：**危害低（空目录、32 KB）、但判据明确**——门禁 A 的那条断言就是探测器，
以后再出现会被直接判 FAIL。谁再看到这条链，先 diff 设备上的看护脚本与仓库版本。

## 4 · 复现

```sh
DEV=<serial> make gate-a    # 期望 PASS=36 FAIL=0
# 若报 /data/user present：
run-as <pkg> sh -c 'find files/linux/distributions/ubuntu-24.04/rootfs/data -type d'
```
