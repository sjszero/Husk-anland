# 当前状态

> 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64），
> 运行域 `untrusted_app`，无 root、无 Shizuku。
> 只写**已上机验证**的结论；未验证写 UNKNOWN。每条的原始输出在 `docs/design/evidence/`。

## 接手须知（冷启动只读这一节 + 本文件的表）

**设备**：`adb` serial `UKHMMB8PYTGECMNV`（RMX3357）。2.0 集成现场是应用
`com.linuxhub.myth.husk`（debug 包名后缀），rootfs 在
`files/linux/distributions/ubuntu-24.04/rootfs`，loader 在 `files/linux/husk/libld.so`，
资产版本标记 `files/linux/husk/assets.version`（当前 `husk-runtime-spawn-link-11`）。
旧测试会话的 `com.linuxhub.husk`（`files/rootfs-full`、`files/libld3.so`）不要动。

**启动契约（两个变量必须同时设，缺一个就"看起来像 shim 坏了"）**：

```sh
NR_ROOTFS=<rootfs 绝对路径> NR_LD=<我们的 loader 绝对路径> NR_LIBRARY_PATH=<完整库路径>   <loader> --library-path "$NR_LIBRARY_PATH"   usr/bin/bash -c '...'          # 程序路径必须相对
```

**构建 loader**：`NDK=<ndk>/bin OUT=/tmp/x sh tools/loader/build-embedded-loader.sh`
（产物覆盖 `tools/loader/ld-embedded.so`，`nr-tools embed` 自带页对齐断言）。

**16K/64K 真内核环境**（QEMU，别重建）：`/tmp/k16`（`Image.rpi5` + `root/` 的 initramfs 源），
命令见 `docs/design/evidence/125`。一次启动 8–15 秒。

**仓库布局**：`runtime/crates/{path-shim,nr-tools,runtime-core}`（Rust）、`tools/loader/*.S`（裸汇编，仅入口与 hook）、
`tools/device-tests/*.sh`、`docs/design/`（设计 + `evidence/` 原始记录）。

**当前开口（按优先级）**：
1. 非 Mali GPU 家族**适配器**（Adreno kgsl+Turnip / 通用 DRM）：探测表与降级已落地（evidence 127），
   适配器本身等有设备再写——没设备只能出未验证代码；
2. 传感器 / GPS / USB / 蓝牙 / NFC / 指纹——未开始（先读 `husk-notes/prior-art/sensor-gps-*.md`、`platform-peripherals-*.md`）；
3. 2.0 集成：映射表与替代启动器已落地（docs/design/linuxhub-2.0-swap.md，evidence 129）；
   ① `su` 已用 husk-su 替掉（evidence 139），PAM 那条路仍是 UNKNOWN 但不再需要；
   ② Kotlin 侧已经接上（运行时替换 + 资产），门禁 A 的干净重跑见 evidence 140；
   ③ guest `DT_RUNPATH` 已由"guest 自己声明库目录"覆盖（evidence 141）；跨进程虚构所有权
      （`st_uid`）已落成 `/var/lib/husk/owners` 这张 inode 例外表（evidence 142）。
      **sudo 现在卡在 PAM/audit**（`ENOTSOCK`、audit netlink），属服务/桌面表面，不在门禁 A 判据里。
      另外把「嵌套 exec 的库目录」从写死的 6 个目录改成读 guest 自己的 `/etc/ld.so.husk-path`
      兜底，并让初始化在 `SAVE_CONFIGURATION` 再声明一次 —— 否则 App 重启前 `sudo` 找不到
      `/usr/libexec/sudo/libsudo_util.so.0`（evidence 143）；
   ④ **C 段：proot 已整段删除**（evidence 144）——APK 不再带 proot / loader / libtalloc /
      libandroid-shmem / getifaddrs 桥，`tools/proot`、`tools/getifaddrs_bridge`、
      `rust/getifaddrs-bridge` 也已出库；登录、会话、`executeContainerCommand`、桌面启动只走
      `HuskRuntime.command`。判据是同 rootfs 回归：空 checkpoint 从 UI 跑完 814 包、`dpkg -C` 空、
      0 `Bad system call`、门禁 A 34/34、supervisor 8/8、542.8 s（删 proot 前 533/541 s）；
