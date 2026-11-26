# 226 · 0.0.13 审计清单：已确认的缺陷、核过不是缺陷的、还没审到的

日期：2026-11-26 00:05（CST）。这一份是**索引**，每条结论都指向带原始输出的那条 evidence。

## 1 · 已确认并修掉的（H）

| 编号 | 面 | 症状（用户能看到的） | 证据 |
|---|---|---|---|
| H-01 | path-shim 覆盖 | `/proc/self/exe` 报的是宿主 loader 路径，`sys.executable`、`/proc/self/exe` 自省全错 | 217 |
| H-02 | path-shim 覆盖 | 老二进制（glibc < 2.33 链接）走 `__xstat` 家族，路径不翻译，全 ENOENT | 218 |
| H-03 | path-shim 覆盖 | `dlmopen` 打不开 guest 里的 `.so` | 219 |
| H-04 | path-shim 覆盖 | `execveat` 绕过 shim | 219 |
| H-05 | path-shim 并发 | `PEER_CACHE` 三段 store：stat 可能把 A 进程的 uid/gid 安到 B 头上 | 220 |
| H-06 | path-shim 并发 | `FAKE_ID_READY` 先置位后写值：身份可能报成 0 | 220 |
| H-07 | path-shim 并发 | `RAND_COUNTER` 非原子：两个线程拿到同一个临时文件名 | 220 |
| H-08 | 挂载 | 挂载表每进程只读一次：同一路径在两个进程里指向两个目录，写进去的字节「消失」 | 221 |
| H-09 | 挂载 | 死挂载项跨会话累积，攒满 8 条后 `mount` 直接 ENOSPC | 221 |
| H-10 | 会话形态 | umask 0000 → 新目录 0777、文件 0666；DING 拒绝从世界可写的 `~/Desktop` 启动 | 222 |
| H-11 | 会话形态 | `readlink("/proc/self/cwd")`、`("/proc/self/root")` 返回 ENOENT；`/proc/self/fd/N` 直接暴露宿主路径 | 222 |
| H-12 | 会话形态 | `inotify_add_watch`/`statfs`/`getxattr` 一族把「不存在」报成 EPERM（dbus 起总线时那句 Cannot setup inotify） | 222 |
| H-13 | loader | `REAL_DLMOPEN` 裸静态懒解析：并发 dlopen 时返回 NULL 且 `dlerror()` 为空 | 223 |
| H-14 | loader | `REPATCH_LOCK` check-then-set：两个线程同时进 `nr_repatch` | 223 |
| H-15 | loader | `nr_repatch` 不持锁遍历 link_map，撞上别人 `dlclose` 的 unmap → **SIGSEGV**（10 次里 2 次） | 223 |
| H-16 | GPU/相机 | `DRM_CTX`/`V4L2_CTX`/`/dev/dri` 假目录/`mali0` 元数据全是无保护的懒初始化；并发下 `DRM_IOCTL_VERSION` 偶发 EACCES | 224 |
| H-17 | GPU | `FAKE_FDS` 的「找空槽→写入」非原子：被覆盖的 fd 的 ioctl 落到 memfd，客户端判定「不是 GPU」退回软件 | 224 |
| H-18 | sigsys | `SEEN_NR`/`SEEN_COUNT` 在信号处理器里 TOCTOU，计数可越过数组长度 | 224 |
| H-19 | 音频中转 | 音频桥在 accept 循环里同步跑长连接会话：一个 idle 客户端堵死整条监听 | 225；231（并发 2/2、播放与采集协议真机 PASS） |
| H-20 | 挂载 | `mnt_read_file()` 的读缓冲是 `static mut`：两个线程往同一块内存里读，表文件在读的间隙被改小就解析出**旧的**挂载项 | 227 §2.1 |
| H-21 | 挂载 | `mnt_save()` 用 `O_TRUNC` 直写，读的一方每次路径解析都读它：能读到 0 字节或**半张表**（刚挂上的东西这次解析里不存在） | 227 §2.2 |
| H-22 | 挂载 | `mount -o remount,ro <tgt>`（util-linux）一直 EINVAL：它把 bind 的**源**当 target 交过来；尾部斜杠也匹配不上 | 227 §3 |
| H-23 | GPU/DRM | BO 只建不释放：`GEM_CLOSE` 没实现、`KBASE_MEM_FREE` 从没被调用，第 64 个 `CREATE_BO` 起一律 ENOMEM | 229 §2 |
| H-24 | GPU/DRM | `DRM_CTX.dispatch()` 无锁：并发 `CREATE_BO` 拿到重复 handle、覆盖 BO 表 | 229 §3 |
| H-25 | 合成 VFS | `/proc/mounts`、`/proc/self/status`、`/proc/sysvipc/shm`、`/proc/uptime` 共用一个无锁 `VTEXT`；16 线程 × 4000 轮中 35,348/64,000 次读到另一个文件的内容 | 235 |
| H-26 | 身份状态 | `FAKE_UID/GID`、`IDENT_PUB_UID/GID`、`IDENT_NOTES` 用无保护 `static mut`；并发读写构成数据竞争，身份对可能半新半旧 | 236 |
| H-27 | 发布工程 | APK `versionName=0.1`、`versionCode=1` 与仓库 `0.0.13` 脱节；改为直接读取 `VERSION` | 237 |
| H-28 | 构建入口 | `make apk` 要求 NDK 指向 `<prebuilt>`，loader 却要求 `<prebuilt>/bin`；并发探针脚本也会拼出 `bin/bin/clang`。同一条 NDK 设置无法跨入口使用 | 238 |
| H-29 | V4L2 | 假相机 fd 的引用计数只扫描 `0..1023`；`dup()` 到高 fd 后关闭原 fd，会提前拆掉仍被引用的 camera socket | 250 |
| H-30 | V4L2 | `V4L2_CTX` 在 lazy init 之后无锁访问；path-shim 与 preload 两层都可能并发改写 socket、buffer 表和设备状态 | 250 |
| H-31 | GPU/DRM | `dup2`/`dup3` 用普通文件覆盖假 DRM/KGSL fd 时，旧目标表项未清理；后续 ioctl 会被错误路由进假设备并可能伪成功 | 251 |
| H-32 | 发布构建 | `libc-nop.so.6` 是被忽略的生成物，但 `make dist` 没生成也没前置拒绝；clean tree 在复制阶段失败 | 252 |
| H-33 | V4L2 | `dev_of_fd()` 只按 fd 号码认相机；`dup2`/`dup3` 用普通文件覆盖假 fd 后，`VIDIOC_QUERYCAP` 仍伪成功 | 253 |
| H-34 | V4L2 | 假 fd 被覆盖后 live memfd 引用可为 0，但 `close()` 只把 1 当最后引用，camera.sock 泄漏不关闭 | 253 |
| H-35 | GPU/DRM | `libdrm_preload.so` 把 fd 号码记在私有表里；`dup2` 用普通文件覆盖假 DRM fd 后仍认成 DRM，并把普通文件 ioctl 错误路由进 GPU 模拟层 | 254 |
| H-36 | GPU/DRM | `libdrm_preload.so` 的 `openat` 把 glibc 已返回的 `-1` 当成原始 `-errno`，`ENOENT` 被改写成 `EPERM` | 255 |
| H-37 | GPU/DRM | `libdrm_preload.so` 的 `open` / `open64` / `openat` / `openat64` 丢弃创建 mode，`O_CREAT` 文件创建成 000（再套 umask） | 255 |
| H-38 | GPU/DRM | `libdrm_preload.so` 的 `mmap` 失败返回 `NULL` 而不是 `MAP_FAILED`，并把 `EBADF` 改写成 `EPERM`；调用方可能把失败当成功地址 | 256 |
| H-39 | GPU/DRM | `libdrm_preload.so` 的普通 fd `ioctl` 透传把 `ENOTTY` 改写成 `EPERM`；同时 DRM 回调收到的是 glibc `-1` 而不是原始 `-errno` | 256 |
| H-40 | GPU/DRM | path-shim 与 preload 打开 DRM 假节点时忽略 `O_CLOEXEC`；带该标志打开的 fd 仍是 `FD_CLOEXEC=0`，会泄漏进 `exec` 子进程 | 257 |
| H-41 | GPU/DRM | path-shim 忽略 kbase `DrmCtx::init` 失败并仍标记 ready；preload 初始化失败既不关闭 mali fd，也不把负 errno 返回给调用方 | 257 |
| H-42 | V4L2 | path-shim 打开假 `/dev/video0` 时丢弃 `O_CLOEXEC` 与 `O_NONBLOCK`；相机 fd 可泄漏进 `exec` 子进程，非阻塞采集语义失效 | 258 |
| H-43 | 文件创建 | `O_CREAT` 被无条件改成 `mode \| 0600`，所有程序创建的文件都可能读回错误权限；dpkg 的写放宽还会把 inode 永久留在 `0600` | 259 |
| H-44 | 用户身份 | `fchown(fd)` 只返回成功但不记录虚构 uid/gid，随后 `fstat(fd)` 与跨进程路径 `stat` 仍报真实 app 所有者 | 259 |
| H-45 | 用户身份 | `access/faccessat` 按真实 Android uid 判断；假 root 读不了 `0000` 文件，非 root 被当成所有者，且不检查父目录 X 权限 | 259 |
| H-46 | 用户身份 | `getgroups` 永远只报主组；`setgroups`/`initgroups` 只返回成功但不保存，`/etc/group` 里的附加组和组权限文件失真 | 260 |
| H-47 | 会话形态 | `husk-session.sh` 在 `env -i` 边界硬编码 `LANG=C.UTF-8`，带中文 locale 的 rootfs 仍以英文环境启动 | 261 |
| H-48 | 会话形态 | 只显式设置 `LANG=en_US.UTF-8` 时仍强制注入 `LANGUAGE=zh_CN:zh`；gettext 的语言优先级会让英文覆盖失效 | 261 |
| H-49 | 会话形态 | `husk-desktop.sh` 与 `/etc/profile.d/husk-locale.sh` 重复了同一个 `LANGUAGE` 覆盖错误，修了 session 仍会在登录 shell/桌面侧复现 | 261 |

