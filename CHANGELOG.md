# 更新日志

本文件记录所有值得公开的变更。格式遵循
[Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本 2.0.0](https://semver.org/lang/zh-CN/)。

**每条结论都带证据编号**，原始输出在 `docs/design/evidence/`。0.0.1 之前的工程日记（36 条，
按时间顺序）在 [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md)。

## [未发布]

> 0.0.3 发布之后补做的实测，进下一版。

### 测过

- **关屏 150 秒再唤醒，会话没死**：同一 pid、线程 33 不变、fd 41→40、内存 +4 kB，唤醒后
  X 根窗口仍 100% 非黑（evidence 173）。注意：全程 USB 供电 + adb 在线，**不是**深睡/Doze。

## [0.0.3] - 2026-11-24

> 主题：**图形通路落到 GPU，并且默认就生效**。桌面不再是软件渲染——gnome-shell 46 跑在
> zink → PanVK 上出帧，默认值写在 guest 的 `/etc/profile.d/husk-gpu.sh`，不需要给应用单独配置。

### 修复

- **桌面在 X11 上由 GPU 出帧**：重编 PanVK 打开 x11 平台后 `VK_KHR_xcb_surface` 才会暴露
  （`panvk_instance.c:125` 的 `#ifdef` 之前被 `platforms=[]` 关掉），`gl-probe window` 从
  "死在 `eglMakeCurrent`" 变成 1→10 全过、60 次 swap；gnome-shell 46 在 zink 上起来并出帧，
  X 根窗口抓到完整桌面（evidence 165）。
- **mutter 启动崩溃（MIT-SHM `BadAccess`）的定位**：guest 的 SysV IPC 是 shim 用文件模拟的，
  X server 在 guest 外，拿整数 `shmid` 去 `shmat` 必然 `BadAccess`。这个机理是实测的（FACT）；
  但"让 `XShmQueryExtension` 返回 0"这条修法**未验证生效**（补测它仍返回 1），
  桌面不再崩另有原因，按 UNKNOWN 记（evidence 165 §2.1）。
- **假 DRM 节点现在"像"设备**：`/dev/dri/renderD128`、`card0` 的 `stat`/`statx`/`fstat`
  回真 `/dev/mali0` 的字符设备号（10:33）+ 0666，并合成
  `/sys/dev/char/<maj>:<min>/device/uevent`、`/sys/class/drm/renderD128/{dev,uevent}`、
  `/proc/bus/pci/devices`（空表）。这是 Chromium/Electron 与 GStreamer VA 认可 GPU 的前提；
  DRI3 零拷贝仍未通（evidence 165）。
- **假 DRM 节点的 dup 记账**：`dup`/`dup2`/`dup3` 与 `fcntl(F_DUPFD)`/`fcntl64` 以前不认假 fd，
  复制出来的渲染节点（`rdev=0:0`、memfd）上的 DRM ioctl 全部落空，客户端退回软件渲染。
  现在副本也报 `rdev=10:33`（evidence 166）。
- **libc 内部调用少打 5 个 hook 而一直没人发现**：105 个 hook 里
  `__readlink_chk`、`__readlinkat_chk`、`__realpath_chk`、`__mq_open_2`（入口窗口里有分支跳进来）
  与 `__openat64_nocancel`（这个 libc 根本没导出）一直被 interposer 拒绝，旧的 shell 脚本不做
  集合比对，所以"装上了"和"少 5 个"看起来一样。应用层这 5 个名字仍由 shim 的 GOT 覆盖，
  libc **内部**的直调仍然落到 host —— 这是已知缺口，本版只是把它变成会失败的断言，没有修
  （evidence 170 §3）。
- **发布启动器包里没有 GPU 默认值**（打包缺陷）：`make dist` 打的 `husk-arm64.tar.gz` 一直
  没带 `guest-config/husk-gpu-defaults.sh`，而安装器那个 `if [ -f … ]` 没有 else，于是从发布包
  装出来的树**没有** `/etc/profile.d/husk-gpu.sh`、也没有任何提示。现在包里带上了，`make dist`
  在打完的 tar 上断言它存在，安装器缺文件时打 WARNING（evidence 172）。
- **`make-release.sh` 会悄悄打出带 `tmp/proc/dev/run/root` 的包**（打包缺陷）：排除表用
  here-document 写，而 mksh 的临时目录在 `run-as` 下不可写，脚本只有 `set -u`，于是排除表是空的、
  `tar -X` 拿到空表。改成 `printf` + 非空检查 + 失败即退出，并加前置拒绝：
  树里出现 `data`/`storage`/`sdcard`（guest 里造出来的宿主路径自镜像）就不许打包（evidence 172）。
- **假 DRM fd 只进不出**：`close()` 从不注销 DRM 假 fd，8 个槽位用满之后新开的节点不再被接管；
  `close()`/`close_range()` 现在会注销，槽位真的用完时会打一条日志（evidence 166）。

### 测过

- **桌面 30 分钟空载长稳**：PSS 137.3 → 139.9 MB（前 60 秒启动分配，之后 29 分钟 +11 KB），
  线程 33 / fd 41 恒定、僵尸 0、同一 pid；结束后 X 根窗口 2400×1080 仍 100% 非黑
  （evidence 169）。八维第 7 维补上 GPU 桌面这一行；睡眠/唤醒与负载型 soak 仍未测。
- **桌面在重绘，不是静态位图**：同一会话四帧根窗口指纹，空载两帧逐字节相同、开关一次
  GNOME 概览各换一帧（evidence 171）。

### 测过

- **发布产物端到端**：`make dist` 的启动器包 + 从验证过的树打出的 rootfs tar → 全新目录装一遍
  → 门禁 A **34 PASS / 0 FAIL**（少的 2 项是发布包不含 apt 缓存、`dpkg-deb` 重装那两条被跳过，
  不是失败）。树包 102 110 条目、1.42 GB，设备侧与主机侧 sha256 一致（evidence 172）。

### 工具

- **libc 补丁链 Rust 化**：`nr-tools libc-patch`（`make libc-patch` / `make libc-verify`）
  取代 `tools/guest-config/install-libc-patch.sh`（已删）。除了原来那两步变换，它把"结果对不对"
  变成断言：glibc 版本绑定、slot 集合必须等于 `libc-interpose.expected`、`clone3` 归零、
  `rseq` 不被动、4 KiB/16 KiB/64 KiB 页都对齐；失败就中止，不再出现"推了一半也算成功"。
  同一份 stock libc 打出来的产物与仓库里 `tools/loader/libc-nop.so.6` **逐字节一致**（evidence 170）。
- `gl-probe root [out.ppm]`：用 `XGetImage` 读 X 根窗口并写 PPM，再打印非黑像素与直方图。
  手机 `screencap` 抓的是 App 自己的 UI，不能当 guest 出帧的证据。
- `gl-probe window` 现在打印 swap 循环的耗时与 fps（`stage 10: 60 swaps OK (506.8 ms, 118.4 fps)`），
  可用于 GPU / CPU 两条路径的 A/B。

### 文档

- `docs/BENCHMARKS.md` 补上第 5 维（桌面与图形）与第 6 维（交互延迟）的数字，并把
  「图形不进表」那段替换成这些数字能说明什么、不能说明什么（evidence 168）。
- `docs/LIMITATIONS.md` §7 更新：WSI 已经通了但呈现是 SHM 拷贝；桌面由 GPU 出帧，
  但轻负载 fps 不如 llvmpipe（照写）。

## [0.0.2] - 2026-11-24

这一版没有新功能，只有「装得上、跑得稳、说得准」。全部结论来自目标机实测
（`docs/design/evidence/156`、`157`）。

### 修复

- **独立安装（不装 APK）真的能用了**：`/data/local/tmp` 里从零装、起 `bash`、`apt-get update`
  拉 38.1 MB、`apt-get install` rc=0、程序能跑（evidence 157）。此前有六个问题挡着：
  `make dist` 打出 644 的 loader；toybox `gzip -dc` 写完仍以 1 退出把好包判成坏包；
  分派漏了 plain tar 分支；启动器把 `-c` 当程序名；缺 `etc/resolv.conf`；缺 `var/log/apt`。
- **安装器不再吞错误**：pass 1 的 `tar … | sed … || true` 把 tar 的退出码丢进管道，
  坏包可以"装成功"。现在落日志再判码，只容忍 SELinux 拒 `link(2)` 这一类，其它 exit 68。
- **绝对符号链接在建装时就被规范化**：新增 `husk-symlinks`（Rust），`--apply` 改相对、
  `--verify` 确认没漏网；再用 `ctypes.CDLL('libblas.so.3')` / `liblapack.so.3` 验 soname。
  实测 29 476 条链接、5 条绝对、改 4 条、1 条悬空、0 条残留。
- **libc 被 `apt upgrade` 换掉会被发现并恢复**：`.husk-install-state` 记 libc 与 loader 的
  sha256，`husk-session.sh` 每次开会话前比对；能恢复就恢复，不能就 exit 74 并说清原因。
- **`make check` 名不副实**：它只做 cargo check，别的门禁散在 `lint`/`docs`。现在
  `check` = fmt + cargo check + clippy + 53 个脚本 `sh -n` + 文档链接 + 版本三处一致。
- **许可证元数据冲突**：workspace 写 `MIT`、LICENSE 是 Apache-2.0，14 个 crate 还各自写死 MIT。
  统一成 `license.workspace = true` + Apache-2.0。

### 变更

- **GPU 接进 0.0.1 之后的树**：kbase 后端的 PanVK（FristOneRR）与它的 ICD 进包，
  `vulkan-probe` 枚举到 `Mali-G77 MC9` 且 compute 回读 `0x4855534b`；会话默认把 GL 交给
  Zink，renderer 从 `llvmpipe` 变成 `zink Vulkan 1.3(Mali-G77 MC9)`（evidence 156）。
  `HUSK_GPU=llvmpipe` 可退回软件路径。**上限仍是 GLES 2.0，窗口呈现（WSI）未做。**
- `make dist` 打 `tools/loader/libc-nop.so.6`（与目标机树上一致），不再打 `binaries/` 里
  11-22 的旧件；包里增加 `husk-symlinks` 与 `VERSION`。

### 文档

- LIMITATIONS §7/§8 按实测重写，并补一张「能力 → 状态 → 证据」表；COMPATIBILITY 补 GPU 段；
  BENCHMARKS 写清 `make -j4` 两个数字（5 194 / 3 509 ms）为何不可互算。
- README 与 SECURITY 明写「不是恶意代码沙箱」，列出 raw syscall、静态 ELF、非 loader 进程
  三类不受控路径。
- 顶层文档全中文；去掉仓库里的设备序列号与本机代理地址；删掉多余的 `v0.1.0` 标签。

### 更正

- 本版最初按「小数位加一」写成 `0.02` / `v0.02`，两段式不是合法 SemVer；发布当天更正为
  `0.0.2` / `v0.0.2`，上一版同理（`0.01` → `0.0.1`）。代码与资产内容不变，只有版本字符串改了。
  规则现在写死在 `CONTRIBUTING.md`：patch 位加一，**发出去的 tag 不再改名、不再移动**。

## [0.0.1] - 2026-11-24

首个发布版本。参考环境：arm64、Android 13、kernel 4.19.191、4 KiB 页，以 `untrusted_app`
运行，无 root。机型与 SoC 只是那一次实测的现场，不是运行时的要求。

### 新增

- **原生运行时。** 一个内嵌 path shim 的 aarch64 loader，让未经修改的 glibc 用户态跑在同一个
  进程树里：无 `ptrace`、无 namespace、无虚拟化、无 root。路径虚拟化在装载时做一次——改写
  每个被装载对象的 GOT——之后程序自己的 `svc` 原样进内核（evidence 130–146）。
- **包管理器可用。** `apt-get install` 27 秒把树从 815 包装到 882 包，含 `build-essential`、
  `git`、`vim`、`nodejs`、`python3-numpy`（evidence 154）。
- **工具链完整。** `gcc`、200 目标工程的 `make -j4`、`git clone`、`python3` + `numpy`
  （evidence 155）。
- **会话服务可用。** polkit 授权（`pkcheck` rc=0）、at-spi 无障碍总线、gvfs 不回退到会话总线、
  RandR 1.2（`xrandr` 无缩放告警）（evidence 151、152）。
- **三运行器基准夹具**（`tools/bench/`）：同一份 rootfs 交给 Husk / proot / proot-distro，
  只换运行器，数字见 [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)。
- **独立安装形态**：`tools/guest/install-husk.sh` 装一份 rootfs 并生成 `./husk` 启动器，
  不需要 JVM、不需要 Python（[`docs/RUNNING.md`](docs/RUNNING.md)）。
- **版本自证**：`./husk --version` 打印 `VERSION` 文件里的版本号，`make version-check` 保证它
  与本节标题、release tag 三处一致。

### 变更

- Android 侧集成里的 proot 已移除：同一份 rootfs 只经 loader 启动，功能没有损失
  （evidence 144）。

### 修复

- shim 缺 `statvfs` / `statvfs64`，导致 `apt` 量的是宿主路径的空闲空间（evidence 154）。
- 发行版自带的静态 `ldconfig` 看不见 guest，换成 `husk-ldconfig`（evidence 154）。
- `execve` 把 argv 放在 64 槽的栈数组里，第 63 个之后的参数被静默丢掉；表现为 200 目标工程
  的 `make -j4` 报了指向 `__TMC_END__` 的链接错误，而不是指向 argv（evidence 155）。
- rootfs 里 552 个绝对符号链接按 Android 根解析，改成相对链接后 `numpy`、`libblas`、
  `liblapack` 才能按 soname 装载（evidence 155）。
- `nr-sigsys.log` 噪声：一次会话一行，而不是 4 499 行（evidence 152）。

### 运行时上限（不属于本版交付能力）

这些是运行时被验证到过的上限，**0.0.1 交付的资产里不含桌面那一层**，它们由应用侧提供：

- GNOME Shell 46 的 X11 会话能在 loader 下起完整一帧（evidence 147）。
- 触摸经 `TouchInputHandler` → 进程内 X server → XI2 → mutter 落到窗口（evidence 148）。

### 已知问题

- 桌面渲染是软件渲染。guest 的 GPU 路径存在，但没接到桌面那条链上
  （[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) §7）。
- 传感器、GPS、USB、蓝牙、NFC、指纹、硬件视频解码、WSI：未开始。
- 随附 rootfs 里没有浏览器包；Termux 不是支持的环境。

[未发布]: https://github.com/xishanp/Husk/compare/v0.0.3...HEAD
[0.0.3]: https://github.com/xishanp/Husk/releases/tag/v0.0.3
[0.0.2]: https://github.com/xishanp/Husk/releases/tag/v0.0.2
[0.0.1]: https://github.com/xishanp/Husk/releases/tag/v0.0.1
