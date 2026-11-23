# 136 · 门禁 A 的验收项：bash / locale / 用户管理 / dpkg / apt / 宿主路径 / 服务启动

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
rootfs `files/linux/husk-clean/rootfs`（135 里那棵干净树），loader `files/linux/husk/libld.so`
（`59b25f2c…688d549`）。

## 0 · 结论（FACT）

两个可复现脚本，一次跑完，全部通过：

```text
tools/device-tests/14-gate-a-acceptance.sh   →  PASS=36 FAIL=0
tools/device-tests/12-supervisor.sh (参数化后) →  PASS=8  FAIL=0
```

覆盖 135 留下的「通过后再验收」清单：bash、apt/dpkg、locale、用户管理、服务启动；
外加宿主路径泄漏与真实 seccomp trap 两项门禁判据。**App 的 UI 流程仍未跑**
（干净树是 loader 手驱的，见 §9）。

## 1 · 怎么复现

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/husk-clean/rootfs LD=files/linux/husk/libld.so \
  ./tools/device-tests/14-gate-a-acceptance.sh

adb push runtime/target/aarch64-unknown-linux-gnu/release/husk-init /data/local/tmp/husk-init
adb push tools/device-tests/12-supervisor.sh /data/local/tmp/12.sh
adb shell "run-as com.linuxhub.myth.husk sh -c 'FILES=/data/data/com.linuxhub.myth.husk/files \
  ROOT=linux/husk-clean/rootfs LD=linux/husk/libld.so sh /data/local/tmp/12.sh'"
```

两个脚本这轮都改成**参数化**，不再硬编码 `com.linuxhub.husk` / `rootfs-full` / `libld39.so`：
`12-supervisor.sh` 读 `FILES`/`ROOT`/`LD`，`14-gate-a-acceptance.sh` 读 `DEV`/`PKG`/`ROOTFS`/`LD`。
此前那轮是把脚本改一份再推上去，改动本身留在设备上没有记录——这条按「工程纪律」补齐了。

`14-gate-a-acceptance.sh` 的执行方式：guest 脚本经 `adb push` → `run-as cp` 落进
`<rootfs>/tmp/`，再由 `husk` 启动器以 `usr/bin/bash /tmp/…` 跑（loader 只接受**相对**的程序路径）。

## 2 · bash（6/6）

```text
PASS  bash version (5.2.21(1)-release)
PASS  pipeline (3)
PASS  heredoc + arithmetic (heredoc:42)
PASS  subshell isolation (1 0)
PASS  process substitution (ok)
PASS  brace expansion (4)
```

## 3 · locale（3/3）

```text
PASS  zh_CN charmap (UTF-8)
PASS  zh_CN weekday (星期一)
PASS  C.UTF-8 collation works (ok)
```

134 修的两个 bug（`posix_spawn(pid=NULL)`、`link` copy 后重同步）在这里闭环：
`localedef` 的 `.gz` charmap 真被解出来、`locale-archive` 真被写完。

## 4 · 用户管理（9/9）

```text
PASS  groupadd rc (0)
PASS  useradd rc (0)
PASS  chpasswd rc (0)
PASS  id resolves (1500)
PASS  home created (yes)
PASS  shadow hash (sha512)
PASS  group entry (nrtest)
PASS  cleanup (gone)
```

`useradd -m` 真建了 `/home/nrtest`，`/etc/shadow` 里是 `$6$`（sha512-crypt），
`getent group 1500` 能反查。测完删干净，不留测试账号。

## 5 · dpkg（7/7）

```text
PASS  dpkg -C clean before (0)
PASS  installed packages (811)
PASS  unpacked packages (0)
PASS  dpkg-deb -I (ok)
PASS  force-overwrite reinstall rc (0)      # dpkg --force-overwrite -i acl_2.3.2-1build1.1_arm64.deb
PASS  dpkg -C clean after (0)
PASS  getfacl still runs (ok)
```

也就是说**「装完再装一遍」这条路也过**：同一个 `acl` deb 重装后 `dpkg -C` 仍为空、
包内程序仍可执行，不是「第一次侥幸装上了」。

## 6 · apt（4/4，一条 NOTE）

```text
PASS  apt version (2.8.3)
PASS  policy knows acl (yes)
        apt-cache policy acl -> Installed: 2.3.2-1build1.1