## 2 · 记录在案、不修或不完全修的（L）

| 编号 | 内容 | 为什么这样定 | 位置 |
|---|---|---|---|
| L-01 | 硬链接模拟的刷新只对程序自己 `close(2)` 生效 | glibc 内部 `fclose → __close_nocancel` 是直接调用，只能靠 L2 那层 | LIMITATIONS §13 / 221 §3 |
| L-02 | 挂载表默认每次重读，代价 +0.5% | 不重读就是静默写错位置，宁可慢 0.5% | LIMITATIONS §13 / 221 §1 |
| L-03 | `/proc/self/{cwd,root}` 可以读链接，不能拿来做路径前缀 | 放行等于给一条跳出 rootfs 的路 | LIMITATIONS §14 / 222 §2 |
| L-04 | umask 由会话入口定成 022，不跟 guest `/etc/login.defs` 走 | 要在 shim 里重算每个创建点的 mode，不划算 | LIMITATIONS §14 / 222 §1 |
| L-05 | 信号处置继承自 Android 父进程（一位 RT 信号被忽略） | loader 既不添也不清，与 `adb shell` 逐位相同 | LIMITATIONS §14 / 222 §5 |
| L-06 | `nr_set_main_map` 是没人调用的导出 | 删它会动 nr-tools 的符号表，本轮不动 | 223 §7 |
| L-07 | DRI3/dma-buf 直传在这台机上不可用（没有 render 节点） | 不是权限问题，是设备没有那个节点；呈现那一次回读还在 | LIMITATIONS §7 |
| L-08 | `glxgears` 在 zink/PanVK 的 `emit_cf_list.isra.0+0x120` 稳定 SIGSEGV | pre-229 loader 同样复现；同一轮 gl-probe/ES3/Vulkan 通过；具体 NIR 破坏 UNKNOWN，不能宣称 GPU 长跑通过 | 232 |

