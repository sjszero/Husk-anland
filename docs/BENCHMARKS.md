# 基准

一台设备、一份 rootfs、三个运行器，只换运行器。下面每个数字都是同一台手机上 5 轮的
中位数。

## 测了什么

| 项 | 值 |
|---|---|
| 设备 | RMX3357，MT6893（天玑 1200），Mali-G77，Android 13（SDK 33），kernel 4.19.191，4 KiB 页，arm64 |
| 沙箱 | `untrusted_app`，无 root、无 Shizuku、未解 Bootloader、无虚拟化 |
| 用户态 | Ubuntu 24.04，glibc 2.39，882 个 dpkg 包，一个 rootfs 目录 |
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

这张表只覆盖能在纯命令行里测的维度。**图形相关（图形栈启动、窗口、帧率）这一维不进表**：
本版不把桌面列为交付能力，rootfs 里也没有独立的 X server（`xvfb` 不在离线包索引里），
所以没有任何可比的图形数字——不是没测出来，是没有两个可比的被测对象。

第 7 维（空载 30 分钟的 RSS / 线程 / fd 增长与僵尸数）单独跑 soak，不放进计时的轮次里：
Husk 30 分钟 PSS 232 661 → 232 138 KiB、线程 47、fd 118、guest 僵尸 0（evidence 153）；
proot 与 proot-distro 的 soak 在 `docs/design/evidence/soak-*.log`。电池放电曲线未测：
adb 需要 USB，全程在充电。

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
- **图形。** 本版不提供。见本节开头：没有可比对象，也不写数字。
