# 当前状态

> 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64），
> 运行域 `untrusted_app`，无 root、无 Shizuku。
> 只写**已上机验证**的结论；未验证写 UNKNOWN。每条的原始输出在 `docs/design/evidence/`。

## 接手须知（冷启动只读这一节 + 本文件的表）

**设备**：一台 arm64 Android 手机（RMX3357）；序列号不入库，用 `adb devices` 的当前值。
当前真机验证使用 `com.linuxhub.husk`；rootfs 在
`files/linux/distributions/ubuntu-24.04/rootfs`，loader 在 `files/linux/husk/libld.so`，
资产版本标记见 `files/linux/husk/assets.version`。

**启动契约（两个变量必须同时设，缺一个就"看起来像 shim 坏了"）**：

```sh
NR_ROOTFS=<rootfs 绝对路径> NR_LD=<我们的 loader 绝对路径> NR_LIBRARY_PATH=<完整库路径>   <loader> --library-path "$NR_LIBRARY_PATH"   usr/bin/bash -c '...'          # 程序路径必须相对
```

**构建 loader**：`NDK=<ndk>/bin OUT=/tmp/x sh tools/loader/build-embedded-loader.sh`
（产物覆盖 `tools/loader/ld-embedded.so`，`nr-tools embed` 自带页对齐断言）。

**16K/64K 真内核环境**（QEMU，别重建）：`/tmp/k16`（`Image.rpi5` + `root/` 的 initramfs 源），
命令见 `docs/design/evidence/125`。一次启动 8–15 秒。

**仓库布局**：`runtime/crates/{path-shim,nr-tools,runtime-core,gpu-env}`（Rust）、`tools/loader/*.S`（裸汇编，仅入口与 hook）、
`tools/device-tests/*.sh`、`docs/design/`（设计 + `evidence/` 原始记录）。

**身份初始化单写者（同一候选）**：`NR_FAKE_UID/GID` 在 loader 的 boot hook 中、
任何构造器和 guest 线程启动前一次播种；运行期 `getuid/getgid` 不再懒初始化。
身份登记锁记录 owner TID，同一线程的信号处理器重入时直接返回，不会自旋自锁。
身份压力探针让 15 个读线程、1 个写线程和高频 `SIGALRM` 重入并发执行；新 loader
已在真机通过 300 个新鲜进程，门禁 A 为 `PASS=38 FAIL=0`。旧代码未稳定复现该窗口，
动态压力不能替代竞态检测器（evidence 264）。

**App 直接 PTY 与全部 fork 子路径（同一候选）**：`Term::spawn()` 已和 shell session
共用 locale 选择规则，不再硬编码 `LANG=C.UTF-8`。guest PTY leader、会话 guard、SIGSYS
启动自检和 trace 诊断四条 fork 子路径都在父进程准备参数，子进程不再分配 `Vec`/字符串。
`make check`、四路径静态门禁和真机门禁 A 已通过，APK 已完整构建；旧测试包
不能原地覆盖为新包，App 动态 fork 压力仍为 `UNVERIFIED`（evidence 263、265）。

**0.0.32 文件模式、所有权与 access 主体**：`O_CREAT` 不再偷偷追加 `0600`；dpkg 重开
`0440` conffile 时只临时加 owner-write，返回 fd 前恢复原模式。`fchown(fd)` 现在会更新虚构
所有权表，`fstat(fd)` 与后续进程的路径 `stat` 都读得到。`access/faccessat` 改按 guest 身份
计算，并逐级检查父目录 X 权限；只读挂载上的 `W_OK` 仍为 `EROFS`（evidence 259）。完整门禁 A
为 `PASS=38 FAIL=0`，coverage-sweep 1400 条无泄漏，dpkg `0440` conffile、V4L2、DRM 和
fd identity 全通过。`make check` 检查 73 个 shell 脚本、284 个 Markdown 文件。

**0.0.33 已发布（2026-11-26）**：补充组语义已落到 `libld.so` 发布资产。假用户从
guest `/etc/group` 自动得到 `[1500, 4242]`，`setgroups([4242])` 后为 `[4242]`，
`initgroups()` 后为 `[1500, 4242]`；组权限文件的 `access(R_OK/W_OK)` 也按补充组判断。
真机门禁 A `PASS=38 FAIL=0`，挂载专项 `PASS=18 FAIL=0`；coverage-sweep 因 rootfs 没有
 `strace` 为 `SKIP`，不能算通过（evidence 260）。0.0.33 已按版本流程生成并发布
 `v0.0.33` / `Husk 0.0.33` / `husk-arm64.tar.gz` / `SHA256SUMS`。

**GPU 软件逃逸与可选 D-Bus（0.0.33）**：`HUSK_GPU=llvmpipe`、
`NR_FORCE_SOFTWARE_GPU=1` 和真值 `NR_GPU_DEFAULT_OFF` 现在都穿过 session 的 `env -i`；
loader 在软件模式下强制 Mesa 软件驱动并隐藏父环境继承的 PanVK/Vulkan 选择器。修前父环境
已继承 PanVK 时，孙子进程显式切软件仍报 `zink Vulkan 1.3(Mali-G77 MC9)`；修后同一入口报
`llvmpipe`，默认入口仍是 PanVK。最小 rootfs 缺少 `dbus-daemon` 或 bus 配置时 session 直接
跳过，不再空等六秒。完整门禁 A `PASS=38 FAIL=0`，coverage-probe PASS，coverage-sweep 因
rootfs 没有 `strace` 为 `SKIP`（不能算通过）；证据见 277。