## 3 · 核过、不是缺陷的（避免下次重新怀疑）

| 项 | 判据 | 证据 |
|---|---|---|
| `MNT[]` 容量检查缺失 | 已有检查 | 220 §末 |
| 「`mnt_read_file()` 清零窗口被别的线程看到」 | 9 处访问点全部在同一持锁区间（逐个列出）；探针 200 000 次 ×46 000 次 mount/umount 零异常 | 227 §1 |
| `elf_interp` 的 `try_into().unwrap()` | 定长区间，不可能 panic | 220 §末 |
| `SHM_ATT` / `NL_SLOTS` / `MNT` / `ROOT_*` / `OWN_*` / `DRM_*` / `V4L2_*` 的表结构 | 逐个查过写点，见各条 | 220 §末 |
| `REPATCH_LOCK` 是无界自旋 | **不是**自旋：它是「忙就跳过」，改 CAS 之前的问题是并发进入，不是死锁 | 223 §3 |
| `IDENT_PUB_UID/GID` 两字段分开写 | 撕裂只会导致「多发布一次」，不影响正确性 | 224 §5（同批审计） |
| boot 侧十几个 `static mut` | 写点全在 `nr_boot`（`_dl_init` 钩子，单线程） | 223 §7 |
| `ctypes` 拿到的是原始符号，绕过 GOT | 设计边界（raw 调用不翻译），不是 bug | 222 §6 |
| raw `syscall(SYS_*)` 绕过 shim | 同上，SECURITY.md 已写明 | LIMITATIONS §9 / 214 |
| netlink `dup2`/`dup3` 普通 socket 覆盖假 fd | 按 socket inode 查槽位；真机三方向 PASS | 253 |
| 最终 symlink 并发替换逃到宿主 | 4 worker 212,050 次 `chmod/utimensat/chown/open/lstat`，同时切换最终链接 12,860 次；宿主哨兵内容、mode、mtime 不变 | 262 |

