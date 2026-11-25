# 基准

一台设备、一份 rootfs、三个运行器，只换运行器。下面每个数字都是同一台手机上 5 轮的
中位数。

## 测了什么

| 项 | 值 |
|---|---|
| 设备（参考） | 一台 arm64 手机：Android 13（SDK 33）、kernel 4.19.191、4 KiB 页；型号与 SoC 见 `docs/STATUS.md` |
| 沙箱 | `untrusted_app`，无 root、无 Shizuku、未解 Bootloader、无虚拟化 |
| 用户态（参考发行版） | Ubuntu 24.04，glibc 2.39，882 个 dpkg 包，一个 rootfs 目录 |
| Husk | loader `rev 23`，sha256 `d5dd7e63…`，path-shim 内嵌，零 ptrace |
| proot | LinuxHub 2.0 原样发货的 `proot` 二进制与参数（`-0 -r --link2symlink -b /dev -b /proc -b /sys`） |
| proot-distro | 同一个二进制，换成 proot-distro 的调用方式 |
| 时间 | 2026-11-24 16:40–17:10（CST） |
| 样本 | 每项每运行器 5 轮（`apt-get update` 冷启动 2 轮，因为它要删索引） |
| 电量 | 全程 100%，电池温度 27.7–28.5 °C |
| 供电 | USB 供电（adb 本身就要求 USB），因此全程不在电池上 |

两个决定这张表算不算数的细节：

**三个运行器读的是同一个目录，不是三份拷贝。** rootfs 是应用私有目录下的一棵树，
Husk、proot、proot-distro 都把根指到它。没有可比的校验和，因为没有复制——两次取样之间
加进去的文件，三个运行器同时看得见。

**运行器是轮转的，不是跑完一个再跑下一个。** 分块跑一轮要 ~25 分钟，而这台手机会降频：
同一份 Husk 在 16:32 比 16:05 慢 1.5–1.8×（`stat`、`find`、`tar`、`make`）。所以 5 轮
在一轮内轮转三个运行器（`tools/bench/run-interleaved.sh`），块间冷却 60 秒。热漂移因此
均摊到三个运行器，而不是记在最后跑的那个头上。

复现：

```
tools/bench/install.sh                       # 推设备侧那一半
tools/bench/run-interleaved.sh 5             # 5 轮、3 运行器、轮转
tools/bench/compare.py docs/design/evidence/bench-*-il.log
```

原始日志：`docs/design/evidence/bench-{husk,proot,proot-distro}-20261124-1640-il.log`。

## 总表

| 维度 / 项 | Husk | proot | proot-distro | Husk 相对 proot |
|---|---:|---:|---:|---:|
| **1 · 启动** | | | | |
| 冷启动到 `/bin/true`（含 adb） | 155 ms | 151 ms | 162 ms | 1.0× |
| `apt-get update` 冷（删索引） | 7,337 ms | 7,901 ms | 7,906 ms | 1.1× |
| `apt-get update` 热 | 7,347 ms | 7,874 ms | 7,941 ms | 1.1× |
| 桌面会话冷启动到可用 | 8 / 8 / 9 s | 不可比 | 不可比 | — |
| 桌面首帧（X 根窗口非黑像素） | 100.0%（2400×1080） | 不可比 | 不可比 | — |
| **2 · 进程与 syscall 密度** | | | | |
| fork+exec × 100 | 363 ms | 550 ms | 564 ms | 1.5× |
| `/bin/true` × 50 | 207 ms | 309 ms | 315 ms | 1.5× |
| shell 循环 1000 次 | 16 ms | 30 ms | 28 ms | 1.9× |
| `getpid` × 20 000 | 62 ms | 104 ms | 99 ms | 1.7× |
| `stat` × 20 000 | 650 ms | 3,734 ms | 3,796 ms | 5.7× |
| `open`+`close` × 20 000 | 1,252 ms | 8,481 ms | 8,697 ms | 6.8× |
| `make -j4`（200 目标链接） | 5,194 ms | 24,182 ms | 24,864 ms | 4.7× |
| **3 · 文件 I/O** | | | | |
| `find /usr` | 1,484 ms | 5,464 ms | 5,623 ms | 3.7× |
| 源码树 tar 解包 | 533 ms | 2,051 ms | 2,124 ms | 3.8× |
| `grep -r` 头文件树 | 214 ms | 713 ms | 885 ms | 3.3× |
| 顺序写 256 MB | 1,294 ms | 1,321 ms | 1,295 ms | 1.0× |
| 顺序读 256 MB（页缓存） | 133 ms | 159 ms | 156 ms | 1.2× |
| 1 万小文件建删 | 239 ms | 687 ms | 666 ms | 2.9× |
| 本地 `git clone` + `status` | 889 ms | 3,357 ms | 3,380 ms | 3.8× |
| **4 · 包管理与开发工作流** | | | | |
| `dpkg -l` | 45 ms | 89 ms | 93 ms | 2.0× |
| `apt-get -s install` 解析 | 89 ms | 258 ms | 249 ms | 2.9× |
| `gcc` 编译 hello | 165 ms | 358 ms | 385 ms | 2.2× |
| `python3` import json | 88 ms | 169 ms | 156 ms | 1.9× |
| `python3` import numpy | 410 ms | 726 ms | 722 ms | 1.8× |
| **5 · 桌面与图形** | | | | |
| 打开「设置」到首个大窗口 | 4,462 ms | 5,353 ms | 未测 | 1.2× |
| 打开终端（xterm）到窗口 | 110 ms | 153 ms | 未测 | 1.4× |
| 帧率 640×480 clear+swap（GPU） | 131 fps | — | — | — |
| 帧率 640×480 clear+swap（CPU/llvmpipe） | 300 fps | — | — | — |
| 文件管理器（Nautilus）首帧 | 未测 | 未测 | 未测 | — |
| **6 · 交互延迟** | | | | |
| 回车 → guest 进程读到 | 41 ms（扣工具开销约 22 ms） | 未测 | 未测 | — |
| 按键 → 屏幕像素 | 未测 | 未测 | 未测 | — |
| **8 · 兼容性** | | | | |
| `bash` | 可用 | 可用 | 可用 | — |
| `gcc` | 可用 | 可用 | 可用 | — |
| `git` | 可用 | 可用 | 可用 | — |
| `make` | 可用 | 可用 | 可用 | — |
| `nano` | 可用 | 可用 | 可用 | — |
| `node` | 可用 | 可用 | 可用 | — |
| `python3` | 可用 | 可用 | 可用 | — |
| `vim` | 可用 | 可用 | 可用 | — |

