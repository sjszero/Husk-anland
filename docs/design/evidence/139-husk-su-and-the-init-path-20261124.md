# 139 · `su` 换成 husk-su：App 的初始化第一次走完（INITIALIZATION COMPLETE）

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
rootfs：App 从 `assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar` 解出来的那棵，
资产版本 `husk-runtime-spawn-link-8`，loader `33dcd681…`，shim `06abee35…`。

## 0 · 结论（FACT）

138 之后，App 的 UI 流程走到 `INITIALIZE_ENVIRONMENT` 的 `ensureGnomeSessionAssets`，死在

```text
su: System error
```

util-linux 的 `su` 在这台机器上**无法工作**，原因是它的换身份动作在容器里没有意义也没有权限：

```text
$ runuser -u husk -- id
runuser: cannot set groups: Operation not permitted            ← EPERM（真实 uid 10512 无 CAP_SETGID）
$ su - husk -c id                     → su: System error       ← PAM_SYSTEM_ERR
$ su husk -c id                       → su: System error
$ su root -c id                       → su: System error       ← 连"切到 root"都失败
```

`runuser` 那条是 `initgroups()`：它在 libc 内部直接调 `setgroups()`，不走 GOT，shim 拦不到，
真实系统调用被内核拒。`su` 那条**不是**某个 PAM 模块：把 `/etc/pam.d/su` 换成
`auth sufficient pam_rootok.so` + 全 `pam_permit` 也照样 `System error`，往配置里加
`pam_echo.so HUSK-MARKER` 也一行都不打印，删掉整个 `/etc/pam.d/su` 还是同一句 ——
失败发生在读配置之前。**UNKNOWN**：具体是 `pam_start` 还是更早；本轮不追。

**修法：换掉 `su`，不用 PAM。** 这个运行时里身份是 shim 维护的虚构（`NR_FAKE_UID` /
`NR_FAKE_GID`，`getuid`/`getgid`/`getgroups`/`chown` 都按它回答），所以 `su` 根本不需要权限，
它只需要：查目标账户 → 设契约变量 → exec shell。新增 `runtime/crates/su`（`husk-su`），
`tools/guest-config/install-su.sh` 用 `dpkg-divert` 把 `/usr/bin/su`、`/usr/sbin/runuser`
换成两行 wrapper。

结果（同一棵 App 的树，UI 流程从 `CREATE_USER` 断点续跑）：

```text
START INITIALIZE_ENVIRONMENT: 正在完成系统设置
DONE  INITIALIZE_ENVIRONMENT
START SAVE_CONFIGURATION: 正在保存 GPU 线路
DONE  SAVE_CONFIGURATION
INITIALIZATION COMPLETE
```

## 1 · husk-su 实测

```text
$ su - husk -c 'id; echo HOME=$HOME USER=$USER SHELL=$SHELL; pwd'
uid=1000(husk) gid=1000(husk) 组=1000(husk)
HOME=/home/husk USER=husk SHELL=/bin/bash
/home/husk                                     ← login 模式会 chdir 到 home
$ su husk -c 'id -un; echo PATH=$PATH'         → husk / 标准 PATH（非 login 继承调用方）
$ su -c 'echo hi' husk                         → hi（-c 在 user 之前的写法）
$ su -c 'id -un'                               → root（缺省目标是 root）
$ su - nosuchuser -c id                        → husk-su: user nosuchuser does not exist（rc=1）
$ NR_FAKE_UID=1000 su - root -c id             → 拒绝：password authentication is not available…（rc=1）
$ su - husk -c /nonexistent                    → 127（exec 失败按 sh 惯例）
$ runuser -u husk -- id -un                    → husk
```

设计取舍（写进代码注释，这里只列）：

- **不做密码认证**：调用方要么是 root（shim 报的 0），要么已经是目标用户；其余一律拒绝并说明。
- **不做 PAM**：没有 session manager 可配，容器用户也不是系统账户。
- `su -` 清环境，只留 `NR_*` / `GLIBC_TUNABLES` / `TERM` / locale / 显示与总线变量
  （会话要够到正在跑的画面与总线，真 `su -` 不会给，调用方得自己 export —— 这里保留，
  是**有意的偏差**，写在 `keeps()` 的注释里）。
- 未知选项报错，不静默忽略（`-w`、`--pty` 等）。

## 2 · 门禁 A 验收脚本在 App 的树上跑：34 PASS / 0 FAIL