4. 交付形态：**已落地**（`tools/guest/install-husk.sh`，evidence 130）——本地 tarball 或 URL 一条命令
   装完 + 生成启动器，本机 app 私有目录端到端通；**真 Termux 验收 UNKNOWN**（本机没装 Termux，
   adb shell 里也没有 curl/wget，URL 分支只在 macOS 上证过下载段）。

**参考笔记不进仓库**：`/Users/l/Documents/Codex/2026-08-10/husk-notes/prior-art/`（必读 `references.md`）。
**推 GitHub 走代理**：`HTTPS_PROXY=http://127.0.0.1:7897 HTTP_PROXY=http://127.0.0.1:7897 git -c http.proxy=http://127.0.0.1:7897 push origin main`

## 一句话

运行时核心（loader + path-shim + exec/PTY/信号/生命周期）、服务器表面（`/proc`、`/dev`、
`/dev/log`、装包）和 GPU、显示、视频、音频、相机、输入六类硬件都已在该机跑通。
按"对标容器/轻量虚拟机"的环境口径，A（账号）、B（挂载）、C（设备节点）、D（网络表面）
四段已收口，E（进程视图 + 服务监督器）已收口；**F 只剩 16K/64K 实机和 Mali 以外的 GPU 家族适配**。
桌面帧出屏（G）按目标延后，不作为环境完整性判据。传感器、GPS、USB / 蓝牙 / NFC / 指纹
尚未开始；2.0 已经**不含 proot**——APK 与仓库里都没有 proot，初始化与会话全部跑在 Husk 上（evidence 144）。

另外记一条会反复踩的事实（evidence 130）：2.0 assets 里的 `ubuntu-base-24.04.4-base-arm64.tar`
**是 gzip**（`1f 8b`，md5 `0be8b9e5…`），只是文件名没有 `.gz`。装之前先 `od -An -tx1 -N6`。

## 已完成（回归门禁，改动后必须重跑）