`Husk 相对 proot` 是 proot ÷ Husk：5.7× 表示 Husk 快 5.7 倍。

同一个工程在另一次单独运行里测到 3 509 ms（`docs/design/evidence/155-*`）。两次差 1.5×，
差异来自那次没做 60 秒冷却、也没有与另外两个运行器轮转；**表里的数字只在本表条件下可比**，
不要把不同批次的数拿来互相换算。

第 5、6 维现在有数字了（evidence 168），但要看清它们**能**说明什么：

- **能有数字，是因为运行器外面有了一个 X server**：Husk 这边的 X server 是 App 进程里的
  Lorie（`:0`），proot 侧没有对应的桌面，所以 proot 列里凡是"不可比/未测"的格子，
  是真的没有对象，不是没去测。
- **帧率那一行对 Husk 不利，照实记录**：640×480 只有 `glClear` + swap，瓶颈是呈现路径上的
  SHM 拷贝（DRI3 未通），zink 还多一层 Vulkan 翻译，所以 CPU（llvmpipe）反而更快。
  这一行说明的是呈现链路的天花板，**不能**拿来说"GPU 没有用"；重负载（着色器/纹理密集）
  的 GPU vs CPU 对比**未测**。
- "设置"的 proot 中位数只来自 2 个有效样本：另 2 次 90 秒超时（单实例应用在 D-Bus 上被
  激活而不是新开进程，残留实例就测不出首帧），超时的不计入中位数但记在 evidence 168 里。
- Nautilus 拒绝以 root 运行，本会话的 guest 身份是 root，所以那一格是**未测**，不是慢。

第 7 维（空载 30 分钟的 RSS / 线程 / fd 增长与僵尸数）单独跑 soak，不放进计时的轮次里：

| 运行器 | 30 分钟空载 | 结论 |
|---|---|---|
| Husk | PSS 232 661 → 232 138 KiB，线程 47，fd 118，guest 僵尸 0（evidence 153） | 通过 |
| proot | RSS 恒定 3 480 KiB，线程 1，fd 3，僵尸 0（`soak-proot-20261124.log`） | 通过 |
| proot-distro | 采样到第 420 秒后 RSS / 线程 / fd 全部是 `NA`，会话已经不在了（`soak-proot-distro-20261124.log`） | **没跑完**：它的 `--kill-on-exit` 与持久的 `sleep` 冲突，会把会话拆掉 |
| Husk（GPU 桌面会话，另一个对象：`gnome-shell` on zink/PanVK） | PSS 137 349 → 139 930 KiB（前 60 秒 +2.5 MB 启动分配，之后 29 分钟 **+11 KB**），线程 33 恒定，fd 41 恒定，僵尸 0，全程同一 pid；结束后 X 根窗口仍 100% 非黑（evidence 169） | 通过 |