**GPU 全局运行时审计（0.0.34）**：修正桌面自检把 `VK_ICD_FILENAMES` 是否出现在
`/proc/<pid>/environ` 当成 GPU 判据的错误；loader 的默认值只回答 `getenv`。现在以 zink 环境、
真实 `/dev/mali0`（或 Adreno `/dev/kgsl-3d0`）fd 和已加载 PanVK/Freedreno 后端共同判定。
真机 `glxinfo -B` 报 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` / `Accelerated: yes`；
GNOME、Nautilus 和 loader 新起的嵌套 GTK 应用均通过 `make gpu-audit`。GTK4 仍默认
`GSK_RENDERER=cairo`，所以本版不宣称 GTK UI 已全部 GPU 化（evidence 278）。

**XKB 与 PanVK 加载器默认值（同一候选）**：loader 在 guest 未显式设置
`XKB_CONFIG_ROOT` 时返回 `<rootfs>/usr/share/X11/xkb`，覆盖登录 shell、桌面 fork、
`env -i` 和嵌套 exec；guest 内 `getenv`/`secure_getenv` 探针得到同一路径，显式值仍优先
（evidence 272）。Mali 设备的 `/etc/husk/gpu-env` 即使还是旧格式、缺少
`PANVK_DXVK_COMPAT`，loader 也会在内存中补 `0`；`/usr/bin/env`、`getenv` 和
`secure_getenv` 都验证通过，显式 `=1` 仍优先（evidence 273）。同一候选真机门禁 A 为
`PASS=38 FAIL=0`，coverage-probe PASS，`gl-probe fixed` 与 `gl-probe es3` 均 PASS。

**会话 guard 清理真实性（同一候选）**：`marker_pids()` 的 `/proc` 扫描失败不再返回空
列表冒充成功；最终无法确认时写 `remaining=UNKNOWN` 并以 70 退出，只有扫描成功、无跳过项
且进程组消失才写 `clean=true`。guard 启动的 fd 3/4 映射改为先 `F_DUPFD_CLOEXEC` 复制，
ready 超时会 SIGKILL 卡住的 helper。真机 marker 清理 PASS，缺失 proc 根路径负对照返回 70，
新增 guard 探针已接入门禁 A（evidence 267）。

**组表并发越界写（同一候选）**：`getgroups()` 的长度读取、缓冲区校验和复制现在位于
同一 owner-TID 锁区间，不再允许并发 `setgroups()` 在检查与复制之间扩大组表。旧 loader
负对照 20 轮中 19 轮出现 `buffer-overrun`/stack smashing；新 loader 20/20 轮通过，并接入
门禁 A。信号处理器同线程重入时返回只含主组的一致快照，不读取半更新表（evidence 266）。

**session 中文默认值（同一候选）**：`husk-session.sh` 的 `env -i` 边界不再固定写
`LANG=C.UTF-8`。rootfs 带 `zh_CN` locale 时默认 `zh_CN.UTF-8` / `zh_CN:zh`，调用方显式
设置的 `LANG`/`LANGUAGE` 保留；真机探针同时验证默认值和 `en_US.UTF-8` 覆盖（evidence 261）。

**最终 symlink 并发替换（同一候选回归）**：新探针在 guest 内持续把最终 symlink 在 guest
文件和 App 私有宿主绝对路径之间替换，同时并发执行 `chmod`、`utimensat`、`chown`、`open`
和 `lstat`。真机 212,050 次操作、12,860 次替换后，宿主哨兵的内容、mode 和 mtime 均未变化
（evidence 262）。这关闭了 evidence 64 留下的动态验证缺口，不代表 raw syscall 或静态 ELF
也成为受控路径。

**0.0.31 V4L2 假节点 open 标志**：path-shim 对 `/dev/video0` 返回的假 fd 现在保留
`O_CLOEXEC` 与 `O_NONBLOCK`。旧 0.0.30 loader 的负对照是 `fdflags=0 fl=131074`，修后为
`fdflags=1 fl=133122`（evidence 258）。同一轮把 netlink 的
`SOCK_CLOEXEC | SOCK_NONBLOCK` 加入 fd identity 门禁，真机结果 `fdflags=1 fl=2050`，核过不是
缺陷。完整门禁 A 为 `PASS=38 FAIL=0`，fd identity 17/17；V4L2 修复进入发布包 `libld.so`。

**0.0.30 DRM 假节点 O_CLOEXEC**：path-shim 与 preload 对 `/dev/dri/renderD128` 的假 fd
现在保留调用方传入的 `O_CLOEXEC`。旧 loader 的负对照是 `fdflags=0`，修后真机两项都是
`fdflags=1`（evidence 257）。同一轮修复了 kbase 初始化失败时的 mali fd 泄漏和 errno
丢失；失败分支没有人为注入，标为 UNVERIFIED。path-shim 的修复进入发布包的 `libld.so`，
独立 `libdrm_preload.so` 仍不在发布 tar 中。

**0.0.29 DRM preload mmap/ioctl 失败语义**：`libdrm_preload.so` 的 `mmap` 失败现在返回
`MAP_FAILED` 并保留 `EBADF`，不再返回 `NULL`/`EPERM`；普通 fd 的 `ioctl` 失败保留
`ENOTTY`，DRM 回调则拿到原始负 errno。旧 0.0.28 负对照为 `mmap rc=0 errno=1`、
`ioctl rc=-1 errno=1`，修后为 `-1/9`、`-1/25`（evidence 256）。非 DRM 路径隔离和
Adreno/KGSL 真机仍是 UNKNOWN。

**0.0.28 DRM preload open ABI**：`libdrm_preload.so` 的 `openat` 失败不再把 glibc 已设置的
`ENOENT` 改写成 `EPERM`；`open` / `open64` / `openat` / `openat64` 都保留创建 mode，
`O_CREAT` 文件不再固定创建成 000。旧 0.0.27 负对照为 `errno=1` 和 `mode=0`，修后为
`errno=2`、`mode=640`（evidence 255）。路径 shim 不在场时的非 DRM 路径翻译边界仍是 UNKNOWN。

**0.0.27 DRM preload fd 覆盖身份**：`libdrm_preload.so` 不再用私有 fd 号码表认定假 DRM。
每次检查都读取 `/proc/self/fd` 的内核目标，因此 `dup2` 用普通文件覆盖旧假 fd 后会退出 DRM
模拟层；假 fd 复制到新号码、源关闭后仍能识别。旧 0.0.26 `.so` 的负对照为
`before=1 after=1`，修后为 `before=1 after=0`，复制保留方向 `PASS`。DRM preload 已接入
`make check`、Clippy、CI 和 fd identity 真机门禁（evidence 254）。Adreno/KGSL 真机、
跨进程 `SCM_RIGHTS` 与多线程覆盖压力仍是 UNKNOWN。

**0.0.26 V4L2 fd 覆盖身份与零引用回收**：V4L2 不再按 fd 号码认定相机；`dup2`/`dup3`
用普通文件覆盖假 `/dev/video0` 后，`VIDIOC_QUERYCAP` 会离开模拟层并返回普通文件失败，
不再伪报 `driver=husk`。系统调用成功后 path-shim 会清核目标号码的旧 V4L2 设备；零 live
memfd 引用时 camera.sock 正常 EOF。旧 loader 负对照为 `QUERYCAP=Ok("husk")`，新 loader 的
V4L2 `dup2`/`dup3`、假 fd 复制、camera EOF 全部 PASS；netlink 三方向也 PASS（核过不是缺陷）。
完整门禁 A、coverage 与 DRM / V4L2 既有回归结果见 evidence 253。跨进程 `SCM_RIGHTS` 与
多线程覆盖压力仍是 UNKNOWN。

**0.0.25 DRM / Adreno fd 覆盖记账**：`dup2`/`dup3` 用普通文件覆盖假 DRM fd 时，目标号码现在
会先从 `FAKE_FDS` 移除；源 fd 是假 DRM 时仍会重新登记，因此副本语义不变。`ADRENO_FDS` 同步
采用先清目标、再判断 KGSL 源的顺序（evidence 251）。Mali-G77 真机的 `drmfake` 结果为
`dup2-ordinary-over-fake|PASS`、`dup3-ordinary-over-fake|PASS`、`dup2-fake-preserved|PASS`；
旧 loader 负对照为 `ioctl=Ok(())`，证明旧表项确实会伪成功。完整门禁 A 为 `PASS=38 FAIL=0`。
Adreno/KGSL 真机与跨进程 `SCM_RIGHTS` 仍是 UNKNOWN。

**0.0.24 V4L2 高 fd 复制与上下文锁**：假相机 fd 的引用计数改为枚举 `/proc/self/fd`，
`dup()` 到 1024 以上后关闭原 fd 不会提前拆掉仍存活的 camera socket；path-shim 与 preload 的
V4L2 上下文访问已串行化。Rust 探针 `v4l2-ref-probe` 在 guest 内完成
`open -> dup(1100) -> close(original) -> QUERYCAP(high) -> close(high)`，输出
`socket=alive`、`ioctl-high rc=0`、`last-close socket=eof` 和 `V4L2_REF|PASS`（evidence 250）。
真实 Camera2 连续取帧、高 fd 下的 STREAMON/DQBUF 组合与长时并发压力仍是 UNKNOWN。

**0.0.23 netlink 复制 fd 生命周期**：假 netlink socket 现在按 client / peer 的 socket inode
识别，`dup`、`dup2`、`fcntl(F_DUPFD_CLOEXEC)` 产生的新 fd 仍保持 `AF_NETLINK` 语义；
关闭原 fd 不会拆掉其他复制 fd 共用私有 peer，最后一个引用关闭后才清理。真机探针输出
`families=16,16,16,16`，逐级关闭后仍为 16，最终 fd 回到基线（evidence 249）。
`SCM_RIGHTS` 跨进程传递假 netlink fd 与多线程 dup/close 竞态仍是 UNKNOWN。

**0.0.22 `close_range` 与私有 fd 记账**：Android app seccomp 拦截 syscall 436 后，
`close_range()` 过去直接返回 `ENOSYS`。现在 path-shim 枚举 `/proc/self/fd`，按真实上界执行原始
`close(57)`，跳过内部 root fd；普通关闭、`CLOEXEC`、`3..UINT_MAX` 都在真机探针里通过。
单 fd 与批量关闭共用 V4L2 / netlink / DRM / Adreno 注销逻辑，netlink 私有 peer 也随 client
一起关闭。`CLOSE_RANGE_UNSHARE` 明确返回 `EOPNOTSUPP`，没有伪成功。完整门禁 A 为
`PASS=38 FAIL=0`，coverage-sweep 扫 1400 条候选路径无泄漏（evidence 248）。
用户态 `realpath` / `glob` / `nftw` / `statvfs` 当前均正确落到 guest 路径（evidence 247）。

**0.0.21 stdio 硬链接刷新**：`fclose()` 已接入 path-shim 的硬链接别名刷新。应用调用
`fopen` / `fputs` / `fclose` 写入时，先 flush 流，再按 `fileno` 刷新复制后备，最后调用真实
libc `fclose`。coverage-probe 输出 `content=AAABBB`，完整门禁 A 为 `PASS=38 FAIL=0`，
coverage-sweep 因 rootfs 没有 `strace` 记为 `SKIP`（evidence 246）。硬链接仍不是内核级语义，
link count / 共享 inode / 跨进程即时共享仍未恢复；绕过导出 `fclose` 的内部路径仍是 UNKNOWN。

**0.0.20 真实 X11 GPU 验证**：在同一台机的现有 X11 server 上，用独立 guest 客户端复验
`glxinfo` / `glxgears`：`Accelerated: yes`、renderer `zink Vulkan 1.3(Mali-G77 MC9)`，
进程实际持有 `/dev/mali0` fd；`glxgears` 连续 600 秒，119 个 5 秒窗口、147872 帧，
215.779–299.303 FPS、平均 248.352 FPS，`RC=124` 是外部 `timeout` 到点（evidence 245）。
边界同时钉死：当前 2.0 拉起的 GNOME 进程仍是
`GALLIUM_DRIVER=llvmpipe`、`MESA_LOADER_DRIVER_OVERRIDE=swrast`、`LIBGL_ALWAYS_SOFTWARE=1`，
日志约 1.0–1.2 FPS、`0/0 present copies offloaded to GPU`。所以可以说“Husk 的真实 X11 GPU
客户端路径已验证”，不能说“2.0 桌面已经全局 GPU”；硬件 GNOME 和桌面并发仍是 UNKNOWN。

**0.0.19 fixed-function PanVK 默认规避**：`gl-probe fixed` 在不启动 X11 的情况下复现了
`glxgears` 的 Bifrost `fa=0x21` 崩溃，确认问题在 fixed-function 转译产生的内部 GS。Mali 默认
`PANVK_DXVK_COMPAT=0`，并让 loader 默认值穿透真实 `setenv(..., 0)` 与 `execve(..., envp)`。
真机默认配置 600 帧 `PASS`（588.5 fps），ES3/Vulkan 回归通过；显式 `PANVK_DXVK_COMPAT=1`
仍 `RC=139`。这是默认值规避，不是 PanVK 编译器修复；DXVK 需要显式 opt-in，真实 X11
`glxgears` 和桌面长跑仍是 UNKNOWN（evidence 244）。

**0.0.18 GPU 默认值统一与 Adreno 启动级路径**：GPU 默认值生成从 App 的 `term.rs` 抽成纯 Rust
`gpu-env` crate，Mali 与 Adreno/KGSL 在 App 侧共用同一套 canonical ICD、原子配置和软件回退逻辑。
App 只有 `/dev/kgsl-3d0` 可读写且 Freedreno/Turnip 驱动存在时才生成 Adreno 配置；有原生
`kgsl_dri.so`/`freedreno_dri.so` 时走 kgsl/freedreno，否则走 zink。5 个主机侧单测通过；
最终 0.0.18 APK 在 Mali 真机删除配置后冷启动重建，`env -i` 仍为 PanVK/Zink，`HUSK_GPU=llvmpipe`
正确回退（evidence 243）。Adreno 真机链路仍是 UNKNOWN。

**0.0.17 GPU 默认值下沉到 loader 版**：App 只在 `/dev/mali0` 真实可打开且 PanVK 驱动存在时，
自动生成 guest `/etc/husk/gpu-env` 和 canonical ICD；这样经 Husk loader 启动的进程即使
`env -i`、完全没有 GPU 变量，也会由 loader 注入 Mali/Zink/GLES 3.2 默认值。删除两份配置后
冷启动可自动重建；`env -i` 的 `gl-probe es3` 报
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`、`RESULT PASS`，加 `HUSK_GPU=llvmpipe` 则正确
回退到 llvmpipe（evidence 242）。同时增加 Mali 设备门控，避免仅凭 rootfs 文件就把其他 GPU
家族改成 Mali 配置。