PASS  apt-get -s rc (0)
NOTE  Reinstallation of acl is not possible, it cannot be downloaded.
```

`apt-get -s install --reinstall acl` 退出码 0，但打印 "cannot be downloaded"：
包就在本机 `/var/cache/doro-offline/packages/acl_*.deb`，缺的是**可用的网络索引**
（`/var/lib/apt/lists` 空）。**INFERENCE（未单独验证）**：这是 2.0 的离线安装设计
（走 `dpkg` 直装而非 apt 仓库），不是 Husk 的缺陷；「apt 真正联网装一个包」这条
在本轮**未验（UNKNOWN）**，前面 106/121 记录的是 apt 走网络下载可用，但那不是这棵干净树。

## 7 · 宿主路径泄漏（5/5 PASS + 一条 FACT 记录）

```text
  vars whose value points at the app dir: NR_LD NR_ROOTFS
PASS  host paths only in internal NR_ vars (0)
PASS  ps sees only the guest process tree (small)
PASS  no host system processes visible (0)
NOTE  guest-visible argv shows the app dir in 4 process(es) (the loader's own argv)
PASS  guest /data/user does not exist (absent)
PASS  guest /system does not exist (absent)
PASS  readlink /proc/self/exe is host loader (host)
```

- 环境里指到 app 私有目录的只有 `NR_ROOTFS` / `NR_LD` 两个内部契约变量，没有别的变量
  带宿主路径；`/data/user`、`/system` 在 guest 内**不存在**（`ENOENT`，不是被遮蔽的假目录）。
- `ps -e` 只看得到 guest 自己的进程树（≤16 个），宿主 `system_server`/`zygote`/`/apex` 一个都看不到。
- **必须记下来的一条 FACT**：`readlink /proc/self/exe` 和 `ps -o args` 能看到**宿主 loader 路径**，
  因为进程镜像就是 loader，argv 也是 [`libld.so --library-path … /rootfs/usr/bin/bash`]。
  这是同一 uid 内的自我暴露，**没有跨过任何权限边界**（别的 app 读不到我们的 `/proc/<pid>`），
  所以不判为门禁失败；但它是产品观感问题，也是「用 /proc/self/exe 重新执行自己」这一类程序的隐患，
  列为已知项：要修只能让 loader 解析完参数后原地重写自己的 argv，属于后续独立改动。

## 8 · 真实 seccomp trap（1/1）

```text
PASS  real SIGSYS traps (0)
```

测法：guest 里 `export NR_SIGSYS_LOG=/tmp/gate-a-sigsys.log`，shim 把每一次
**真的被 seccomp 拦下**的记录写成 `sigsys|nr=…` 行（`nr-init|sigsys handler installed`
是启动标记，不算）。整轮 36 项检查 + 前面的全量装包，`sigsys|nr=` 命中 0 次。

## 9 · 服务启动：husk-init（8/8）

```text
PASS  a service really runs (tick.log=4)
PASS  once service ran and was not respawned
PASS  once service ran exactly once (count=1)
PASS  on-failure service logged its failure
PASS  a killed service is restarted (tick.log 15 -> 19)
PASS  SIGTERM stops the supervisor (1x0.5s)
PASS  no service process survives the stop
PASS  no zombies left behind
```

`husk-init` 是 freestanding 静态二进制（不经 loader 启动），这轮在**干净树**上重跑，
服务命令用的是 guest 的 `/usr/bin/bash`。二进制 sha256
`65b54b70fd2df4efeec85b7686b103a1c4a619f866be1c2b4c40dd0d652be41e`（28864 bytes，static-pie）。

## 10 · 仍未做（UNKNOWN，别当已通过）

- App UI 流程（`initialize()` + `ensureHuskRuntime()`）在干净目录上没跑过。
- 165 条清单缺包在 Kotlin 侧怎么处理，仍未查。
- `apt-get install` 真联网装上包，这棵干净树上未验。
- 桌面/GNOME 会话未在这棵树上跑（门禁 A 不含）。
- 16K/64K 真机、非 Mali GPU、外设族维持原结论（UNKNOWN）。