proot-distro 这一格我们**没有数据**，不拿别的数字顶替。电池放电曲线也没测：adb 需要 USB，
全程在充电。两行 Husk 是**不同对象**：一行是命令行树里的被测进程（evidence 153），
一行是 GPU 桌面会话（evidence 169），不要把两行的 PSS 相减。桌面这一行只覆盖空闲，
睡眠/唤醒与负载型 soak **未测**。

## 为什么

差距不是一个常数，它跟着 syscall 与路径密度走——这正是两种设计的区别所在。

- **Husk 不拦截 syscall。** 路径虚拟化在装载时做一次（改写每个对象的 GOT），之后程序
  自己的 `svc` 直接进 Android 内核。所以纯 syscall 循环几乎不额外付钱：`getpid` × 20 000
  62 ms 对 104 ms，`fork+exec` × 100 363 ms 对 550 ms——1.5–1.7×，这是进程本身的开销，
  不是翻译层的开销。
- **proot 用 ptrace 拦截每条 syscall。** 每个 `stat`、`open`、`close`、`readlink` 都要停住
  被跟踪进程、解码、路径翻译、再放行。所以路径密集的那些行跑得最多：`stat` × 20 000 5.7×、
  `open`+`close` × 20 000 6.8×、`find /usr` 3.7×、`grep -r` 3.3×、`tar` 解包 3.8×。
- **大块 I/O 打平，表里就这么写。** 256 MB 顺序写 1 294 ms 对 1 321 ms（1.0×），命中页缓存
  的读 133 ms 对 159 ms。数据成块进内核时，运行器没有可翻译的东西。`apt-get update`
  （7 347 ms 对 7 874 ms，1.1×）同理，它被镜像与解包主导。
- **构建与解释器在中间**，因为它们是进程、路径、CPU 的混合：`make -j4` 4.7×、`gcc` 2.2×、
  `import numpy` 1.8×、`git clone` 3.8×。

## 代价

- **成熟度。** proot 是十年积累下来的通用工具，各种发行版的坑都处理过。Husk 只跑它验证过
  的树：arm64、glibc 2.39、Ubuntu 24.04。loader 的补丁是版本断言的，换 glibc 会直接失败，
  而不是碰巧能用。
- **发行版覆盖。** 一个发行版、一个 ABI、一个设备族（arm64 Android 4 KiB 页已验证；
  16/64 KiB 只在 QEMU 上）。
- **rootfs 准备。** stock 的树要三步才能跑：libc 打补丁、绝对符号链接改相对、
  `husk-ldconfig` 替换静态 `ldconfig`（`docs/RUNNING.md`）。proot 一步都不要。
- **设备访问。** 两个运行器都没有传感器、GPS、USB、蓝牙、NFC、指纹。
- **图形。** 桌面能出帧、默认走 GPU（第 5 维）。呈现仍要回读 + 拷贝（DRI3 未通），所以
  清屏 + 窗口呈现这一格反而输给 CPU；但**着色器密集的负载 GPU 快一个数量级**。四类负载分开
  写，见下节。proot 侧没有可比对象，那几格是「不可比」，不是「更快」。

## GPU 原生加速（2026-11-25，2400×1080 = 本机 X 屏幕）

同一份 `gl-probe` 二进制，只换 `GALLIUM_DRIVER`。`pbuf` 是离屏（无呈现），`xput` 是纯 X 传输，
`fill` 是每像素 48 次 `sin()` × 2 遍的着色器负载，每帧 `glFinish`。

| 负载 | GPU（zink→PanVK→Mali-G77） | 软件（llvmpipe） | 结论 |
|---|---:|---:|---|
| `fill` 着色器密集，2400×1080 | 23.84 ms/帧（41.9 fps） | 266.73 ms/帧（3.7 fps） | **GPU 快 11.2×** |
| `fill` 着色器密集，1280×720 | 14.90 ms/帧（67.1 fps） | 101.92 ms/帧（9.8 fps） | **GPU 快 6.8×** |
| `pbuf` 纯清屏，2400×1080 | 2.25 ms/帧 | 2.01 ms/帧 | 打平（清屏对 GPU 最不利） |
| `window` 清屏 + 呈现，2400×1080 | 50.18 ms/帧（19.9 fps） | 20.12 ms/帧（49.7 fps） | 软件快，见下 |

窗口那一格的拆法（2400×1080，ms/帧）：zink = 渲染 2.25 + X 传输 17.66 + **回读 30.27** = 50.18；
llvmpipe = 渲染 2.01 + X 传输 18.12 + 回读 0（像素本来就在内存里） = 20.12，残差 −0.01 就是噪声，
这一格同时是模型的自检。要消掉回读 + 传输只能走 dma-buf 直传，而这台机的 X server 是应用进程
里的 bionic Xorg，拿不到 `/dev/dri`/`/dev/dma_heap`，DRI3 停在 `Could not get DRI3 device`。
做法与原始输出：[evidence 174](design/evidence/174-present-cost-split-20261125.md)。