**0.0.16 PanVK ICD 路径修正版**：0.0.15 在独立验证树上看到的 `vulkan-probe` /
`gl-probe` SIGSEGV，根因确认是 ICD JSON 的 `library_path` 使用了宿主 `/data/data/...`
路径，不是 PanVK 驱动版本旧。App、安装器、登录 shell 和 session 脚本现在统一使用自动生成的
guest 相对 manifest `usr/share/vulkan/icd.d/husk_panvk_guest.json`，且只有真实驱动存在时才
启用。真机冷启动后不手工 export，初始 guest 环境直接指向该 manifest；`vulkan-probe`
枚举 `Mali-G77 MC9` 并跑通 compute/graphics，`gl-probe es3` 报
`zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`、`RESULT PASS`（evidence 241）。
X11 SHM 回读和 `glxgears` 编译器崩溃是另外两条问题，不在本修正范围内。

**0.0.15 相机默认启用与接口口径版**：相机桥不再要求 `files/camera` / `files/camera-guest`
测试 marker，App 每次启动都按活动 rootfs 建 `camera.sock` 和 V4L2 影子树；权限被拒时仍由
socket 返回明确拒绝。独立验证包上删除 marker 后冷启动，`camera.sock` 自动出现，最终 0.0.15
构建上前后摄复跑落在 12.26–17.42 fps（evidence 240）。同时更正旧文档：guest `/dev/videoN` 是标准 V4L2
facade，`open()` 返回 shim memfd，帧经 socket 来自 App 内 Camera2，**不是真节点直连**。
App 初始终端会话补齐 `HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF=1` 回退和
`MESA_GLES_VERSION_OVERRIDE=3.2`；真机初始会话环境已核到这些值。