| 项 | 结论 | 证据 |
|---|---|---|
| loader + path-shim | 零 ptrace，路径一次改写完成；dlopen 进来的库也覆盖 | 27 / 100 |
| exec / PTY / 信号 / 生命周期 | 600 秒长稳、零僵尸、无重启风暴；旋转与 Surface 重建不重启会话 | 32 / 72 |
| 页大小 | rootfs 2987 个 ELF + loader + 我们全部产物在 4K/16K/64K 静态体检全绿（含 `p_align % 页` 规则）；**真 16K 内核（RPi5 kernel）与真 64K 内核（Ubuntu `linux-image-6.8.0-31-generic-64k`）上我们的 loader 端到端全绿**：`cat /etc/os-release`、`/usr/bin` 计数、嵌套 exec、`/tmp` 写读、`check-elf` 727 个 ELF 三档 congruent；修掉三个真 Bug（段重叠、mprotect 对齐、xattr/readlink 返回裸 -errno）；**16K/64K 真机仍 UNKNOWN**（本机只有 4K） | 30, 122, 124, 125, 126 |
| GPU | PanVK 列举 Mali-G77、compute、离屏 graphics、Zink surfaceless | 46 / 47 |
| 设备族探测与降级 | `path-shim/src/plan.rs`：按节点定族（mali0 / kgsl-3d0 / renderD128 / 都没有），只有 Mali 才遮蔽 `/dev/dri`，有真节点则透传，其它家族拒绝并输出 `nr-plan: … reason=…`；`NR_FORCE_SOFTWARE_GPU` 实测降到 llvmpipe 且不崩。Adreno 与真 DRM 节点仍 UNKNOWN（无设备） | 127 |
| 外设族探测 | `gpu-probe periph`：传感器/GPS/USB/蓝牙/NFC/指纹/输入/i2c/spi 逐个节点实测，`EACCES`(13) 与 `ENOENT`(2) 分开记录；不造假节点、进程退出码 0。本机是否真有传感器仍 UNKNOWN（EACCES 藏住了） | 128 |
| 用户身份与 2.0 swap | `NR_FAKE_UID/GID` + `setuid/setresuid/setreuid/getresuid` 跟随 + `getgroups` 只报主组：`id` 报 `uid=1000(hu1)`，`su` 这一步可以删；`tools/guest/husk-session.sh` 是 `prootCommand()` 的替代契约（`env -i` 必须带 `NR_ROOTFS/NR_LD`）。**`su`/PAM 仍报 `PAM_SYSTEM_ERR`（UNKNOWN，已绕开）** | 129 |
| 显示 | AHB → dma-buf → SurfaceControl；3-buffer ring + 真实 SYNC_FD；10,000 帧 / 665 秒 | 53–59 / 73 |
| 视频 | H.264 1080p30 连续 600 秒，dropped=0；VA-API 解码出真实帧 | 74–80 / 85–87 |
| 音频输出 | PulseAudio → Rust relay → AAudio，610 秒连续会话 | 60 / 62 / 63 |
| 音频输入 | 声学回环 440 Hz 通过 | 88 |
| 相机 | guest `/dev/videoN` V4L2，GStreamer 取流、拍照、录像 | 91 / 92 / 93 |
| 输入 | 触摸/键盘/鼠标经 mutter `RemoteDesktop` 注入 guest 合成器 | 95 / 98 |
| 桌面 | 2.0 rootfs 起 GNOME Shell 46（Wayland 显示服务器），GTK4 客户端可连 | 101 |
| 装包 / dpkg | `apt-get install` / `--reinstall` / `-f install` 全通；dpkg 库零半配置包 | 106 / 109 |
| `scandir` / `dpkg-deb` | `scandir` interpose 补齐；guest `/tmp` 临时控制目录可见，`dpkg-deb -I` 与真实 `dpkg -i acl` 均返回 0 | 131 |
| ld.so.cache | 自研 Rust writer（1.1 格式），glibc 自己的静态阅读器接受 | 109 |
| 嵌套 exec 的库路径 | `NR_LIBRARY_PATH` 由 launcher 传给 shim，所有 nested exec 复用同一份完整 `--library-path`；只设 `NR_ROOTFS`/`NR_LD` 时走内置多目录 fallback。旧行为只重建 `<rootfs>/usr/lib/aarch64-linux-gnu`，`systemd` 私有目录在第二层进程消失（`libsystemd-shared-255.so` 找不到） | 133 |
| `posix_spawn` 的 NULL pid | glibc 允许 `pid == NULL`（`spawni.c` 只非空才回写），Ubuntu `localedef` 正是这样把 `.gz` charmap 交给 `gzip -d -c`；shim 原先把 NULL 当 EFAULT，导致所有压缩 charmap 报"not found" | 134 |
| 硬链接的 copy 模拟 | 本域 `link(2)` 被拒（EACCES），一直 copy 兜底；但"发布后再写"的模式（glibc 的 locale-archive：mkstemp→mmap→link→unlink→继续写原 fd）会丢掉全部后续写入。现在 copy 后登记"源 inode→目标绝对路径"，`close()` 时经 `/proc/self/fd/<fd>` 重同步 | 134 |
| locale | `locale-gen` 全量生成 `zh_CN.utf8`/`zh_SG.utf8` 进 `/usr/lib/locale/locale-archive`，`LC_ALL=zh_CN.UTF-8 locale charmap` → `UTF-8` | 134 |
| 门禁 A（干净重跑 + 验收） | APK 自带的 ubuntu-base tar → 新目录 → 资产里的包 → `dpkg --configure -a`：`RC=0`、`dpkg -C` 空、status 811/811 installed、`zh_CN.utf8`/`zh_SG.utf8` 生成、真实 SIGSYS trap 0 次。随后验收 `tools/device-tests/14-gate-a-acceptance.sh` **36/36 PASS**（bash 6、locale 3、用户管理 9、dpkg 7 含 force-overwrite 重装、apt 4、宿主路径 5、trap 1）与 `12-supervisor.sh` **8/8 PASS**；两个脚本已参数化（`PKG`/`ROOTFS`/`LD`）。已知不是门禁失败的项：`/proc/self/exe` 与 argv 暴露宿主 loader 路径（同 uid，无权限跨界）。清单里 165 个 deb 资产里没有（原现场也没有），已记账。App UI 流程当时仍未在干净目录上跑 | 135 / 136 |
| **门禁 A（App 自己跑）** | APK 自带的 ubuntu-base tar → 空 checkpoint → **全程 UI 点击**：`EXTRACT_SYSTEM → CREATE_CONTAINER → CREATE_USER → CONFIGURE_SUDO → INITIALIZE_ENVIRONMENT → SAVE_CONFIGURATION` 全部 DONE，`INITIALIZATION COMPLETE`，522.7 s。同一棵树上：`dpkg -C` 空、814 装、0 unpacked、窗口内 `error processing package` 0 次、`Bad system call` 0 次，1202 条 seccomp trap 全被 handler 接住（1182 `name_to_handle_at` / 19 `fchmodat2`）；`14-gate-a-acceptance.sh` **34/34**、`12-supervisor.sh` **8/8**。噪声全部定性：62 条 dpkg 符号链接尺寸警告、python3 两条 pre-dependency 由后续轮次补上（末态 `ii`）、libpxbackend 的 RUNPATH 缺口、dconf 的 `SCM_CREDENTIALS` | 140 |
| 跨进程虚构所有权 | rootfs 文件一律报 `0:0`，只有 guest 自己 chown/创建的是别人；锚点是 `/var/lib/husk/owners` 的 inode 例外表（约 30 行/树），从 `stat`/`fstatat`/`statx` 读出、从 `chown`/`O_CREAT`/`mkdir`/`symlink` 写入、`unlink` 时按 `nlink==1` 清理，只对 rootfs 的 `st_dev` 生效 | 142 |
| **C 段：删除 proot** | APK 与仓库都不再有 proot：`libproot.so`/`libproot-loader.so`/`libtalloc`/`libandroid-shmem`/getifaddrs 桥（server + `LD_PRELOAD`）全部出包，`tools/proot`、`tools/getifaddrs_bridge`、`rust/getifaddrs-bridge` 出库；登录/会话/命令/桌面只走 `HuskRuntime.command`，`/run/doro-audio` 改成 rootfs 里的真目录（宿主与 guest 用同一个 FIFO）。判据是同 rootfs 回归：空 checkpoint 全程 UI，814 包、`dpkg -C` 空、0 `Bad system call`、1,193 条 trap 全接住、542.8 s（删前 533/541 s）、门禁 A 34/34、supervisor 8/8。桌面首帧仍无画面（G 延后），删除前本机没启动过桌面，故与删除无关这一条是 UNKNOWN | 144 |
| 共享存储（`/sdcard`） | 删 proot 时一起删掉的 `-b /storage/emulated/0:/sdcard` 让 guest 里 `/sdcard` ENOENT；现在 `passthrough()` 把 `/sdcard`、`/storage` 当宿主路径（`/sdcard`→`/storage/self/primary`→`/storage/emulated/<user>`），含 `..` 的路径不算透传（第一版 `/storage/../system` 曾落到宿主 `/system`）。App 自带终端实测：`ls -ld /sdcard` 是宿主符号链接、`ls /sdcard` 出真实目录项、`touch` 写入成功；`NR_SHARED_STORAGE=0` 可关。回归：空 checkpoint 814 包、`dpkg -C` 空、0 `Bad system call`、548.2 s、门禁 A 34/34、监督器 8/8 | 145 |
| 删 proot 后的 B/D/E 回归 | 生产树重跑 `01/03/04/05/09/10/11`：`/dev/shm` 随 proot 的 bind 一起消失（guest 里 `shm_open` 全 ENOENT）、嵌套 exec 的 `comm` 一直是 `libld.so`（`name_once()` 只在首次走 guest 路径时调用，且 2 KiB 栈缓冲读不下几 KB 的 `--library-path`）。修完 01 31/0、03 escapes=0、05 18/0、09 11/0、10 9/0、11 8/0；空 checkpoint 814 包、`dpkg -C` 空、0 `Bad system call`、552.6 s、门禁 A 34/34、supervisor 8/8。loader 673,280 → 738,816 B（64 KiB `.bss` 缓冲） | 146 |
| `argv[0]` 与符号链接分派 | 嵌套 exec 原先把程序名换成**解析后的目标路径**，`/usr/sbin/addgroup → adduser` 这类符号链接分派因此失效（perl 的 `$0` 变成 `…/adduser`，`addgroup --system X` 建的是**用户**不是组，dpkg-statoverride 报 `group 'geoclue' does not exist`）。现在 dynamic ELF 走加载器的 `--argv0 <调用方 argv[0]>`，`#!` 脚本把**调用时用的路径**交给解释器（内核语义）；`ps` 的 comm 同步跳过 `--argv0` 的值。实测 `addgroup` 建组、`adduser` 不受影响、嵌套 exec 与 `ps -o comm` 正常 | 137 |
| 静态二进制是域边界 | guest 里静态链接的 ELF 不过 shim，于是没有 SIGSYS handler、也没有路径层：`/sbin/ldconfig.real` 在 `untrusted_app` 域被 seccomp 直接打死（159），在 `adb shell`/`run-as` 下却能跑（只是写宿主 /etc 得 EROFS）。所以**门禁类结论必须在 App 进程里量**；`/usr/sbin/ldconfig → husk-ldconfig` 从"最好有"升级为"第一条 guest 命令之前就必须有" | 135 / 137 |
| `dpkg` 拆 conffile 的写权限 | dpkg 的拆包顺序是 create → write → `chmod`（包里的模式）→ **再 open 写**；`/etc/sudoers.d/README` 是 `0440`，内核按真实 app uid 判 owner 位 → `EACCES`，`sudo` 装不上（`unable to open '/etc/sudoers.d/README.dpkg-new': Permission denied`）。`openat` 现在 `O_CREAT` 时补 `0600`，并在「写意图 + `EACCES` + `NR_FAKE_UID=0`」时把目标 `chmod 0600` 重试一次；`NR_DEBUG` 下把失败的 `errno/flags/path` 记进 `<rootfs>/tmp/nr-exec.log`。实测 `dpkg --purge sudo` 后再 `dpkg -i` → `rc=0`、`/usr/bin/sudo` 保持 `-rwsr-xr-x`。已知偏差：`README` 落地 `0600` 而非 `0440`（模式保真待修） | 138 |
| `su` / 换身份 | util-linux 的 `su` 在这台机器上用不了：`runuser -u X -- id` → `cannot set groups: Operation not permitted`（libc 内部 `initgroups()` 直接发 `setgroups`，拦截不到，真实 uid 无 CAP_SETGID）；`su` 无论切到哪个用户都是 `su: System error`，且换掉 `/etc/pam.d/su`、删掉它、插 `pam_echo` 都不改变结果（失败在读配置之前，具体位置 UNKNOWN）。现在 `/usr/bin/su`、`/usr/sbin/runuser` 指向 `husk-su`（Rust，`runtime/crates/su`）：查账户、设 `NR_FAKE_UID`/`NR_FAKE_GID`、exec shell，不需要任何权限。实测 `su - husk -c id` → `uid=1000(husk)`、`HOME=/home/husk`、cwd 在 home；非 root 换别人被拒并说明。App 的初始化因此第一次走完（`INITIALIZATION COMPLETE`），`14-gate-a-acceptance.sh` 在 App 的树上 34 PASS / 0 FAIL | 139 |
| guest 声明的库目录 | `husk-ldconfig --library-path-file /etc/ld.so.husk-path` 递归读 `/usr/bin /usr/sbin /usr/libexec /usr/lib /usr/games /usr/local` 的每个 ELF 的 `DT_RUNPATH`/`DT_RPATH`，把绝对目录去重写出（实测 21 个，含 `/usr/libexec/sudo`、`.../libproxy`、`mutter-14`、`gnome-shell`、`gvfs`、`systemd`）；App 把它接到 `--library-path` 后。生效后 `sudo` 能加载（不再缺 `libsudo_util.so.0`）、`gsettings` 不再报 `libpxbackend-1.0.so`。生成耗时 3.96 s，只在初始化/装完包/App 启动/开会话这四个时机跑 | 141 |
| guest `DT_RUNPATH`（139 时的原始记录） | 不给 `--library-path` 就找不到 `RUNPATH` 目录里的库（sudo 的 `libsudo_util.so.0` 在 `/usr/libexec/sudo`，文件在、搜不到；gio 的 `libpxbackend-1.0.so` 同理）。sudo 起来之后还会拒绝：`/etc/sudo.conf is owned by uid 10265, should be 0`（跨进程的虚构所有权还没设计）。两者都不在门禁 A 判据里 | 139 |
| 服务器表面 | `/proc` 合成、`/sys/class/net`、`/dev/log` 落地；79 PASS / 5 PARTIAL / 6 FAIL | 107 / 109 / 111 / 112 |
| 挂载 | bind、tmpfs、只读 bind 的命名空间等价物（`mount`/`umount2` + 路径层，`/proc/mounts` 一致）；只读对目录描述符同样生效，元数据写入（chmod/chown/utimensat/xattr）同样 `EROFS`，表有锁、存储目录不留残留 | 112 / 113 / 117 |
| 服务监督器 | `husk-init`（freestanding 静态，无 libc）按 `etc/husk/services.conf` 起服务、崩溃按 1→30s 退避重启、`once` 不重启、`on-failure` 只在失败时重启、stdout/stderr 落到 `var/log/husk/<name>.log`、`PR_SET_CHILD_SUBREAPER` 收孤儿、SIGTERM 0.5s 内停且零残留零僵尸 | 120 |
| 进程视图 | `ps -e`/`top`/`free`/`uptime` 只看到 guest 进程，且用真实程序名（`prctl(PR_SET_NAME)` 从 cmdline 取 basename）；`pgrep -x bash` 能命中；宿主 `/proc/<pid>` 读不到 | 119 |
| 设备节点 | `/dev/{null,zero,random,urandom,shm,pts/*,mali0,videoN}` 按名字打开全部正常，`mkfifo` 可用；三条平台墙已定性并写成断言：`/dev` 列不了目录（SELinux，不经 loader 也一样）、`/dev/full` 做不到 `ENOSPC`（要拦 `write(2)`，且 glibc stdio 走内部 `__write_nocancel`，判定为**不支持**）、`/dev/tty` 缺控制终端（`ENXIO`，属终端载体） | 118 |
| 网络表面 | `ip addr`/`ip route`/`ip link`/`ip -br`/`hostname -I`/`/proc/net/{dev,route}`/`if_nameindex`/`getent`/`getaddrinfo` 全通；**出口流量**（DNS/UDP/TCP/HTTP/HTTPS/`apt-get install` 真实下载）可用，改网络的动词回 `EPERM`（不假成功），`ip route get` 与 `ip neigh` 回 `EOPNOTSUPP`；MAC 全 0、无 IPv6、无默认网关、`ss` 空表；原始 ICMP 不可用 | 116 / 121 |
| 账号管理 | 自研 `husk-usermgr`（useradd/groupadd/usermod/userdel/groupdel/chpasswd/passwd/chfn/chsh）；`apt-get install tor` 的 postinst 建号成功 | 111 |
| 安装路径（E 段） | `tools/guest/install-husk.sh`：`--rootfs <tar[.gz|.xz|.zst] | URL> --dest --loader [--user]`，两遍解包补 SELinux 拒掉的硬链接（pass 2 走 shim 的 link→copy 回退），生成 `husk` 启动器（绝对路径 + `NR_LD` 内建）；实测 `PRETTY_NAME="Ubuntu 24.04.4 LTS"`、`bins=294`、`perl5.38.2`/`uncompress` 成为真实文件；`--cwd` 用 `env -C` 落到 guest，`--user hu1` → `uid=1000(hu1)` / `/home/hu1`。**真 Termux 未验（UNKNOWN）** | 130 |
| POSIX 消息队列 | guest 内 `mq_open/mq_send/mq_receive/mq_getattr/mq_unlink` 全通（优先级顺序、`EAGAIN`、`curmsgs` 计数），12/12 PASS；根因是 arm64 glibc 的 `mq_*` 走未实现的 legacy syscall，必须函数级 interpose | 114 |
| SysV IPC | `shmget/shmat/shmdt/shmctl`、`semget/semop/semctl`、`msgget/msgsnd/msgrcv/msgctl` 全部在 shim 里实现（文件 + mmap + flock）：跨进程共享内存、P/V 阻塞唤醒、按类型取消息、`IPC_RMID` 宽限期；13 + 24 项全通过。`SEM_UNDO` = `EOPNOTSUPP`；`/proc/sysvipc/{shm,sem,msg}` 已合成，`ipcs` 出正确表头 | 115 / 116 |

