# 286 · 第 3 轮收尾：符号链接深度 16→40、独立安装缺的三块、gate A 22/14 → 35/1

日期：2026-11-26（CST）。设备：UKHMMB8PYTGECMNV（RMX3357 / MT6893 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / 108 G 里剩 72 G）。包：`com.linuxhub.husk`
（firstInstallTime 2026-11-26 17:16:26）。rootfs：官方
`ubuntu-base-24.04.3-base-arm64.tar.gz`（29 865 086 B），loader：本仓库 `make loader` 产物。

本轮新编号缺陷：A-15（stock tarball 装不上）、A-16（独立安装缺 guest 工具替换）、
A-17（gate A 把旧包名写死）、A-18（usrmerge 链接被物化成空目录，`/bin/sh` 丢失）、
A-19（guest UDP 出站被设备策略拒绝，非 Husk 缺陷）。

---

## 1 · A-03 符号链接递归深度：16 → 40（Linux `MAXSYMLINKS`）

### 修前（FACT）

`runtime/crates/runtime-core/src/lib.rs` 四处 `depth > 16`，且 `resolve_at` 每跟一层链接
递归一次、每帧带 `gpath/raw/np/link` 四个 4096 字节栈数组 = 16 KiB/层。设备实测（旧 loader，
`tools/device-tests/path-probe`）：

```
PASS depth-final-16   PASS depth-mid-16
FAIL depth-final-17   FAIL depth-mid-17
FAIL depth-final-40   FAIL depth-mid-40
PASS depth-final-41   PASS depth-mid-41
Segmentation fault            ← 40 层链 + 64 KiB 线程栈
```

17 层起就 `ELOOP`（Linux 允许 40），且"把上限调到 40"会先把小栈线程打崩：**这是拿栈溢出
换 ELOOP**，所以必须先改结构。

### 修法（FACT）

1. `resolve_at` 改成迭代：所有 buffer 移到函数帧一次分配（5 × 4 KiB），链接跟随变成
   `continue 'outer` 重新从 root 走，栈占用与链接深度无关。
2. 新增 `pub const LINK_MAX: u32 = 40`；计数**只在真正跟随链接那一刻**检查
   （`resolve_at` 1 处 + `resolve_open`/`resolve_final`/`resolve_access` 各 1 处，
   共 5 处，外加 `resolve_open` 的 `Err(22)` 分支）。
   修前把检查放在循环顶部，会让第 40 层解析完却因为"下一轮"被判 `ELOOP`——第一版补丁就是这么错的，
   实测 `depth-final-40 FAIL`，已改。

### 修后（FACT，同一台设备、同一 probe）

```
PASS depth-final-16   PASS depth-mid-16
PASS depth-final-17   PASS depth-mid-17
PASS depth-final-40   PASS depth-mid-40
PASS depth-final-41   PASS depth-mid-41      ← 41 层 ELOOP，Linux 语义
PASS depth-40-on-64k-stack    errno=Some(0)
PASS depth-40-on-128k-stack   errno=Some(0)
PASS depth-40-on-256k-stack   errno=Some(0)
PASS depth-40-on-1024k-stack  errno=Some(0)
#checks=20 fail=0
```

复现：

```sh
./tools/device-tests/build-path-probe.sh
adb push tools/device-tests/path-probe /data/local/tmp/path-probe
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/path-probe \
  files/linux2/distributions/ubuntu-24.04/rootfs/tmp/path-probe'
adb shell 'run-as com.linuxhub.husk sh -c "cd files/linux2/distributions/ubuntu-24.04 &&
  ./husk /tmp/path-probe"'
```

---

## 2 · A-15 一个 stock Ubuntu tarball 根本装不上（pass 1 拒绝对目标为绝对路径的链接）

### 修前（FACT）

```
== pass 1: extract (hard-link errors are expected) ==
   tar: './etc/alsa/conf.d/60-a52-encoder.conf' /usr/share/alsa/alsa.conf.d/... not under '<DEST>'
   ... （同形的还有 /etc/alternatives/* -> /usr/bin/*、/etc/systemd/system/* -> /lib/...）
   tar: had errors
install-husk: pass 1 failed (tar rc=1); this archive is not usable
RC=68
```