**0.0.14 发布前补审版**：在 0.0.13 全链路审计基础上，补修合成 VFS 并发、虚构身份状态、
APK 版本元数据和 NDK 构建入口，共记录 **28 条已修缺陷（H-01…H-28）**。
逐条有真机或静态证据，索引见
`docs/design/evidence/226-audit-inventory-0.0.13.md`。`/proc/self/exe` 改报 guest 路径；
`__xstat` 家族补 6 个入口；`dlmopen`/`execveat` 两个真缺口收掉（覆盖 90 → 98 / 111）；
挂载表重读、死项清理和 `remount,ro` 修复；音频桥并发、播放和采集协议真机通过；X11 输入
全屏和双客户端各 5/5 命中。门禁 A 为 **38/38**。

**未闭合、不得写成通过**：libc 体内跳板（L2）仍是 0 覆盖；
`adb input swipe` 到 XI2 的事件路径 UNKNOWN；GPU 的 fixed-function GS 只做了默认规避，
显式 `PANVK_DXVK_COMPAT=1` 仍会 SIGSEGV，真实 X11 `glxgears` 和桌面长跑未验证
（evidence 232、244），所以不能宣称 GPU 长跑通过。
跨设备仍只有 Mali-G77 / Android 13 / 4 KiB 真机证据。机制账见 `docs/design/compat-layer.md`，
限制见 `docs/LIMITATIONS.md`。

**当前开口（按优先级）**：
1. 非 Mali GPU 家族**适配器**（Adreno kgsl+Turnip / 通用 DRM）：探测表与降级已落地（evidence 127），
   适配器本身等有设备再写——没设备只能出未验证代码；