## 桌面帧出屏（2026-11-24：首帧已出屏）

FACT：11:14 的截屏上就是 GNOME 46 桌面（顶栏 / Ubuntu 壁纸 / DING 桌面图标 / 左侧 dock /
通知气泡），会话进程树是 `com.linuxhub.myth.husk` → `libld.so` → `gnome-shell` + DING /
Nautilus / gvfs / tracker。整条链路四个独立故障与修法见 evidence 147；这里保留的是当年
为"不靠 X"而做的协议层调研结论。


- mutter 46 自带 `org.gnome.Mutter.ScreenCast`；已实测能建 session / stream，`state=running`（99 / 102）。
- 已排除：厂商 GPU 用户态库直接进 guest（25 个 bionic `DT_NEEDED`）；
  自研合成器替代 gnome-shell（与产品形态冲突）。
- 现在走协议层：ScreenCast → PipeWire 节点 → dma-buf → AHardwareBuffer → SurfaceControl。
- buffer 类型已有答案（evidence 103）：mutter 46 两条路都支持，**由消费者协商决定**——
  回传的格式带 `SPA_FORMAT_VIDEO_modifier` 就给 dmabuf，不带就给 memfd。零拷贝取决于我们自己。
- 已修掉「有 session bus 时 PipeWire 客户端卡死」（evidence 104）：pipewire 默认走 legacy
  access 模式，会用 `openat("/proc/<pid>/root")` 判断是不是 flatpak；我们的 shim 有意挡住
  `/proc/*/root`，内核回 `ENOENT` 而不是它预期的 `EACCES`，于是它把客户端当 flatpak 永久挂起。
  改用 `access.socket = unrestricted`（配置文件 `tools/guest-config/pipewire.conf`）后，
  `PipeWireStreamAdded` 正常到达、Produce 节点（`node=Some(32)`，`Stream/Output/Video`）可见，
  `EnumFormat` = BGRx/BGRA 1280x720。