ubnutu-base 24.04.3 里有 **552 条**绝对目标符号链接。宿主的 tar 会把链接目标按**宿主** `/`
解析，发现落在 `<DEST>` 之外就拒绝创建；安装器把这些当成致命错误，于是**任何正常发行版
tarball 都装不上**（这不是"某个包坏了"）。

### 修法（FACT）

pass 1 只容忍两类 entry：硬链接（SELinux 拒绝 `link(2)`）与"not under"（绝对目标链接）。
其余任何 `error|cannot|denied|corrupt|unexpected` 仍然 `exit 68`。这两类都由 pass 2 通过
shim 重建（在 guest 里 `/usr/...` 就在 root 内，tar 允许），再由步骤 2c 的 `husk-symlinks
--apply/--verify` 改写成相对链接。

### 修后（FACT）

```
== pass 1: extract (hard-link errors are expected) ==
   (pass 1 could not create hard links or absolute-target symlinks;
    pass 2 recreates those names through the shim)
== pass 2: re-extract through the shim (fixes hard links) ==
== symlinks: absolute -> relative ==
== libc ==
== ok: hard links materialised (perl5.38.2 + uncompress are real files) ==
== done ==
RC=0
```

---

## 3 · A-16 独立安装从来没装 `husk-usermgr` / `husk-su` / `husk-ldconfig`

### 修前（FACT）

`install-husk.sh` 里 `grep -n usermgr` = **0 命中**。这一步只存在于 App 的资产流程
（`docs/RUNNING.md` §1 的 2/4/5），tarball 安装路径完全没有，于是装出来的树里
`useradd/groupadd/passwd/su/ldconfig` 全是发行版原版：

```
groupadd: /etc/group.21094: lock file already used (nlink: 1)
groupadd: cannot lock /etc/group; try again later.
rc=10
useradd: group '1500' does not exist
rc=6
```

根因不是 `husk-usermgr`：`lckpwdf()` 用 `link(2)` 判断数据库占用，本 SELinux 域拒绝
`link(2)`。gate A 的 user management 8 项因此全红（PASS=22 FAIL=14）。

### 修法（FACT）

安装器新增 `2e. guest tools`：把 `husk-usermgr`/`husk-su`/`husk-ldconfig` 装进
`usr/sbin`，把九个账号工具与 `su`、`ldconfig` 换成指向它们的链接，**原文件一律保留为
`<name>.shadow` / `ldconfig.ubuntu`**；装完立刻断言这三个工具已经不是发行版原版
（否则 `exit 78`）。

### 修后（FACT）

```
== guest tools (useradd/groupadd/passwd/su/ldconfig) ==
   usr/sbin/useradd -> husk-usermgr (distro binary kept as useradd.shadow)
   ... usr/bin/su -> husk-su (distro binary kept as su.shadow)
   usr/sbin/ldconfig -> husk-ldconfig (distro binary kept as ldconfig.ubuntu)
   ok: useradd/groupadd/usermod/userdel/groupdel/chpasswd/passwd/chfn/chsh -> husk-usermgr
```

---

## 4 · gate A：同一棵 base rootfs，22/14 → 35/1

| 阶段 | PASS | FAIL | 说明 |
|---|---|---|---|
| 修前（旧 loader + 旧安装器） | 22 | 14 | 账号 8 项 + locale 2 + acl 3 + 包数 1 |
| 修 A-16 后 | 29 | 7 | 账号 8 项全绿 |
| 修 A-17 + 从**新发行的 tarball** 干净目录重装 | 30 | 6 | ps 泄漏项其实是脚本写死旧包名 |
| 装 `locales` + `acl` 并 `locale-gen zh_CN.UTF-8` | **35** | **1** | 只剩"包数 ≥ 700" |

剩下那 1 项是**内容**不是缺陷：`ubuntu-base` 只有 93 个包，发行用的桌面树才有 800+。
这条断言保留原样（它正是在区分"base 树"和"发行树"）。

### A-17（FACT，是门禁 bug 不是运行时 bug）

```sh
ps -e -o args= | grep -oE 'com\.[a-z0-9_.]+' | grep -v '^com\.linuxhub\.myth\.husk$'
```

排除的是**旧包名**，而现役包是 `com.linuxhub.husk`，于是我们自己的进程被算成"其他 App 泄漏"。
改成从 `HUSK_PKG` 取当前包名（旧名继续排除），并把命中的行打印出来，出现非 0 时能直接看图。

---