2. 传感器（中转，NDK `ASensorManager_*` → IIO 影子树）已在本机验证 5 类默认传感器、guest 三路径读取与节点枚举（evidence 302）；定位 / USB / 蓝牙（中转，卡在 `hasCode=false` 的 Java 回调）、NFC / 指纹（不需要）——四类口径与逐条理由见 `docs/design/hardware-interface-map.md`；
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
4. **GPU：桌面已经在 Mali 上由 GPU 出帧，默认生效，但呈现仍是拷贝路径**（evidence 165/167/168）：
   重编 PanVK 打开 x11 平台（`-Dplatforms=x11`）后 `VK_KHR_xcb_surface` 进表，
   `gl-probe window` 的 `stage 7/8/9/10` 全过、60 次 swap，
   `renderer=zink Vulkan 1.3 (Mali-G77 MC9 (MESA_PANVK))`；gnome-shell 46 在 zink 上起来并
   出帧（X 根窗口 `2400x1080`、非黑像素 100%）。mutter 启动时的 MIT-SHM `BadAccess` 已定位
   并绕开：guest 没有 SysV IPC，X server 在 guest 外，`ShmAttach` 必然失败，所以 guest 侧
   如实报告"不支持 MIT-SHM"。
   **默认值不再依赖会话脚本，也不再依赖登录 shell**：安装器写 `/etc/husk/gpu-env`，由 loader
   在程序自己没设置时补（走 `getenv`/`secure_getenv` 符号，不是 syscall 拦截），所以 cron、
   桌面 fork 出来的进程、`env -i` 看到的是同一套渲染器——`env -i` 起 `gl-probe es3` 报
   `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，门禁 A 36/36（evidence 182）。
   `/etc/profile.d/husk-gpu.sh` 仍然写入（evidence 167），`HUSK_GPU=llvmpipe` /
   `NR_GPU_DEFAULT_OFF=1` 一键退回软件；程序自己 export 的变量永远优先。
   假 DRM 节点的 `dup`/`dup2`/`dup3`/`fcntl(F_DUPFD)`/`fcntl64` 记账与 `close` 注销也已修
   （evidence 166：旧 loader 下 dup 出来是 memfd `rdev=0:0`，新 loader 是 `10:33`）。
   八维里第 1/5/6 维已有数字（evidence 168）：会话冷启动 8/8/9 s、设置首帧 4462 ms vs
   proot 5353 ms、xterm 110 vs 153 ms、回车到 guest 进程读到 41 ms；
   640×480 clear+swap 是 **zink 131 fps vs llvmpipe 300 fps**（这个负载 CPU 更快，照实记录，
   瓶颈是 SHM 拷贝不是 GPU），重负载对比未测。
   **30 分钟空载长稳已跑完**（evidence 169）：PSS 137.3 → 139.9 MB（前 60 秒启动分配，
   之后 29 分钟只涨 11 KB），线程 33 / fd 41 恒定、僵尸 0、同一 pid，结束后 X 根窗口仍
   100% 非黑。只覆盖空闲；负载型 soak **未测**。
   **关屏 150 秒再唤醒**也测了（evidence 173）：同一 pid、线程 33 不变、fd 41→40、
   内存 +4 kB，唤醒后 X 根窗口仍 100% 非黑（直方图逐桶几乎相同）。注意那不是深睡/Doze：
   全程 USB 供电 + adb 在线。
   **重绘也是实测的**（evidence 171）：同一会话四帧指纹，空载两帧逐字节相同，
   开/关 GNOME 概览各换一帧 —— 不是静态位图。
   **未验的一层（2026-11-25 用户实测发现）**：以上全是像素级检查，**没有**验会话形态——
   主题/样式是否加载、窗口能否拖动与对齐。主题文件在树里是齐的（`/usr/share/themes/Adwaita…`、
   `gnome-shell-theme.gresource` 710 151 B、`yaru-theme-gnome-shell` 已装），所以更像会话侧
   （session mode / dconf / gsd xsettings）没生效，根因 UNKNOWN；窗口拖动问题与输入默认
   trackpad 语义（`touchMode` 在 2.0 侧）相关，未测。
   未做：DRI3 零拷贝（`Could not get DRI3 device` 仍在）、
   2.0 侧 `GpuRuntimeEnvironment.gnomeSessionShell()` 仍硬钉 swrast。
   **0.0.4 补的数字**（evidence 174/176/178/179/180/181）：着色器密集负载在 2400×1080 上
   GPU **23.84 ms/帧（41.9 fps）对软件 266.73 ms/帧（3.7 fps），快 11.2×**；清屏+呈现是
   **19.9 vs 49.7 fps**，差在每帧把 GPU 表面回读交给 X server——2400×1080 上**回读 30.27 ms**，
   呈现按面积线性涨（zink 24.8 ms/兆像素、llvmpipe 11）。DRI3 通不了的原因在 **X server 侧**：
   应用内 bionic Xorg 21.1.99 是 `untrusted_app`：`/dev/dri` 下只有 `card0`
   （`system:system` `0660`）、**没有 `renderD128`**，`/dev/dma_heap` 不存在，GPU 实际走厂商节点
   `/dev/mali0`（`10,33`，`0666`）→ 只能停在 `Could not get DRI3 device`，
   **不是 path-shim 能补的**。
   **2026-11-26 第 5 轮定案（evidence 290）**：DRI3 结构性不可达，判定链已写死；
   2400×1080 逐段直测 = zink 回读 30.27 ms + X 协议拷贝 18.92 ms = 整窗 50.3 ms（19.9 fps），
   llvmpipe 回读 7.29 ms + 同一份拷贝 = 整窗 20.1 ms（49.7 fps），两条独立路径互相对上（差 3%）。
   零拷贝能拿回的全部价值就是那 30.27 ms。替代路线只有「自建 AHB present（客户端把 surface 做成
   AHB 经 SCM_RIGHTS 交给应用内 X server）」，验收标准已写进 evidence 290；MIT-SHM 改 memfd
   收益更小不先做，换合成器已否决。
   另记一条现场事实（FACT）：19:37/19:44 两轮核设备时，两棵树都**没有 Vulkan ICD** ——
   `rootfs` 是 0.0.36 的干净 base 安装（93 包，没有 mesa/GNOME）；
   `rootfs-broken-20261126` 有 GNOME + stock Ubuntu Mesa **25.2.8**，但 `usr/share/vulkan` 不存在。
   **同日 19:46 已补回**：把 `husk-panvk-builder` 容器里的 FristOneRR X11 构建装成
   `libvulkan_panfrost_frist370_execva.so` + `usr/share/vulkan/icd.d/husk_panvk_guest.json`，
   19:49 写 `etc/husk/gpu-env`。此后**不带任何 GPU 环境变量**、只经加载器启动的进程拿到
   `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，`NR_GPU_DEFAULT_OFF=1` 与 `HUSK_GPU=llvmpipe`
   都掉回 llvmpipe（门禁 `18-gpu-loader-defaults.sh` PASS=9 FAIL=0，evidence 291）。
   这棵桌面树同时缺 `etc/ld.so.husk-path`、`usr/local/share/linuxhub/`、`etc/profile.d` 那几份
   —— 它不是安装器产出的树，Husk 的安装层整个不在；之前那些 zink 记录（179/245/278）用的是
   另一棵带 ICD 的树。别再让它静默掉回软件：这条现在是门禁项。
   **同日 20:05（evidence 292）**：4 路并发 × 300 s 每路 **579 fps**、fd/RSS 不涨、GL 错误 0；
   固定管线即时模式 + 光照 320 fps；`PANVK_DXVK_COMPAT` 取 0 或 1 都通过（这个开关不是修复）。
   同时核清一条硬阻塞：**本应用没有 X server** —— APK 三个 `.so` 里没有，桌面树没有
   `usr/bin/Xorg`，2.0 包的 `:x11` 从本应用 uid 连是 `EPERM`。以前那些桌面 zink 记录用的都是
   2.0 的 X server。所以「桌面 + 多应用并发 + 长稳」这半条第 6 轮现在做不了，先要有 X server。
   真实客户端：`gnome-text-editor`（GTK 4.14 ngl）与 `gnome-control-center`（GTK 4 + Adwaita）
   完整出画，键盘 XTEST 敲进去标题与正文都更新（evidence 176/180）；**`nautilus` 46 内容区空白**
   ——能开窗、能枚举，但 `cairo`/`ngl`、`list`/`icon`、**proot 也同样空白**，根因 UNKNOWN
   （evidence 178）。整块 `glReadPixels` 目的行距**恒为 16 字节**、每行只有前 4 个像素落地；
   **逐行 `W×1` 与逐像素 `1×1` 读 100% 正确**，这是绕法（evidence 179）。
   `KBASE_IOCTL_MEM_EXEC_INIT` 每轮 EPERM：FACT + INFERENCE（只挡 CPU 可执行映射）+ UNKNOWN（内核为何拒）。