## 4 · 还没审到的面（0.0.13 未闭合的部分）

| 面 | 卡在哪 | 需要什么 |
|---|---|---|
| GPU 直通（剩余） | 懒初始化、BO 生命周期、`dispatch` 并发已审并修（229）；常规 GPU 探针通过，但 `glxgears` 在 PanVK/Bifrost 编译器稳定崩溃（L-08） | 需要 Mesa 侧定位 NIR 破坏；不能以单个回退参数冒充修复 |
| 跨设备 | 16 KiB / Adreno / 其它 Android 版本只有 QEMU 或未验证 | 真机 |

## 5 · 已审完的面

| 面 | 结论 | 证据 |
|---|---|---|
| X11 呈现与零拷贝 | 除 DRI3 外**当前布局里没有更便宜的路径**：MIT-SHM 因 server 是 bionic + 内核无 SysV IPC 而 `BadAccess`；客户端只能 `XPutImage`（2400×1080 上 18.9 ms/帧）；应用侧 5–9 FPS 且 `0/0 present copies offloaded to GPU`。见 evidence 228 的三条路径与代价排序 | 228 |
| X11 输入 | 全屏 5/5 坐标命中；双客户端各 5/5、各 15 个 XI 事件，无串扰。`adb input swipe` 25 秒 0 事件，注入路径仍是 UNKNOWN，不冒充支持 | 230 |
| 音频中转运行 | idle 客户端不再堵死两个监听口（2/2 PASS）；播放 `48000/48000` 帧写入，1 次 xrun、平均 121.667 ms；采集 49152 帧 / 196608 字节全部收回 | 231 |

## 6 · 结论怎么用

- H-xx 全部有复现与修后对照，能进 0.0.13 的 CHANGELOG。
- L-xx 不修，但必须写在 LIMITATIONS 里，别让下一个人以为是漏掉的 bug。
- §3 是「别再查一遍」的清单；§4 是这一版不打算宣称完成的部分。
- H-19 已在 evidence 231 做真机运行验证；但“用户实际听到”和“麦克风声学质量”仍是 UNKNOWN。
- L-08 已写入 0.0.13 的公开限制；GPU 长跑未通过，不能把 `gl-probe` 通过写成“所有 GPU 程序稳定”。
