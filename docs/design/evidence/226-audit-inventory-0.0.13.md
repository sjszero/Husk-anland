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
| H-19 | 音频中转 | 音频桥在 accept 循环里同步跑长连接会话：一个 idle 客户端堵死整条监听 | 225（**代码已改，运行未验证**） |
| H-20 | 挂载 | `mnt_read_file()` 的读缓冲是 `static mut`：两个线程往同一块内存里读，表文件在读的间隙被改小就解析出**旧的**挂载项 | 227 §2.1 |
| H-21 | 挂载 | `mnt_save()` 用 `O_TRUNC` 直写，读的一方每次路径解析都读它：能读到 0 字节或**半张表**（刚挂上的东西这次解析里不存在） | 227 §2.2 |
| H-22 | 挂载 | `mount -o remount,ro <tgt>`（util-linux）一直 EINVAL：它把 bind 的**源**当 target 交过来；尾部斜杠也匹配不上 | 227 §3 |

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

## 4 · 还没审到的面（0.0.13 未闭合的部分）

| 面 | 卡在哪 | 需要什么 |
|---|---|---|
| X11 呈现与零拷贝 | 呈现那一次回读（2400×1080 上 30.27 ms/帧）已有结论，但「除了 DRI3 还有没有更便宜的交付路径」没审 | 能用的桌面（设备解锁、应用在前台） |
| GPU 直通（剩余） | 懒初始化这一批已修；`DrmCtx` 内部字段布局、Mesa/ICD 在并发下的行为没审 | 能用的桌面 + `gl-probe` |
| 输入栈 | evidence 95/148/180 已有行为记录，但并发（多客户端 / 触摸与指针混用）没审 | 能用的桌面 |
| 音频 / VA-API 中转的**运行**验证 | 应用侧已改（H-19），但设备上跑的是 2.0 的包，这条桥没被拉起 | `make apk` 装一次，再跑 `tools/probe/audio-bridge-concurrency.py` |
| 跨设备 | 16 KiB / Adreno / 其它 Android 版本只有 QEMU 或未验证 | 真机 |

## 5 · 结论怎么用

- H-xx 全部有复现与修后对照，能进 0.0.13 的 CHANGELOG。
- L-xx 不修，但必须写在 LIMITATIONS 里，别让下一个人以为是漏掉的 bug。
- §3 是「别再查一遍」的清单；§4 是这一版不打算宣称完成的部分。
- H-19 是唯一一条「改了但没跑过」的，CHANGELOG 里写明了，不算已验收。