## 5 · A-18 usrmerge 链接被物化成空目录 → `/bin/sh` 不存在

历史树（`/tmp/rootfs-full.tar.gz`，776 MB，2026-11-22 21:54）解出来是：

```
/bin/   drwxr-xr-x 空目录        （应为 bin -> usr/bin）
/sbin/  drwxr-xr-x 空目录        （应为 sbin -> usr/sbin）
/lib/   drwxr-xr-x 普通目录       （应为 lib -> usr/lib）
```

后果：`/bin/sh` 不存在，`#!/bin/sh` 的脚本全废；`install-husk.sh` 的 pass 2 直接
`usr/bin/env: '/bin/sh': No such file or directory`，`rc2=127`。

复现与修复（FACT，本机执行后该树可启动）：

```sh
# 三个目录的内容并入 usr/ 后改成相对链接
( cd bin && tar cf - . ) | ( cd usr/bin && tar xpf - ); rm -rf bin; ln -s usr/bin bin
( cd sbin && tar cf - . ) | ( cd usr/sbin && tar xpf - ); rm -rf sbin; ln -s usr/sbin sbin
( cd lib && tar cf - . ) | ( cd usr/lib && tar xpf - ); rm -rf lib; ln -s usr/lib lib
```

修复后该树可跑：`uid=0(root)` / `PRETTY_NAME="Ubuntu 24.04.4 LTS"` / 99 个 gschema。
推断（INFERENCE）：物化发生在 pass 2——shim 为先到的成员建了父目录，随后那条
`bin -> usr/bin` 的链接 entry 因 EEXIST 失败，而 pass 1 早已把 `tar: had errors` 归入
"可容忍"。**这条推断尚未用一次受控安装证实**，所以本轮没有改 shim 的行为，只记录了复现和修复。

---

## 6 · A-19 guest 里 DNS 解析不出来（UNKNOWN，且不是 Husk 的错）

- `getent hosts ports.ubuntu.com` 在 guest 里对所有 nameserver 都返回空（8.8.8.8 /
  223.5.5.5 / 114.114.114.114 / 192.168.1.1，含 `options use-vc`）。
- 从 guest 里 `exec 3<>/dev/udp/8.8.8.8/53` 再写一包 DNS 查询：`write error: Operation not permitted`（EPERM）。
- 对照（FACT）：**不经过 Husk**，`run-as com.linuxhub.husk toybox nc -u 8.8.8.8 53` 同样是
  `nc: xwrite: Operation not permitted`；而 `adb shell` 域的同一命令正常。
- 结论：本机把**应用域 UDP 出站**挡住了，TCP 出站正常（`/dev/tcp/8.8.8.8/53` 通）。
  所以 `apt-get update` 失败是 ROM/网络策略，不是 loader。根因（谁来挡的：SELinux /
  netd / 运营商）**UNKNOWN**。

对发行的影响：`apt` 要能用在当前这台机器上只能走离线 deb（本轮就是这么装 `locales`/`acl`
的：`dpkg -i` + `locale-gen zh_CN.UTF-8` → `星期一`、`getfacl 2.3.2`）。

---

## 7 · 发行包的两处工程缺陷（已修）

1. **AppleDouble 混进 tar**：macOS bsdtar 把扩展属性写成 `._husk-su` 之类的伴生项，
   2026-11-26 的 `dist/husk-arm64.tar.gz` 里就有 4 个。`Makefile` 改成
   `COPYFILE_DISABLE=1 tar ...` 并加断言：包内出现 `._*` 直接失败。
2. **`husk-wallpaper-fit.sh` 位置不一致**：打包放在顶层（`husk-session.sh` 按
   `$BASE/husk-wallpaper-fit.sh` 找），安装器只认 `guest/` 下的位置，于是每装一次都打
   一条 WARNING 并少装一个文件。安装器改成两处都找；从新包重装后 WARNING 消失。

---

## 8 · 本轮之后 gate A 的失败项（照写，不美化）

| 项 | 值 | 性质 |
|---|---|---|
| installed packages | 93 < 700 | 内容：这是 base 树，不是发行桌面树 |
| Adreno/KGSL、16/64 KiB 真机、相机前后摄、GStreamer | 未跑 | UNVERIFIED，第 4/7/8 轮 |
| 桌面（GNOME/X11）在本轮的新树上 | 未装、未跑 | base 树没有桌面栈 |
