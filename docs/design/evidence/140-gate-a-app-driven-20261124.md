# 140 · 门禁 A 通过：原始资产 + 空 checkpoint + 全程 App UI

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`
（`applicationIdSuffix .husk` 的 debug APK），GPU 线路「天玑 / Mali」，用户 `husk`。

## 0 · 结论（FACT）

**门禁 A 通过。** 这一轮没有手工步骤：树是 App 用 APK 自带的
`assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar` 新解出来的，checkpoint 与
`container.properties` 都在开跑前挪走，全程只有 UI 点击（开始初始化 → 天玑 / Mali
→ 继续创建 Linux 用户 → 填账号密码 → 下一步）。

```text
START EXTRACT_SYSTEM             1795481012578
DONE  EXTRACT_SYSTEM
DONE  CREATE_CONTAINER           （729 包全量 unpack + configure）
DONE  CREATE_USER
DONE  CONFIGURE_SUDO
DONE  INITIALIZE_ENVIRONMENT
DONE  SAVE_CONFIGURATION
INITIALIZATION COMPLETE          1795481535273      共 522.7 s（8m43s）
```

判据逐条（同一棵树，验收脚本 + 日志）：

| 门禁要求 | 实测 |
|---|---|
| 全量 dpkg 配置，postinst 全部成功 | `dpkg -C` 空、`install ok installed` **814**、`install ok unpacked` **0**；本轮窗口内 `error processing package` **0** 次 |
| 无 SIGSYS | 窗口内 `Bad system call` **0** 次；`tmp/nr-sigsys.log` 共 **1202** 条 trap 记录，全部由我们的 handler 接住（其中 1182×`nr=264`=`name_to_handle_at`、19×`nr=452`=`fchmodat2`，1 条记录没有 `nr=` 字段），**没有一次是未接住导致进程死** |
| 无 clone3 | 窗口内无 clone3 相关报错（`libc-nop.so.6` 里 clone3→ENOSYS 的补丁在，`32d4d3f9…`） |
| 无 argv[0] 错误 | 验收脚本 `groupadd` 建的是**组**（`getent group nrtest` 命中），`addgroup → adduser` 分派正确 |
| 无 libc 槽位错误 | 窗口内 `HUSK-SLOT` 相关报错 0；`dpkg -C` 空 |
| 无宿主路径泄漏 | 只有 `NR_LD`/`NR_ROOTFS` 指向 app 目录；guest `/data/user`、`/system` 都不存在；`ps` 只见 guest 自己 |

## 1 · 验收脚本（同一棵树）

```text
EXPECT_PKGS=814 … 14-gate-a-acceptance.sh   →  PASS=34 FAIL=0
12-supervisor.sh（FILES/ROOT/LD 指到这棵树） →  PASS=8  FAIL=0
```

34 项覆盖：bash 6、locale 3、用户管理 8（groupadd/useradd/chpasswd/uid/home/shadow/组/清理）、
dpkg 4、apt 4、宿主路径 6、identity 2、trap 1。两条 NOTE 不是失败：

```text
NOTE  no .deb on this tree（离线缓存已被 App 删掉）→ 重装两项跳过
NOTE  apt-get -s 需要网络索引（离线缓存里有 deb）：Reinstallation of acl is not possible…
```

服务监督器 8/8：真跑、`once` 只跑一次、`on-failure` 记失败、被杀后按退避重启、
SIGTERM 0.5 s 内停、无残留、无僵尸。

## 2 · 窗口内的全部噪声（逐条定性，没有一条是失败）

```text
62  warning: symbolic link '…' size has changed from 18 to 16      dpkg 单遍 -i 的固有观感
2   dpkg: error processing archive python3{,-minimal} …            pre-dependency problem，后续轮次补上
    → 结束时 python3 / python3-minimal 都是 ii，/usr/bin/python3 在
2   libpxbackend-1.0.so: cannot open shared object file             RUNPATH 缺口（139 §3.1）
8+  dconf-WARNING: failed to commit changes to dconf: 发送凭据时出错  SCM_CREDENTIALS（139 §3.3，G 段）
```

`python3` 那两条值得记：`dpkg -i` 单遍遇到 `Pre-Depends` 会明说 "not installing" 并继续，
App 的安装循环后面的轮次把它装上了 —— 这是**顺序问题不是缺陷**，但它是「`dpkg -C` 空」
不能单独作为「全部装好」判据的原因（因此本轮同时数了 814 / 0 unpacked）。

## 3 · 这一轮之前修掉的三层（前序 evidence）

- **137**：`/usr/sbin/ldconfig` 是包装脚本、真正的 `ldconfig.real` 是静态二进制，在
  `untrusted_app` 域被 seccomp 打死 → APK 发 `husk-ldconfig` 并把 `ldconfig` 指过去；
  嵌套 exec 丢 `argv[0]` 导致 `addgroup` 建用户 → `--argv0` + `#!` 传调用路径。
- **138**：dpkg 拆 conffile 的 create→write→`chmod(0440)`→**再 open 写** 在假 uid 下必
  `EACCES` → `openat` 的 `O_CREAT|0600` + `EACCES` 提权重试。
- **139**：util-linux 的 `su` 用不了（`initgroups` 的 `setgroups` 拦不到；PAM_SYSTEM_ERR）
  → `husk-su`。

资产：`husk-runtime-spawn-link-8`；loader `33dcd681…`、shim `06abee35…`、libc `32d4d3f9…`、
`husk-ldconfig 35a9c2a8…`、`husk-su e64cdf1e…`、`husk-usermgr 33859a1e…`。

## 4 · 仍未做（UNKNOWN / 下一段）

- **C 段**：用 Husk 替换 2.0 的 proot（Kotlin 侧已接上，运行时已是 Husk），逐项删除
  proot 专用补丁 —— 以「同 rootfs 功能回归」为删除依据。这轮之后 2.0 的初始化路径上
  已无 proot 依赖，但没有做过「删掉 proot 分支后整体回归」。
- 「服务启动」只验了 `husk-init` 监督器本身（8/8）；**2.0 的真实服务单元**
  （`etc/husk/services.conf` 里的 GNOME 会话服务）不在本轮范围。
- 139 §3.1/§3.2 的两个口径缺口（guest `DT_RUNPATH`、跨进程虚构所有权）。
- 桌面帧出屏（G 段）按目标延后；dconf 的 `SCM_CREDENTIALS` 属于它。

## 5 · 收尾时量到的一条泄漏（FACT，待设计）

清理设备时发现 **孤儿 `dbus-daemon`**：`ps` 里有 5 个 PPID=1 的
`--argv0 dbus-daemon … --nofork --print-address 4 --session`，全部指向这棵树的
`usr/bin/dbus-daemon`。来源是 `dbus-run-session`：它启动的会话总线父进程退出后不回收到
init，于是每跑一次留一个。App 每轮初始化至少调用两次（`ensureGnomeSessionAssets` 的
legacy-tray 与 `apply-ubuntu-defaults`），`startDesktop()` 每次启动还会再跑一遍 ——
也就是说**这是一个会累积的泄漏**，不是本轮的一次性现象。

本轮把它们 `kill` 掉了（我们自己的 5 个；`com.linuxhub.husk` 那条旧会话进程不属于本包，
没动）。修法需要设计：要么启动时按 `XDG_RUNTIME_DIR` 里的 pid 文件收割旧总线，要么让
`husk-su` 结束前把自己的进程组收干净。未做，记在这里。
