# 135 · 从原始 2.0 资产、干净目录完整重跑一遍初始化（门禁 A 的第一次干净复现）

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），
包：`com.linuxhub.myth.husk`，loader：`59b25f2c…688d549`（= 仓库 `tools/loader/ld-embedded.so`）。

## 0 · 结论（FACT）

用 APK 自带的 `assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar`（gzip，md5 见 130）
在**新目录**里装一份干净 rootfs，再按资产里的清单装包，最后 `dpkg --configure -a`：

```text
CONFIGURE_RC=0
dpkg -C → 无输出（rc=0）
status 里 Package 811 条，install ok installed 811 条，install ok unpacked 0 条
localedef --list-archive → zh_CN.utf8 / zh_SG.utf8
bash → 5.2.21(1)-release
id   → uid=0(root) gid=0(root) groups=0(root)
sigsys 真实 trap → 0 次
```

也就是说：**在这台机器、这套资产、这份 loader 上，全量 dpkg 配置第一次跑通，没有
SIGSYS/clone3、没有账号工具 argv[0] 错误、没有 libc 槽位错误。**
这不是 App 的 UI 流程（UI 仍未跑），也不改变「原 rootfs 已被手工 configure 过」的事实。

## 1 · 干净现场（不污染原数据）

```text
files/linux/distributions/ubuntu-24.04/rootfs     ← 原来的现场，不动
files/linux/husk-clean/rootfs                      ← 本次干净目录（2.2 GB，含 509 MB 包缓存）
```

装法（`tools/guest/install-husk.sh`，两遍解包补 SELinux 拒掉的硬链接）：

```text
== pass 1: extract (hard-link errors are expected) ==
   tar: can't link 'usr/bin/perl5.38.2' -> 'usr/bin/perl': Permission denied
   tar: can't link 'usr/bin/uncompress' -> 'usr/bin/gunzip': Permission denied
== pass 2: re-extract through the shim (fixes hard links) ==
   PRETTY_NAME="Ubuntu 24.04.4 LTS" / bins=294
   -rwxr-xr-x … /usr/bin/perl5.38.2
   -rwxr-xr-x … /usr/bin/uncompress
== ok: hard links materialised ==
```

随后把 2.0 侧的运行期组件补齐（等于 App 的 `ensureHuskRuntime()` 做的事）：
打过补丁的 libc（`32d4d3f9…`）、`husk-usermgr`、`husk-ldconfig`（`/usr/sbin/ldconfig`
本来就指向它）、以及 9 个账号工具的 wrapper。

## 2 · 装包（按资产清单，缺的就跳过并记账）

1354 条清单项（bootstrap 613 + gnome 751）里有 **165 条的 .deb 不在资产包里**，
例如 `bash-completion_2.11-8_all.deb`、`fonts-liberation_2.1.5-3_all.deb`、
`gettext-base_0.21-14ubuntu2_arm64.deb`。**原现场也没有这些包**
（`var/cache/doro-offline/packages` 729 个 deb，原始 rootfs 的 status 里同样没有
`fonts-liberation`、`bash-completion`、`gettext-base`）。本轮把 165 条跳过记账，
其余 0 条 unpack 失败。

> 这是 2.0 资产与清单不同步的问题（UNKNOWN：App 运行时是靠什么把这 165 条跳过的，
> 代码里没找到过滤，需下一步在 Kotlin 侧确认），不是 Husk 的缺陷。

## 3 · 配置期暴露的两个真问题（已修，见 133/134）

第一次 `dpkg --configure -a` 挂在这里，根因都在 133/134：

```text
locales …/locales.config: 377: awk: not found          ← 见 §4，是脚手架 PATH 造成的假象
libc-bin  /sbin/ldconfig.real: Can't create temporary cache file
          /etc/ld.so.cache~: Read-only file system
dbus-system-bus-common: groupadd: cannot lock /etc/group  ← 缺 husk-usermgr（脚手架漏装）
```

- `ldconfig.real` 是**静态链接**（`.dynsym` 只有 3 个符号、无 `NEEDED`），GOT 拦截对
  它完全无效，所以路径全部落到宿主 `/lib`、`/etc`：`/etc` 只读 → `EROFS`。
  这正是 `/usr/sbin/ldconfig → husk-ldconfig` 必须存在的原因；补上后 libc-bin 的
  trigger 输出 `husk-ldconfig: wrote /etc/ld.so.cache (656 entries from 5 directories)`。
- `groupadd: cannot lock /etc/group` 是 `husk-usermgr` 没装（本域 `link(2)` 被拒，
  影子工具自己的锁做不出来），装上 wrapper 后 dbus / pipewire / geoclue / polkit 全部配置成功。

## 4 · 脚手架教训（写下来免得再踩）

我这轮一开始把 `PATH` 写成了 `<rootfs>/usr/bin:…`（宿主绝对路径，旧会话的习惯写法）。
结果任何"经 /etc/alternatives 的符号链接"的程序都找不到：

```text
$ env awk
usr/bin/env: ‘awk’: No such file or directory
$ test -x <rootfs>/usr/bin/awk   → 失败
$ test -x /usr/bin/awk           → 成功
```

原因是 `usr/bin/awk → /etc/alternatives/awk → /usr/bin/mawk` 的中间目标是**绝对路径**，
内核按宿主根解析；而宿主侧 `access()` 又不认"已经带 rootfs 前缀"的路径。
**产品侧 `HuskRuntime.command()` 用的是 guest 口径 PATH**
（`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`），所以这不是产品缺陷；
改成 guest 口径 PATH 后，同一条 `--configure -a` 直接 `RC=0`。

顺带：这轮试过给 `search_guest_path` 加"宿主前缀也能 resolve"的补丁，**未验证到它解决任何问题，
已回退**——回退后重新构建的 loader 哈希仍是 `59b25f2c…`，与提交里的二进制逐字节一致。

## 5 · 仍未做（UNKNOWN，不要当成已通过）

- App UI 流程（`initialize()` + `ensureHuskRuntime()`）在干净目录上的整合未跑；
  干净目录是我用 loader 手驱的。
- 165 条清单缺包在 App 里怎么处理，未查。
- 桌面/会话（GNOME Shell 起会话）没在这棵干净树上跑；门禁 A 只验到 dpkg 全量配置。
- 16K/64K 真机、非 Mali GPU、传感器等维持原结论（UNKNOWN）。