5. 交付形态：**已落地**（`tools/guest/install-husk.sh`，evidence 130）——本地 tarball 一条命令装完 +
   生成启动器，唯一支持的目标是**本应用的私有目录**（`run-as .../files/...`），端到端实测通过。
   **Termux 不是交付形态，也不承诺可用**：guest 树需要的二进制与库 Termux 的 `$PREFIX` 不提供，
   在那里跑会直接卡在缺依赖上（FACT，已实测失败），不是脚本能补的缺口。

## A 项：门禁 A 前的六条已知问题（2026-11-24 全部闭合）

| # | 问题 | 状态 | 实测 | 记录 |
|---|---|---|---|---|
| A1 | polkit 调用者可信层 | 闭合 | `pkcheck ... power-off --process <gnome-shell>` rc=0；`GetConnectionCredentials` 报 uid=1000 | evidence 151 |
| A2 | at-spi 无障碍总线 | 闭合 | 连上 at-spi 总线并 `ListNames`（同因：EXTERNAL 身份） | evidence 151 |
| A3 | 会话总线/终端 gvfs | 闭合 | `dbus-send --session ListNames` 正常；`gio mount -l` 不再报 proxy 错 | evidence 152 |
| A4 | xrandr scaling 告警 | 闭合 | `xrandr --version` 报 RandR 1.2；`grep -c "Impossible to set scaling"` = 0 | evidence 152 |
| A5 | 触控默认模式 | 闭合 | 默认模式 2 点左上角开出 GNOME 概览（点哪落哪）；模式 3 同 | evidence 152 |
| A6 | `nr-sigsys.log` 噪声 | 闭合 | 单次会话 246 行（此前单会话 4499 行） | evidence 152 |

门禁 A：`tools/device-tests/14-gate-a-acceptance.sh` **36 PASS / 0 FAIL**（桌面会话运行中）。
sudo/PAM 一条按"不需要"结案：`husk-su` 已覆盖提权路径，PAM 的 `ENOTSOCK`/audit netlink 不再出现在
任何验收判据里（evidence 139）。

## 构建与动态链接：argv 截断、绝对符号链接（2026-11-24，evidence 155）

| 项 | 结论 |
|---|---|
| `make -j4`（200 目标） | **可用**：`MAKE_RC=0`，3 509 ms。原来 shim 的 `execve` 用栈上 64 槽数组拼 argv，collect2 的第 64 个以后被静默丢掉，`ld` 报 `hidden symbol '__TMC_END__' isn't defined` —— 改成 `argv_len`/`argv_alloc` 堆分配（loader rev 23） |
| rootfs 绝对符号链接 | 552 个，宿主机加载器按 **host 根** 解析链接目标 → 逃出 guest。`husk-symlinks --root <rootfs> --apply` 改写 551 条为相对路径后，`numpy 1.26.4`、`libblas.so.3`、`liblapack.so.3` 按 soname 全部可用 |
| 注意 `adb shell` 的 guest stdout | 是 socket。`make` 往它写完，同一会话后续写入全部 `EACCES`；**proot 下完全一样**，不是 shim 行为。基准因此统一 `make -s -j4 >/dev/null 2>&1` |

## apt / libc：装包可用，但 libc6 升级会拆掉运行时（2026-11-24，evidence 154）

| 项 | 结论 |
|---|---|
| `apt install` | **可用**（815 → 882 包，27 秒）。两个前置修好了：`statvfs/statvfs64` 进 shim（`apt` 的剩余空间检查原来问的是 host 路径）、`/sbin/ldconfig.real` 换成 `husk-ldconfig`（静态 ldconfig 看不见 guest） |
| `dpkg --configure -a` | rc=0（`husk-ldconfig` 写 `/etc/ld.so.cache`，950 条 / 6 个目录） |
| 注意 `apt upgrade` 升 libc6 | **会覆盖运行时打过补丁的 libc**，之后 `getent shadow` 返回空、`sudo` 的用户查询失败、桌面起不来（`/bin/sh: 1: Cannot fork`）。修法：`make libc-patch` 重新打补丁（Rust 化，带断言；evidence 170） |
| 判据 | `python3 -c 'import os;print(os.statvfs("/").f_blocks)'` = 28 292 600（不是 194 478）；`getent shadow husk` 非空 |

**参考笔记不进仓库**：`$REPO-notes/prior-art/`（必读 `references.md`）。
**推 GitHub 走代理**：`HTTPS_PROXY=http://<host>:<port> git -c http.proxy=http://<host>:<port> push origin main`（地址不入库）

## 一句话

运行时核心（loader + path-shim + exec/PTY/信号/生命周期）、服务器表面（`/proc`、`/dev`、
`/dev/log`、装包）和 GPU、显示、视频、音频、相机、输入六类硬件都已在该机跑通。
按"对标容器/轻量虚拟机"的环境口径，A（账号）、B（挂载）、C（设备节点）、D（网络表面）
四段已收口，E（进程视图 + 服务监督器）已收口；**F 只剩 16K/64K 实机和 Mali 以外的 GPU 家族适配**。
桌面帧出屏（G）按目标延后，不作为环境完整性判据。传感器已接入中转 IIO 接口并通过 guest
读取/枚举验证；GPS、USB、蓝牙还需要最小 Java 回调垫片，NFC 与指纹不需要。2.0 已经**不含
proot**——APK 与仓库里都没有 proot，初始化与会话全部跑在 Husk 上（evidence 144）。