## 下一步（按环境完整性口径排序）

1. **F：页大小已收口**——真 16K（RPi5 kernel）与真 64K（Ubuntu 64k 内核）内核上端到端全绿
   （evidence 125 / 126）；`ls -l` 多一行 stderr 的根因已修（xattr/readlink 裸 -errno）。
   剩 16K/64K **真机** UNKNOWN（本机只有 4 KiB），不是代码问题。
2. **F：GPU 家族适配器**——本机只验了 Mali(kbase)。Adreno(kgsl + Turnip) 与通用 DRM
   按 `docs/design/device-adapters.md` 的表结构补适配器，没设备的一律 UNKNOWN。
3. **2.0 集成：运行时替换已完成**（evidence 140–145；proot 及其补丁已全部删除，
   `/sdcard` 共享存储已补回并在 App 域实测——B 段的已知缺口只剩"`/storage` 只暴露自卷"）。
4. **G：首帧已出屏**（evidence 147）——GNOME 46 的完整桌面在 2.0 的树上跑起来了
   （顶栏 / 壁纸 / DING 图标 / dock / 通知，会话全程 `untrusted_app`）。卡了很久的四个
   独立故障都修在 shim 里：`RLIMIT_NOFILE` 天花板（`setrlimit` 钳位）、容器不能给自己人
   发信号（kill 家族透传）、GDBus 的 `SCM_CREDENTIALS`（`sendmsg` 改写 `ucred`）、
   系统总线 `EXTERNAL` 认证（改 `ANONYMOUS`）；外加 2.0 侧 X socket 路径错位。
   僵尸也一并修了（kernel 4.19 没有 `waitid(P_PIDFD)`，GLib 的 child watch 于是不回收——
   shim 里按 `/proc/self/fdinfo` 把 pidfd 翻成 pid，100 s 后 0 僵尸，见 147 §7.1）。
   剩下的都是"桌面能不能用"的问题，不是"能不能上屏"：**桌面收不到输入**（`X11Runtime`
   根本没有注入指针/键盘的入口，147 §8）、`polkitd` 退出 1、gvfs 点对点总线、at-spi、
   xrandr scaling。