`14-gate-a-acceptance.sh` 原来假设 rootfs 旁边有个 `husk` 启动器、且不设 `HOME`/`PATH`，
换成参数化的 App 树就跑不起来。本轮把它改成自洽的启动方式（`cd` 到 guest root、
`--library-path` 显式给全、`HOME=/root`、丢掉 `XDG_*`），并修了两处判据：

```text
PASS=34 FAIL=0
  dpkg -C 空 / 安装 814 / unpacked 0 / getfacl 可用
  NOTE  no .deb on this tree（离线缓存已被 App 删掉）→ 重装两项跳过，不再假失败
  宿主路径泄漏：只有 NR_LD / NR_ROOTFS；guest /data/user 不存在
  真实 SIGSYS trap 0
```

- **FACT（本轮新踩）**：`HOME` 必须由调用方设成 guest 值。App 进程自己的 `HOME` 是
  `/data/user/0/com.linuxhub.myth.husk`，dconf 会去 `$HOME/.cache/dconf` 写缓存，
  shim 照着改写就在 rootfs 里长出 `data/user/0/com.linuxhub.myth.husk/.cache/dconf` ——
  宿主形状的垃圾进了 guest。脚本现在 `unset XDG_*` 并显式 `HOME=/root`。
  （App 自己的会话用 `env -i` + `HOME=/root`，没有这个问题。）
- 安装数从 811 变 814：App 会额外装 `sudo` 等 3 个 deb，所以判据改成 `EXPECT_PKGS` 参数。

## 3 · 顺带量出来的三个缺口（都不在门禁 A 判据里，记账）

### 3.1 guest 二进制的 `DT_RUNPATH` 不被改写（FACT）

```text
$ … usr/bin/sudo --version
usr/bin/sudo: error while loading shared libraries: libsudo_util.so.0
$ … --library-path …:/usr/libexec/sudo usr/bin/sudo --version
sudo: /etc/sudo.conf is owned by uid 10265, should be 0
```

sudo 的 `RUNPATH=/usr/libexec/sudo`，文件就在那儿（`ls` 有），但加载器搜不到：`--library-path`
里没有它。也就是说 **`RUNPATH` 目录里的库只有靠调用方把它们加进 `--library-path` 才能找到**
（App 现在正是这么对 mutter-14 / gnome-shell / gvfs / systemd 做的）。同类：gio 的
`libgiolibproxy.so` 找不到 `libpxbackend-1.0.so`（`/usr/lib/aarch64-linux-gnu/libproxy/`），
`gsettings` 仍然 rc=0，只是每次都打一行警告。

系统性的修法是在 ld.so 的 `open_path` 上把 guest `RUNPATH` 按 `NR_ROOTFS` 改写；
本轮只记录，不改。

### 3.2 `sudo` 的假账没人认（FACT，接 3.1）

加载起来之后 sudo 立刻拒绝：`/etc/sudo.conf is owned by uid 10265, should be 0`。
也就是说 sudo 看到的 `st_uid` 是真实 app uid，而不是虚构的 0 —— 我们对"guest 自己 chown 过的
路径"有别名表，但这条路径不是本进程改的。**UNKNOWN**：这属于"跨进程的虚构所有权"该怎么落，
需要单独设计（chown 表持久化？按 NR_FAKE_ROOT 全局谎报 owner？）。门禁 A 不依赖 sudo 可用。

### 3.3 dconf 的 `SCM_CREDENTIALS`（已知，G 段）

```text
dconf-WARNING: failed to commit changes to dconf: 发送凭据时出错：不允许的操作
```

`apply-ubuntu-defaults.sh` 里每条 `gsettings set` 都 rc=0（脚本整体 rc=0），但设置**没落地**。
与 STATUS 里记的 GDBus `SCM_CREDENTIALS` 在 fake root 下 EPERM 是同一件事，
属于桌面段（G，延后）。

## 4 · 指纹

```text
husk-su          e64cdf1e…（本轮新增，515736 字节，`tools/guest-config/build-guest-tools.sh` 可复现）
ld-embedded.so   33dcd681…（不变）
libpath_shim.so  06abee35…（不变）
libc-nop.so.6    32d4d3f9…（不变）
```

## 5 · 仍未做（UNKNOWN）

- **干净树完整 UI 重跑**：本文的 INITIALIZATION COMPLETE 是**断点续跑**（`CREATE_CONTAINER`
  与 `CONFIGURE_SUDO` 来自上一轮，另外手工在树里放过 husk-su 与 wrapper 做验证）。门禁 A 要的
  「原始资产 + 空 checkpoint + 全程 UI」这轮**还没跑**，以 evidence 140 为准。
- `sudo` 真的能用、`su` 真的走 PAM（如果以后需要），都要等 3.1/3.2 的修法。