另外记一条会反复踩的事实（evidence 130）：2.0 assets 里的 `ubuntu-base-24.04.4-base-arm64.tar`
**是 gzip**（`1f 8b`，md5 `0be8b9e5…`），只是文件名没有 `.gz`。装之前先 `od -An -tx1 -N6`。

## 已完成（回归门禁，改动后必须重跑）

| 项 | 结论 | 证据 |
|---|---|---|
| loader + path-shim | 零 ptrace，路径一次改写完成；dlopen 进来的库也覆盖 | 27 / 100 |
| exec / PTY / 信号 / 生命周期 | 600 秒长稳、零僵尸、无重启风暴；旋转与 Surface 重建不重启会话 | 32 / 72 |
| 页大小 | rootfs 2987 个 ELF + loader + 我们全部产物在 4K/16K/64K 静态体检全绿（含 `p_align % 页` 规则）；**真 16K 内核（RPi5 kernel）与真 64K 内核（Ubuntu `linux-image-6.8.0-31-generic-64k`）上我们的 loader 端到端全绿**：`cat /etc/os-release`、`/usr/bin` 计数、嵌套 exec、`/tmp` 写读、`check-elf` 727 个 ELF 三档 congruent；修掉三个真 Bug（段重叠、mprotect 对齐、xattr/readlink 返回裸 -errno）；**16K/64K 真机仍 UNKNOWN**（本机只有 4K） | 30, 122, 124, 125, 126 |
| GPU | PanVK 列举 Mali-G77、compute、离屏 graphics、X11 窗口化 EGL 与桌面出帧；canonical guest ICD 和 loader 级 GPU 默认值已在真机验证，`env -i` 仍走 zink/PanVK；App 默认值生成已抽成 `gpu-env` crate，并补 Adreno 启动分支；0.0.19 默认关闭 DXVK 兼容 GS，并让默认值穿透 `setenv`/`execve`，fixed-function 600 帧与 ES3/Vulkan 回归通过；0.0.20 在真实 X11 窗口里 `glxgears` 连续 600 秒，119 个窗口平均 248.352 FPS，进程持有 `/dev/mali0`；0.0.34 当前 GNOME 进程已核对为 zink/PanVK，fd 表有两个 `/dev/mali0`；普通 Nautilus 进程也加载 PanVK 并持有同一设备 fd；`make gpu-audit` 通过 glxinfo 和嵌套 GTK 应用验证继承链。当前 GTK4 仍默认 `GSK_RENDERER=cairo`，因此不能把 GTK UI 绘制说成 GPU；**显式 `PANVK_DXVK_COMPAT=1` 仍崩溃，X11 窗口内容到 server 仍非零拷贝** | 46 / 47 / 165 / 232 / 241 / 242 / 243 / 244 / 245 / 278 |
| 设备族探测与降级 | `path-shim/src/plan.rs`：按节点定族（mali0 / kgsl-3d0 / renderD128 / 都没有）。Mali 使用 kbase DRM 语义层；Adreno/KGSL 已实现 render-node → 真实 `/dev/kgsl-3d0` fd、`DRM_IOCTL_VERSION=kgsl`、fd 继承和 stat/statx/uevent；`gpu-probe adreno` 可在真机复验。当前 Mali 门禁 A 38/38、Vulkan/GL 探针通过，App 入口重启的 GNOME 由当前 loader 映射 `/dev/mali0`；但 **Adreno 真机与通用 DRM 真节点仍 UNKNOWN（无设备）** | 127、233 |
| 外设族探测 | `gpu-probe periph`：传感器/GPS/USB/蓝牙/NFC/指纹/输入/i2c/spi 逐个节点实测，`EACCES`(13) 与 `ENOENT`(2) 分开记录；不造假节点、进程退出码 0。节点探测仍不能证明硬件型号，因此传感器改走 NDK 枚举；本机 5 类默认传感器全部 `READY`，guest IIO 读取与枚举通过 | 128 / 302 |
| 用户身份与 2.0 swap | `NR_FAKE_UID/GID` + `setuid/setresuid/setreuid/getresuid` 跟随 + `getgroups` 只报主组：`id` 报 `uid=1000(hu1)`，`su` 这一步可以删；`tools/guest/husk-session.sh` 是 `prootCommand()` 的替代契约（`env -i` 必须带 `NR_ROOTFS/NR_LD`）。**`su`/PAM 仍报 `PAM_SYSTEM_ERR`（UNKNOWN，已绕开）** | 129 |
| 显示 | AHB → dma-buf → SurfaceControl；3-buffer ring + 真实 SYNC_FD；10,000 帧 / 665 秒 | 53–59 / 73 |
| 视频 | H.264 1080p30 连续 600 秒，dropped=0；VA-API 解码出真实帧 | 74–80 / 85–87 |
| 音频输出 | PulseAudio → Rust relay → AAudio，610 秒连续会话 | 60 / 62 / 63 |
| 音频输入 | 声学回环 440 Hz 通过 | 88 |
| 相机 | guest `/dev/videoN` 是 V4L2 facade（memfd + Camera2 NDK 中转），不是真节点直连；前后摄自动启动，取流、拍照、录像 | 91 / 92 / 93 / 240 |
| 输入 | 全屏点击 5/5、双 X11 客户端各 5/5、各 15 个 XI 事件且无串扰；`adb input swipe` 路径仍是 UNKNOWN | 95 / 98 / 230 |
| 桌面 | 2.0 rootfs 起 GNOME Shell 46（Wayland 显示服务器），GTK4 客户端可连 | 101 |
| 装包 / dpkg | `apt-get install` / `--reinstall` / `-f install` 全通；dpkg 库零半配置包 | 106 / 109 |
| `scandir` / `dpkg-deb` | `scandir` interpose 补齐；guest `/tmp` 临时控制目录可见，`dpkg-deb -I` 与真实 `dpkg -i acl` 均返回 0 | 131 |
| ld.so.cache | 自研 Rust writer（1.1 格式），glibc 自己的静态阅读器接受 | 109 |
| 嵌套 exec 的库路径 | `NR_LIBRARY_PATH` 由 launcher 传给 shim，所有 nested exec 复用同一份完整 `--library-path`；只设 `NR_ROOTFS`/`NR_LD` 时走内置多目录 fallback。旧行为只重建 `<rootfs>/usr/lib/aarch64-linux-gnu`，`systemd` 私有目录在第二层进程消失（`libsystemd-shared-255.so` 找不到） | 133 |
| `posix_spawn` 的 NULL pid | glibc 允许 `pid == NULL`（`spawni.c` 只非空才回写），Ubuntu `localedef` 正是这样把 `.gz` charmap 交给 `gzip -d -c`；shim 原先把 NULL 当 EFAULT，导致所有压缩 charmap 报"not found" | 134 |
| 硬链接的 copy 模拟 | 本域 `link(2)` 被拒（EACCES），一直 copy 兜底；`close()` 已能经 `/proc/self/fd/<fd>` 重同步，0.0.21 起导出 `fclose` 后也会先 flush 再刷新 stdio 写入。仍然不是内核级硬链接，link count / 共享 inode / 跨进程即时共享没有恢复 | 134 / 246 |
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
| 挂载 | bind、tmpfs、只读 bind 的命名空间等价物（`mount`/`umount`/`umount2` + 路径层，`/proc/mounts` 一致）；只读对目录描述符同样生效，元数据写入（chmod/chown/utimensat/xattr）同样 `EROFS`；表有锁、存储目录不留残留。**注意**：这条在 0.0.36 之前是不成立的——只有 `umount2` 被 interpose，libmount 无 flag 时调的旧 `umount()` 是内联系统调用，回 EPERM，于是表只进不出（实测攒了 240 个 store 目录 / 26 条死项）。现在两个入口都覆盖，`05-mount-audit` 18/0、`16-umount-legacy` 12/0；跨会话的死项回收仍未做 | 112 / 113 / 117 / 287 |
| 运行时权限 | 首次安装（**不带 `adb install -g`**）首启弹系统权限框；拒绝后 toast 说明缺哪个权限、为什么；第二次拒绝自动打开本应用的权限页；在设置页改成允许后下次启动 `checkSelfPermission` 读回 granted、计数归零。`hasCode="false"` 未改，Activity 只从 `ANativeActivity.clazz` 取，无 hidden API。`USER_FIXED`（拒绝且不再询问）单独判定。放弃的项：`onRequestPermissionsResult` 在 NDK 回调表里不存在，信号只能靠 resume + `checkSelfPermission` 读回，所以分不清「拒绝」与「不再询问」——设置页引导因此按次数而不是按按钮猜 | 288 |
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
通知气泡），会话进程树当时使用已退役的旧应用包（准确包名见 evidence 147）→
`libld.so` → `gnome-shell` + DING / Nautilus / gvfs / tracker。整条链路四个独立故障与修法见 evidence 147；这里保留的是当年
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
2. **F：GPU 家族适配器**——Mali(kbase) 已在真机通过；Adreno(kgsl + Turnip) 的适配器代码
   已实现，但没有 Adreno 真机，打开、Turnip 枚举、ioctl/mmap 与桌面仍一律 UNKNOWN（evidence 233）。
   下一步是在 Adreno 设备运行 `gpu-probe adreno`；通用 DRM 真节点同样待验。
