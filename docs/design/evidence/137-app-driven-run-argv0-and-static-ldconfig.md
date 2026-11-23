# 137 · 走 App 的 UI 流程才暴露的两个根因：静态 `ldconfig.real`、符号链接丢掉的 `argv[0]`

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
2.0 debug APK（`applicationIdSuffix .husk`），rootfs：App 自己从
`assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar` 解出来的那棵。

## 0 · 结论（FACT）

135/136 都是**手驱**的（`run-as` + loader），全绿；换成 App 自己的 `initialize()` 之后
第一次真跑就死在两条 135/136 根本没有覆盖到的路上：

```text
①  App 的第一条 guest 命令就死：mesa-gpu-runtime → "Bad system call"（SIGSYS, 159）
    根因：/usr/sbin/ldconfig 是 Ubuntu 的 shell wrapper → /sbin/ldconfig.real
          是**静态** glibc 二进制，装不进 shim，在 untrusted_app 域里被 seccomp 打死。
    修法（2.0 侧）：把 husk-ldconfig 随 APK 一起发，`/usr/sbin/ldconfig` 指过去。

②  过了 ① 之后，全量 unpack（811 包）成功，但 `--configure -a` 挂在 geoclue 上：
    dpkg-statoverride: error: group 'geoclue' does not exist
    根因：`/usr/sbin/addgroup` 是指向 `adduser` 的符号链接，而我们的 execve 把程序名
          换成了**解析后的目标路径**，perl 的 $0 变成 /usr/sbin/adduser →
          `addgroup --system X` 创建的是**用户**不是组。
    修法（Husk 侧）：dynamic ELF 用加载器的 `--argv0` 保留调用方写的 argv[0]；
          `#!` 脚本把**调用时用的路径**交给解释器（内核就是这么做的）。
```

两处都改完、各自上机验证；在 App 那棵树上 `dpkg --configure -a` → `RC=0`，`dpkg -C` 空。

## 1 · 为什么 135/136 没抓到

135/136 的设备命令都是 `adb shell run-as <pkg> sh -c '…'`：**SELinux 域切到 app 了，
seccomp 过滤器还是 adb shell 那套**。App 自己是 `untrusted_app` 的过滤器。
同一条命令，两种过滤器下行为不同：

```text
# run-as（shell 过滤器）
$ /sbin/ldconfig.real
/sbin/ldconfig.real: Can't create temporary cache file /etc/ld.so.cache~: Read-only file system
$ echo $?      → 1        # 活着，只是写不了宿主 /etc

# App（untrusted_app 过滤器，07:31 那次）
[mesa-gpu-runtime] Bad system call        # 159，进程被 SIGSYS 打死
```

同一份 loader、同一份 rootfs、同一条命令。**INFERENCE**（未单独比对两份过滤器内容）：
差异来自 seccomp 过滤器本身，而不是路径层——证据是那次的 `tmp/nr-sigsys.log` 里
只有 5 条 `nr-init|…` 记录、**没有一条 `sigsys|nr=`**，说明死掉的那个进程**根本没装过
我们的 SIGSYS handler**（静态二进制、不过 shim）。这条链条后面被 ① 的修法正面证实。

> 教训写进纪律：**门禁 A 必须在 App 进程里量**，`run-as` 的结果不能当 App 的结果。

## 2 · 根因 ①：静态 `ldconfig.real` 在 untrusted_app 里必死

App 的树里：

```text
$ ls -l /usr/sbin/ldconfig /sbin/ldconfig.real
-rwxr-xr-x  387      /usr/sbin/ldconfig          # Ubuntu 的 sh 包装：exec /sbin/ldconfig.real
-rwxr-xr-x  865840   /sbin/ldconfig.real        # 静态链接（无 PT_INTERP）
```

静态 = 没有 GOT 可补 = shim 完全进不去（135 §3 已经从另一个角度记过这条）。
App 的第一条 guest 命令里就有 `ldconfig`，于是整个 `EXTRACT_SYSTEM` 直接失败——
**在装任何一个包之前**。

修法（2.0 侧，`HuskRuntime.ensureInstalled()`）：

- APK 里加 `assets/husk/arm64-v8a/husk-ldconfig`（`35a9c2a8…`，由
  `tools/guest-config/build-guest-tools.sh` 可复现构建）；
- 装到 `<rootfs>/usr/sbin/husk-ldconfig`；
- `/usr/sbin/ldconfig`（以及 `/sbin` 若真是独立目录）改写成
  `#!/bin/sh` + `exec /usr/sbin/husk-ldconfig "$@"`，发行版原文件留作 `ldconfig.ubuntu`；
