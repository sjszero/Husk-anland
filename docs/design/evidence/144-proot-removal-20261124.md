# 144 · 删掉 proot：同一个 rootfs，功能不减

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`（debug），
空 checkpoint（无 `container.properties` / 无 `distributions`）从 App 界面驱动。

## 0 · 结论（FACT）

LinuxHub 2.0 的 APK 里**不再有 proot 的任何东西**，容器仍然完成全量 dpkg 配置：

| 判据 | 结果 |
|---|---|
| 包状态 | 814 包 `install ok installed`，`unpacked 0` |
| `dpkg -C` | 空（前后都查） |
| `Bad system call` | 0 |
| seccomp 陷阱 | 1,181 × `nr=264 name_to_handle_at` + 12 × `nr=452 fchmodat2`（=1,193 条），全部被 shim 接住 |
| 初始化总时长 | 542.8 s（删 proot 前两次：533.2 s / 541.3 s） |
| 门禁 A 验收 | 34 PASS / 0 FAIL（`EXPECT_PKGS=814`） |
| 服务监督器 | 8 PASS / 0 FAIL |
| 跨进程所有权 | `/etc/passwd`、`/usr/bin/sudo` = 0:0；`/home/husk/*` = 1000:1000；`owners` 31 行 |
| guest 声明库目录 | `/etc/ld.so.husk-path` 22 项，含 `/usr/libexec/sudo` |

```text
1795485471200 START EXTRACT_SYSTEM          3.7 s
1795485474888 START CREATE_CONTAINER      514.1 s
1795485988950 START CREATE_USER             1.0 s
1795485989967 START CONFIGURE_SUDO          3.1 s
1795485993047 START INITIALIZE_ENVIRONMENT 16.2 s
1795486009293 START SAVE_CONFIGURATION      4.7 s
1795486013992 INITIALIZATION COMPLETE     542.8 s total
```

同一次初始化里 loader 起了 **10,379 个 guest 进程**（`sigsys handler installed` 计数），
这是 dpkg postinst 的真实 fork 量，也是"每进程装钩子"这条设计的压力值。

## 1 · 删了什么（LinuxHub-2.0，两个 commit）

`24851531 husk: the container runs on Husk, so proot leaves the build`

- `LinuxContainerManager`：`loginCommand` 只剩 `exec /bin/bash -l`（假 uid 由 loader 导出，
  不再 `su`）；删 `ensureAndroidGroupNamesScript()`（proot 的 `getgroups` 泄漏，evidence 129）、
  `prootCommandLegacy`、`PROOT_LOADER`/`PROOT_TMP_DIR`/`loginEnvironment` 的 proot 分支、
  `sessionWorkingDirectory` 的 proot 分支、`installRuntime` 里的 getifaddrs 安装/启动、
  `environmentScript` 的 `LD_PRELOAD` 拼接、`prepareContainerMounts()`、
  `androidStorageBindings()`、`bridgeProcess`/`runtimePreparedRevision`/`bridgeLock`。
- `ContainerPaths`：删 `prootTmpDir`、`sharedMemoryDir`、`compatibilityProc*`、
  `compatibilityPowerSupplyDir`、`proot`、`loader`、`getifaddrs*`、`libraryDir`、
  `talloc`、`androidShmem`、`x11SocketDir`、`desktopScreenshotDir`。
- `HuskRuntime`：删已无调用方的 `asUser`/`loginShell`；`command()` 在 loader 缺失时
  直接报错（`Husk 运行时缺失：<path>`），不再存在"静默回退 proot"的语义。
- `GetifaddrsBridge.kt` + `GetifaddrsBridgeTest.kt` 删除；`assets/runtime/**`、
  `jniLibs/{libproot.so,libproot-loader.so,libexec_getifaddrs_bridge_server.so,libtalloc.so,libandroid-shmem.so}`
  删除（APK 里已无匹配条目，`unzip -l` 实测）。
- UI 上的 `PRoot` 标签改成 `Husk`。

`35563003 proot: the vendored runtime and its bridge leave the repository`

- `tools/proot/`（vendored upstream + `build.sh` + `include/`）、`tools/getifaddrs_bridge/`、
  `rust/getifaddrs-bridge/` 全部删除；没有 Gradle 任务引用它们（查过 `*.gradle.kts` 与 jniLibs 过滤）。
- `NOTICE` / `ACKNOWLEDGEMENTS.md` 不再把 PRoot 列为分发组件。

## 2 · 每一块为什么可以删（替代能力都在 shim 里，已验）

| proot 的东西 | 现在的来源 |
|---|---|
| `-r /` | `NR_ROOTFS`（shim 改写路径） |
| `-0` + `su - user` | `NR_FAKE_UID`/`NR_FAKE_GID`；实测 `id` → `uid=1000(husk) gid=1000(husk) groups=1000(husk)`，`HOME=/home/husk` 可写，`sudo` 仍要密码 |
| `-b /dev -b /proc -b /sys` | shim 自己合成（evidence 112/116/121） |
| `/proc/stat`、`/proc/version`、`/proc/bus/pci/devices`、`/sys/class/power_supply/BAT0` 兼容文件 | 同上，宿主不需要再写 |
| `--link2symlink` | shim 的 `link()` 退化成复制 |
| getifaddrs 桥（server + `LD_PRELOAD` client） | shim 直接 interpose `AF_NETLINK`（evidence 116/121） |
| `-b /storage/emulated/0:/sdcard` | guest 与宿主同一命名空间，`/sdcard` 走真实路径（本次未单独验收） |

## 3 · 唯一的语义改变：`/run/doro-audio` 变成 rootfs 里的真目录

proot 用 `-b <app>/tmp/audio:/run/doro-audio` 把宿主目录挂进 guest。Husk 下目录是"真的"，
所以 `ContainerPaths.audioDir` 从 `baseDir/tmp/audio` 改成 `rootfs/run/doro-audio`
（`HuskRuntime.guestDirs` 本来就会建这个目录）。于是：

- 宿主的 `ContainerAudioBridge` 在 rootfs 里 `mkfifo`，guest 的
  `module-pipe-sink file=/run/doro-audio/pulse-output.pcm` 打开的是**同一个** FIFO；
- 宿主写的 `/run/doro-audio/battery-capacity` 也正是电池小程序 `cat` 的那个路径。

实测（本次运行，`ls -l rootfs/run/doro-audio`）：

```text
-rw------- 1 u0_a265 u0_a265 4 battery-capacity      # 值 100，由宿主写入
prw------- 1 u0_a265 u0_a265 0 pulse-output.pcm      # 宿主建出的 FIFO
```

## 4 · 未闭环（不冒充）

- **GNOME 桌面首帧仍然没有画面。** 点"打开桌面"后会话确实起来了（`:x11` 进程在、
  `doro-startup-stage.log` 有 `stage=outer-root`、FIFO 建出），App 在 45 s 后报
  "GNOME 桌面首帧超时：画面仍未出现（可见像素 0%）"并给出返回按钮。
  **删除前从未在这台机器上启动过桌面**（140/143 只记录 App 初始化），所以
  "这一失败与删除 proot 无关"目前是 UNKNOWN——没有基线可比，不写结论。G 段仍按目标延后。
- `sudo` 的 PAM/audit 失败（`ENOTSOCK`）沿用 143 的状态，不在门禁 A 判据里。
- 假 uid 只改"报告值"，内核的权限检查仍按 App 的 uid 做：guest 里 `uid=1000` 仍能读
  `/etc/shadow`（文件属宿主 App uid）。这是本方案固有的边界，不是本次改动引入。
- **`/sdcard`（外部存储）未单独验收**：proot 以前靠 `-b` 提供，现在只靠「命名空间同一」这个前提。
  门禁 A 只验了 `/data/user`、`/system` 在 guest 里不存在；`/sdcard` 读写在 Husk 下的真实行为是 UNKNOWN，
  归到 B 段（挂载 / 外部存储）补测。
- 每次桌面初始化仍留一个 `[virgl_test_serv]` 僵尸（父进程 = App，原因在无源码的
  `libdoro_virgl_process.so`，见 143 §6）。

## 5 · 复现

```sh
# 空 checkpoint：把 distributions 挪走并删掉状态文件，然后从 UI 走完向导
run-as com.linuxhub.myth.husk sh -c 'cd files/linux && mv distributions distributions.attemptN && rm -f container.properties initialization.properties'

# 门禁 A（宿主侧跑，脚本自己 push 进 guest）
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs LD=files/linux/husk/libld.so \
EXPECT_PKGS=814 ./tools/device-tests/14-gate-a-acceptance.sh

# 服务监督器（先 push husk-init 到 /data/local/tmp）
run-as com.linuxhub.myth.husk sh -c 'FILES=/data/data/com.linuxhub.myth.husk/files \
ROOT=linux/distributions/ubuntu-24.04/rootfs LD=linux/husk/libld.so sh /data/local/tmp/12.sh'
```

APK（本次实测的那一个）：1,050,179,937 B，
`sha256 30ff02f6dce4818147e90f834611d445ac1bb8322e83e8396eb55bca10e8a909`。