3. **2.0 集成：运行时替换已完成**（evidence 140–145；proot 及其补丁已全部删除，
   `/sdcard` 共享存储已补回并在 App 域实测——B 段的已知缺口只剩"`/storage` 只暴露自卷"）。
4. **G：首帧已出屏**（evidence 147）——GNOME 46 的完整桌面在 2.0 的树上跑起来了
   （顶栏 / 壁纸 / DING 图标 / dock / 通知，会话全程 `untrusted_app`）。卡了很久的四个
   独立故障都修在 shim 里：`RLIMIT_NOFILE` 天花板（`setrlimit` 钳位）、容器不能给自己人
   发信号（kill 家族透传）、GDBus 的 `SCM_CREDENTIALS`（`sendmsg` 改写 `ucred`）、
   系统总线 `EXTERNAL` 认证（改 `ANONYMOUS`）；外加 2.0 侧 X socket 路径错位。
   僵尸也一并修了（kernel 4.19 没有 `waitid(P_PIDFD)`，GLib 的 child watch 于是不回收——
   shim 里按 `/proc/self/fdinfo` 把 pidfd 翻成 pid，100 s 后 0 僵尸，见 147 §7.1）。
   `polkitd` 也通了（`initgroups` 仿真，现在常驻并持有 `org.freedesktop.PolicyKit1`）。
   剩下的都是"桌面能不能用"的问题，不是"能不能上屏"。

   **输入是通的**（evidence 148，纠正 147 §8 的错误结论）：注入入口一直在
   （`LorieView` → `EmbeddedInputController` → `TouchInputHandler` → JNI → lorie X server）。
   拖动移光标、悬停出 tooltip、点按开应用网格、点图标起终端、键盘出 `uname` 输出都实测通过。
   真正的坑是 `touchMode` 默认 `"1"`=trackpad（点按落在**光标**处，不落在手指处），
   而 `"2"` 模拟触摸在这台机子上**点击不落地**（FACT，未修），`"3"` 未测。

   身份链修了两层（evidence 150）：`SO_PEERCRED` / 收到的 `SCM_CREDENTIALS` /
   `/proc/<pid>/status` 现在按对端自己的 `NR_FAKE_UID/GID` 回答，转发来的凭据改成丢弃。
   还差最后一层——polkit 仍判调用者不可信（`Only trusted callers`，见 150 §4），
   `tools/guest-config/49-husk.rules` 已写好（root 与 uid 1000 = 管理员）。
   **这一层后来补上了**（evidence 151，见上面的 A1）：`pkcheck --process <gnome-shell>` rc=0，
   `GetConnectionCredentials` 报 uid=1000。这里保留的是当时的结论。

   SIGSYS 日志去重后，一次完整会话从 832 KB 降到 10 KB（320 行）。

   速度有了数（evidence 149）：同一个 App 沙箱、同一个 rootfs，只把被测量进程换成 proot——
   开设置 1.00 s vs 1.63 s（1.6×），`fork+exec` 1.73×、`find /usr` 1.75×、`tar` 2.06×、
   `dpkg -l` 1.11×、纯 CPU 1.20×：proot 的税在 syscall 密度上。

## 已知环境细节（踩过的坑，别再踩）

- loader 只接受**相对**程序路径（`usr/bin/bash` 可以，`/usr/bin/bash` 不行）。
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
