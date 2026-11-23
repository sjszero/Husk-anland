# 当前状态

> 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64），
> 运行域 `untrusted_app`，无 root、无 Shizuku。
> 只写**已上机验证**的结论；未验证写 UNKNOWN。每条的原始输出在 `docs/design/evidence/`。

## 一句话

运行时核心（loader + path-shim + exec/PTY/信号/生命周期）、服务器表面（`/proc`、`/dev`、
`/dev/log`、装包）和 GPU、显示、视频、音频、相机、输入六类硬件都已在该机跑通。
按"对标容器/轻量虚拟机"的环境口径，A（账号）、B（挂载）、C（设备节点）、D（网络表面）
四段已收口，E（进程视图 + 服务监督器）已收口；**F 只剩 16K/64K 实机和 Mali 以外的 GPU 家族适配**。
桌面帧出屏（G）按目标延后，不作为环境完整性判据。传感器、GPS、USB / 蓝牙 / NFC / 指纹
尚未开始；2.0 的 proot 运行时尚未替换。

## 已完成（回归门禁，改动后必须重跑）

| 项 | 结论 | 证据 |
|---|---|---|
| loader + path-shim | 零 ptrace，路径一次改写完成；dlopen 进来的库也覆盖 | 27 / 100 |
| exec / PTY / 信号 / 生命周期 | 600 秒长稳、零僵尸、无重启风暴；旋转与 Surface 重建不重启会话 | 32 / 72 |
| 页大小 | rootfs 2987 个 ELF + loader + 我们全部产物在 4K/16K/64K 静态体检全绿（含 `p_align % 页` 规则）；**真 16K 内核（QEMU + RPi5 16K kernel）上原厂 glibc 全通，我们的 loader 修掉两个真 Bug（段重叠、mprotect 对齐）后仍有一处 shim 窗口写保护未解**；16K/64K 真机仍 UNKNOWN | 30, 122, 124 |
| GPU | PanVK 列举 Mali-G77、compute、离屏 graphics、Zink surfaceless | 46 / 47 |
| 显示 | AHB → dma-buf → SurfaceControl；3-buffer ring + 真实 SYNC_FD；10,000 帧 / 665 秒 | 53–59 / 73 |
| 视频 | H.264 1080p30 连续 600 秒，dropped=0；VA-API 解码出真实帧 | 74–80 / 85–87 |
| 音频输出 | PulseAudio → Rust relay → AAudio，610 秒连续会话 | 60 / 62 / 63 |
| 音频输入 | 声学回环 440 Hz 通过 | 88 |
| 相机 | guest `/dev/videoN` V4L2，GStreamer 取流、拍照、录像 | 91 / 92 / 93 |
| 输入 | 触摸/键盘/鼠标经 mutter `RemoteDesktop` 注入 guest 合成器 | 95 / 98 |
| 桌面 | 2.0 rootfs 起 GNOME Shell 46（Wayland 显示服务器），GTK4 客户端可连 | 101 |
| 装包 / dpkg | `apt-get install` / `--reinstall` / `-f install` 全通；dpkg 库零半配置包 | 106 / 109 |
| ld.so.cache | 自研 Rust writer（1.1 格式），glibc 自己的静态阅读器接受 | 109 |
| 服务器表面 | `/proc` 合成、`/sys/class/net`、`/dev/log` 落地；79 PASS / 5 PARTIAL / 6 FAIL | 107 / 109 / 111 / 112 |
| 挂载 | bind、tmpfs、只读 bind 的命名空间等价物（`mount`/`umount2` + 路径层，`/proc/mounts` 一致）；只读对目录描述符同样生效，元数据写入（chmod/chown/utimensat/xattr）同样 `EROFS`，表有锁、存储目录不留残留 | 112 / 113 / 117 |
| 服务监督器 | `husk-init`（freestanding 静态，无 libc）按 `etc/husk/services.conf` 起服务、崩溃按 1→30s 退避重启、`once` 不重启、`on-failure` 只在失败时重启、stdout/stderr 落到 `var/log/husk/<name>.log`、`PR_SET_CHILD_SUBREAPER` 收孤儿、SIGTERM 0.5s 内停且零残留零僵尸 | 120 |
| 进程视图 | `ps -e`/`top`/`free`/`uptime` 只看到 guest 进程，且用真实程序名（`prctl(PR_SET_NAME)` 从 cmdline 取 basename）；`pgrep -x bash` 能命中；宿主 `/proc/<pid>` 读不到 | 119 |
| 设备节点 | `/dev/{null,zero,random,urandom,shm,pts/*,mali0,videoN}` 按名字打开全部正常，`mkfifo` 可用；三条平台墙已定性并写成断言：`/dev` 列不了目录（SELinux，不经 loader 也一样）、`/dev/full` 做不到 `ENOSPC`（要拦 `write(2)`，且 glibc stdio 走内部 `__write_nocancel`，判定为**不支持**）、`/dev/tty` 缺控制终端（`ENXIO`，属终端载体） | 118 |
| 网络表面 | `ip addr`/`ip route`/`ip link`/`ip -br`/`hostname -I`/`/proc/net/{dev,route}`/`if_nameindex`/`getent`/`getaddrinfo` 全通；**出口流量**（DNS/UDP/TCP/HTTP/HTTPS/`apt-get install` 真实下载）可用，改网络的动词回 `EPERM`（不假成功），`ip route get` 与 `ip neigh` 回 `EOPNOTSUPP`；MAC 全 0、无 IPv6、无默认网关、`ss` 空表；原始 ICMP 不可用 | 116 / 121 |
| 账号管理 | 自研 `husk-usermgr`（useradd/groupadd/usermod/userdel/groupdel/chpasswd/passwd/chfn/chsh）；`apt-get install tor` 的 postinst 建号成功 | 111 |
| POSIX 消息队列 | guest 内 `mq_open/mq_send/mq_receive/mq_getattr/mq_unlink` 全通（优先级顺序、`EAGAIN`、`curmsgs` 计数），12/12 PASS；根因是 arm64 glibc 的 `mq_*` 走未实现的 legacy syscall，必须函数级 interpose | 114 |
| SysV IPC | `shmget/shmat/shmdt/shmctl`、`semget/semop/semctl`、`msgget/msgsnd/msgrcv/msgctl` 全部在 shim 里实现（文件 + mmap + flock）：跨进程共享内存、P/V 阻塞唤醒、按类型取消息、`IPC_RMID` 宽限期；13 + 24 项全通过。`SEM_UNDO` = `EOPNOTSUPP`；`/proc/sysvipc/{shm,sem,msg}` 已合成，`ipcs` 出正确表头 | 115 / 116 |