## 已知环境细节（踩过的坑，别再踩）

- loader 只接受**相对**程序路径（`usr/bin/bash` ✓，`/usr/bin/bash` ✗）。
- **启动契约**：`NR_ROOTFS=<rootfs 绝对路径>` 与 `NR_LD=<我们的 loader 绝对路径>` 两个变量必须同时设。
  少 `NR_ROOTFS` → 父进程完全不改写（症状 `command not found`）；少 `NR_LD` → exec 出来的
  子进程回落到 rootfs 里的原厂 glibc loader，shim 不在里面（症状：父进程路径对、子进程读宿主路径）。
  排查"shim 好像没生效"先看这两个变量，别先怀疑内核或页大小（2026-11-23 踩过）。
- `run-as` 下不能写 `/tmp`，临时文件放 `$R/tmp/`。
- `dbus-daemon` 在 Android 上要先解决两件事（2026-11-24 查清，见 evidence 147）：
  1. 它每次建连接都会 `setrlimit(RLIMIT_NOFILE, 65536)`，而硬上限是 32768、无 CAP_SYS_RESOURCE，
     失败即把所有客户端判成 AccessDenied（症状：socket 在、daemon 在 epoll_wait、客户端 11 ms
     就 "Did not receive a reply"）。shim 已钳位。
  2. 私有系统总线必须用 `<auth>ANONYMOUS</auth>`：假身份下 `EXTERNAL` 永远对不上
     （假 root：uid 0 vs SO_PEERCRED 10265；真身份：没有 passwd 条目）。
  `--fork --nopidfile --print-pid=1` 是可用的；早先"`--fork` 会堆出多个 daemon"是误判，
  真正原因是当时容器杀不掉进程（kill interpose 已修）。
- `--library-path` 是替换不是追加，必须显式带 `$R/lib/aarch64-linux-gnu` 与 `.../gvfs`。
- rootfs 缺包会导致与运行时无关的怪故障，清单见 `docs/design/rootfs-packages.md`
  （gdk-pixbuf 的「所有图片格式都不认识」就是缺 `shared-mime-info`）。