- `ASSET_REVISION` → `husk-runtime-spawn-link-5`。

结果（07:38）：

```text
assets.version = husk-runtime-spawn-link-5
-DONE EXTRACT_SYSTEM          ← 之前死在这里
[gnome-packages] husk-ldconfig: wrote /etc/ld.so.cache (522 entries from 5 directories)
```

## 3 · 根因 ②：符号链接分派丢了 argv[0]

过了 ① 之后 811 个包全部 unpack 成功，`--configure -a` 死在：

```text
Setting up geoclue-2.0 (2.7.0-3ubuntu7) ...
dpkg-statoverride: error: group 'geoclue' does not exist
dpkg: error processing package geoclue-2.0 (--configure): … exit status 2
```

geoclue 的 postinst 是标准写法：

```sh
if ! getent group geoclue >/dev/null; then addgroup --quiet --system geoclue; fi
if ! getent passwd geoclue >/dev/null; then adduser --quiet --system --ingroup geoclue …; fi
dpkg-statoverride --update --add geoclue geoclue 0755 /var/lib/geoclue
```

`getent` 本身没问题（`getent group nosuchgrpxyz` → rc=2，实测）。问题是 `addgroup`：

```text
$ ls -l /usr/sbin/addgroup
lrwxrwxrwx  addgroup -> adduser          # Debian 的符号链接分派
$ addgroup --quiet --system probegrp9; echo $?
0
$ grep -c probegrp9 /etc/group /etc/passwd
/etc/group:0      /etc/passwd:1          # 建出来的是**用户**，不是组
```

根因在 `execve` 的 interpose（`runtime/crates/path-shim/src/lib.rs`）：嵌套 exec 拼出来的
加载器参数是

```text
libld.so --library-path <libs> <解析后的真实路径> <调用方的 argv[1..]>
```

`argv[0]` 被整个丢掉，程序看到的 argv[0] 是**解析后的目标路径**。对 `#!` 脚本更糟：
交给解释器的是 `<root>/usr/sbin/adduser`，于是 perl 的 `$0` 是 `…/adduser`，
adduser 就按"加用户"跑。这正是"账号工具 argv[0]"这一类问题的另一面。

修法：

1. dynamic ELF：加 `--argv0 <调用方写的 argv[0]>`（glibc 2.33+ 的正式选项，本机 loader 实测
   `--argv0 myname bash -c 'echo $0'` → `myname`）。
2. `#!` 脚本：解释器收到的脚本参数改成**调用时用的那个路径**（guest 绝对路径），
   与内核语义一致，符号链接分派因此成立。
3. `name_once()`（`ps`/`top` 里的 comm）同步跳过 `--argv0` 的值，否则会把程序名认成
   `addgroup` 这一串而不是真正的程序。

验证（同一棵 App 的树）：

```text
$ addgroup --quiet --system fixedgrp; echo $?
0
group: 1  passwd: 0          ← 现在是组了
$ getent group fixedgrp
fixedgrp:x:106:
$ adduser --quiet --system fixedusr      → rc=0，passwd:1
$ bash -c 'echo bash_ok'                → bash_ok
$ ps -o comm= -p $$                     → bash
$ dpkg --configure geoclue-2.0          → Setting up geoclue-2.0 …；getent group geoclue → geoclue:x:107:
$ dpkg --configure -a                   → RC=0
$ dpkg -C                               → 空
```

loader 变了：`ld-embedded.so` = `7b4300a4…`（旧 `59b25f2c…`），
`libpath_shim.so` = `1b86173b…`；`docs/ENGINEERING.md` 两个基线已更新。

## 4 · 仍未做（UNKNOWN）

- 修完这两条之后，从 UI 全新跑一遍（空 checkpoint、空 rootfs）的**端到端**结果，
  本文写作时正在跑；成功与否以下一份 evidence 为准。
- 165 条清单缺包已定性：只在 1.x 遗留的 `bootstrap-packages.txt` 里，
  2.0 实际走 `gnome-packages.txt`（729 条，与资产里 729 个 deb 一一对应，0 缺失），
  且 `installBootstrapPackages()` 在 `app/src/main/java` 里**没有任何调用点**（FACT）。
- 静态二进制这条要单独收口：guest 里凡是静态链接的 ELF 都不会经过 shim，
  在 untrusted_app 域里随时可能被 seccomp 打死。目前只处理了 ldconfig.real 这一个。