## 正在进行：桌面帧出屏

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

1. **F：16K/64K 实机**——本机只有 4 KiB；`nr-tools check-elf` 已能静态体检。
   已用 QEMU + Raspberry Pi 5 的 16K 内核搭出真 16K 环境（evidence 124）：原厂 glibc 全通，
   我们自己的 loader 还有一处 shim 可写窗口的 `SEGV_ACCERR` 待定位；修完再谈 16K 真机。
2. **F：GPU 家族适配器**——本机只验了 Mali(kbase)。Adreno(kgsl + Turnip) 与通用 DRM
   按 `docs/design/device-adapters.md` 的表结构补适配器，没设备的一律 UNKNOWN。
3. **2.0 集成**：替换 proot 运行时，逐项删除 proot 专用补丁（用 artifacts/rollback 里的
   已知好包做对照）。
4. **G（延后）**：GNOME 帧出屏。前置未解：会话进程须 `NR_FAKE_ROOT=0`、GDBus 的
   `SCM_CREDENTIALS` 在 fake root 下 EPERM、gnome-shell 46 会 `free(): invalid pointer` 崩。

## 已知环境细节（踩过的坑，别再踩）

- loader 只接受**相对**程序路径（`usr/bin/bash` ✓，`/usr/bin/bash` ✗）。
- `run-as` 下不能写 `/tmp`，临时文件放 `$R/tmp/`。
- 不要用 `dbus-daemon --fork`：同一 address 会出现多个 daemon，之后所有客户端收不到回应。
  用 `--nofork` 自己后台化，`XDG_RUNTIME_DIR` 必须 `chmod 700`。
- `--library-path` 是替换不是追加，必须显式带 `$R/lib/aarch64-linux-gnu` 与 `.../gvfs`。
- rootfs 缺包会导致与运行时无关的怪故障，清单见 `docs/design/rootfs-packages.md`
  （gdk-pixbuf 的「所有图片格式都不认识」就是缺 `shared-mime-info`）。
