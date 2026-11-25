# 更新日志

本文件记录所有值得公开的变更。格式遵循
[Keep a Changelog 1.1.0](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本 2.0.0](https://semver.org/lang/zh-CN/)。

**每条结论都带证据编号**，原始输出在 `docs/design/evidence/`。0.0.1 之前的工程日记（36 条，
按时间顺序）在 [`docs/design/DEVLOG.md`](docs/design/DEVLOG.md)。

## [0.0.6] - 2026-11-25

> 主题：**把"桌面会话"从一堆手搓命令收成一个入口，并把三件长期挂在"根因 UNKNOWN"上的事钉死。**
> 0.0.5 说的是"硬件怎么接"，0.0.6 说的是"这个会话凭什么能稳定起来、以及它哪里还坏着"。
> 这一版新增了**一条用户可见的边界**（应用被冻结时桌面会停摆），也把一个被当成 bug 的现象
> 明确降级成已知限制。

### 改

- **新增 `husk-desktop.sh`（会话单一入口）**，安装器落盘（evidence 200、203）。
  清残留进程 → 清 `gnome-shell-disable-extensions` / 陈旧 `bus` / `gvfsd` / `at-spi` →
  起系统总线与会话总线 → 起 shell → 等 `_NET_SUPPORTING_WM_CHECK` → 自检 → 拉起壁纸看护。
  实测输出 `CHECK wm=yes` / `CHECK ding=yes` / `CHECK gpu=zink`，`EXIT=0`，两次独立会话均可复现。
  两条硬坑写死在注释里：**dbus 地址必须用 guest 侧路径**（宿主路径会二次加 rootfs 前缀，
  main.js 直接抛异常）；**后台进程必须重定向 stdin/stdout/stderr**（否则调用方 `| tail` 挂住）。
- **发布包补齐运行时要用的脚本**：`make dist` 以前只把 `install-husk.sh` / `husk-session.sh`
  打进 tarball，`husk-desktop.sh`、`husk-ding-fallback.sh`、`husk-wallpaper-fit.sh`、
  `guest/husk-wallpaper-scale.js`、`guest-config/pulse-client-no-shm.conf` 全都不在里面。
  后果是"从 Release 的 tarball 装出来的树"和"开发机上的树"不是同一棵树：DING 兜底永远为假、
  壁纸不做预缩放、pulse 客户端静默丢掉 `enable-shm=no`（就是那个"装了 pulse 却没声音"）。
  现在五个文件都进包，并加了逐条 `tar tzf | grep -qx` 断言——少一个就构建失败，不再靠人记。
  安装器也补上把 `husk-ding-fallback.sh` / `husk-wallpaper-fit.sh` 放到 `husk-session.sh` 同级。
- **壁纸缺失/失效兜底**（evidence 199）：`husk-wallpaper-fit.sh` 在空 URI 时按
  `husk-default.png` → 内置壁纸 → 任意 png/jpg 兜底，并复位 `primary-color`；
  安装器此前只装 `husk-wallpaper-scale.js`、**从没装 `husk-wallpaper-fit.sh`**，
  于是 `husk-session.sh` 的可执行判断永远为假——这一处一并补上。
  验收：跨整会话重启的六个采样点逐字节一致。

### 测过（数字）

- **输入延迟**（evidence 201，3 轮 × 100 次，同进程 `XTestFake*` + `XSync` 往返）：
  键盘中位 **1.04–1.21 ms**、按钮中位 **0.25–3.09 ms**，最小 0.14 ms ——
  path-shim 与 X server 的请求通路**不是**瓶颈；尖峰 **8.3 / 30.3 ms** 正好是 120 Hz / 33 Hz
  一帧，是被批量到下一个 grab/frame 之后投递，视为一拍延迟（INFERENCE，未做帧计时器对拍）。
  限定：不含 `/dev/input` → X 这一段，也不含合成器 → 屏幕扫描，**不等于手指到画面的延迟**。
- **拖动窗口**（evidence 203 复核）：`Super`+拖位移 **0**（`236,188` → `236,188`）；
  `alt+F7` + 指针 motion 位移有效（→ `186,120`）。
- **发版门禁 A：PASS=36 FAIL=0**（evidence 204）。第一次跑是 35/36，失败项是
  `guest /data/user does not exist`——rootfs 里有一条 14 层深、32 KB 的空目录链，
  正好等于把 `$ROOTFS` 的宿主绝对路径当成 guest 路径 `mkdir -p` 了一次
  （时间戳 16:17，会话调试期间）。已清；成因记 UNKNOWN 并写明候选，
  这条门禁断言就是探测器。
- **`/proc/self/mountinfo` 的 EPERM 定根因**（evidence 202）：虚拟 proc 文件全是 **memfd**
  （`S_IFREG`、`dev 0:5`、`/memfd:husk-virt (deleted)`），而 libmount 的内核挂载监视只做
  `epoll_ctl(ADD)`，**内核对普通文件一律 EPERM**。不是 mountinfo 独有，`/proc/uptime` 一样。

### 已知问题（本版未修，已核实）

- **窗口拖不动、连带对齐/吸附**（限制 §10）：坏在 mutter 的 grab op，位置在 X server 侧；
  客户端请求、X 层被动抓取、「mutter 没用 `XIGrabDevice`」三条全部实测排除。
  替代操作 `alt+F7`。给 2.0 侧的复现步骤在 evidence 203 §4。
- **挂载变化不再有通知**（限制 §11 之外的另一条，evidence 202）：`GUnixMountMonitor` 拿不到
  channel 后**既不挂 watch 也不退回轮询**（3 秒轮询只属于"找不到 mtab 路径"那条分支），
  于是 `udisksd` / `gvfsd` / `nautilus` 都只在主动读时才看到最新挂载表。
  修法三条已列（pipe 承载 / hook `epoll_ctl` / 不动），前两条各自的代价写在 evidence 202 §5，
  本版选"不动"，因为它**不产生错误行为**，只产生噪音与延迟。
- **Nautilus 的 `This app cannot work correctly if run as root`**：会话真 uid 就是 0，
  假身份（`NR_FAKE_UID`）被 dbus 的 EXTERNAL 认证挡死（evidence 200）。要消只能改
  gnome-shell 资源或预载层丢 `Notify`——都是动别人代码，0.0.6 不做。
- **屏幕熄灭后桌面"像死了一样"**（限制 §11）：应用转 cached 被 Android Cached Apps Freezer
  整体 SIGSTOP，X server 住在应用进程里，于是新连接一律卡住。判据是
  `/proc/<x11 pid>/task/*/wchan` 全为 `get_signal` 且 `dumpsys power` 显示 `mWakefulness=Asleep`；
  唤醒即恢复，**不是崩溃**。

### 文档

- [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)：§8 输入一行补"拖动失效"；新增 §10（拖动缺陷与
  `alt+F7` 替代、三条已排除路径、给 2.0 侧的三步复现）、§11（冻结判据与"别当崩溃查"）。
- 新增 evidence 199 / 201 / 202 / 203；198 追加两次（XI2 设备树、mutter 不用 `XIGrabDevice`）。

## [0.0.5] - 2026-11-25

> 主题：**说清楚「硬件到底怎么接」，并把图形默认值从登录脚本下沉到加载器。**
> 0.0.4 把代价钉死了，0.0.5 把「谁在什么地方吃到 GPU」这件事从"登录 shell 才算"改成
> "经加载器起来就算"，同时把接口口径收成一根轴。

### 改

- **图形默认值下沉到加载器**（evidence 182）。`getenv` / `secure_getenv` / `__secure_getenv`
  由 path-shim 提供，值来自安装器写的 `/etc/husk/gpu-env`；查询顺序是**先 `environ`，
  查不到才用默认值**。于是登录 shell、cron、systemd user unit、桌面 fork 出来的进程、
  `env -i` 看到的是同一套渲染器：`env -i` 起 `gl-probe es3`
  → `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`（改动前是 `llvmpipe`），两者都 PASS。
  - 程序自己 `export` 的变量永远优先（只覆盖一个变量时其余仍取默认值，实测过）；
  - `HUSK_GPU=llvmpipe` / `NR_GPU_DEFAULT_OFF=1` 一条都不注入；
  - `secure_getenv` 尊重 `AT_SECURE`；
  - 树里没有 `husk_*.json` 时 `/etc/husk/gpu-env` 保持空文件，行为与改动前完全一致；
  - 门禁 A：**PASS=36 FAIL=0**。

### 已知问题（本版未修，已核实）

- **桌面会话形态没验过**：本仓库的验收只到「X 根窗口非黑像素 100%」「帧有变化」
  「GTK 客户端出画」「键盘能敲进去」，**没有**验「这是不是一个带主题的正常会话」。
  实测这台机器上主题文件是齐的（`/usr/share/themes/{Adwaita,Adwaita-dark,…}`、
  `gnome-shell-theme.gresource` 710 151 B、`gnome-shell-common` / `yaru-theme-gnome-shell` /
  `gnome-themes-extra` 都是 `install ok installed`），所以「主题丢了」不是缺包，更像会话侧
  （session mode / dconf / gnome-settings-daemon 的 xsettings）没把主题用起来——**根因 UNKNOWN**，
  要开着会话抓 `gnome-shell` 日志才能定。
- **窗口拖不动、位置不对齐**：输入默认是 trackpad 语义（2.0 侧 `touchMode` 默认值不在本仓库），
  触摸拖动在 mutter 里不是「按住标题栏拖窗口」。现有证据只证明「点哪落哪」（evidence 148、180），
  **拖动 / 对齐 / 吸附没有测过**。

### 文档

- 新增 [`docs/design/hardware-interface-map.md`](docs/design/hardware-interface-map.md)：每类硬件
  一根轴——直连（GPU、相机）/ 中转（音频、视频）/ 未做，并附 2026-11-25 在应用域实测的节点布局
  （`/dev/mali0` = `10,33` `0666`、`/dev/video0` = `81,0`、`/dev/dri` 只有 `card0`、
  `renderD128` 不存在、`/dev/snd` 打不开）。
- [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)：§8 表加「接法」列；§7 补两件实测——
  呈现那次拷贝是**架构共性而非本机带宽**（guest 内 `memmove` 10.05 GiB/s，2400×1080 一帧
  值拷贝下限 1.0 ms，对实测回读 30.27 ms/帧），以及「三条可选路线各自落在哪一层」。
- README：硬件接口按「直连 2 类 / 中转 2 类 / 显示零拷贝 / 输入窗口层 / 未做 5 类、指纹不做」
  重写，并明确「全部直连」和无限定的「零拷贝」都是不能说的话。

## [0.0.4] - 2026-11-25

> 主题：**把「GPU 到底快多少」和「哪里确实还坏着」用数字钉死**。0.0.3 说的是「桌面由 GPU 出帧」，
> 0.0.4 说的是「哪类负载快多少倍、呈现那一段贵在哪、哪些真实客户端画不出来、以及为什么」。
> 这一版没有新增能力面，全部是测量、定位与工具。

### 测过

- **GPU 原生加速按负载分开量**（2400×1080，本机日常分辨率）：着色器密集负载 GPU 23.84 ms/帧
  （41.9 fps）对软件 266.73 ms/帧（3.7 fps），**快 11.2×**；纯清屏离屏 2.25 vs 2.01 ms；
  清屏 + 窗口呈现反而是软件快（19.9 vs 49.7 fps）——差在每帧把 GPU 表面回读再交给 X server
  （30.27 ms/帧 @ 2.59 兆像素），但 llvmpipe 同尺寸的残差是 −0.01 ms，说明模型成立
  （evidence 174）。
- **窗口 swap 拆成「渲染」和「呈现」两段**：GPU 渲染在 160×120→1080×1080 之间都是 1.2–1.7 ms，
  呈现按窗口面积线性涨（zink 24.8 ms/兆像素，llvmpipe 11 ms/兆像素）。640×480 的 8.9 ms 里
  GPU 只占 1.3 ms，其余是 GPU→CPU 回读 + 进 X server 的拷贝；DRI3 在这台机上没有设备可用
  （evidence 174）。
- **关屏 150 秒再唤醒，会话没死**：同一 pid、线程 33 不变、fd 41→40、内存 +4 kB，唤醒后
  X 根窗口仍 100% 非黑（evidence 173）。注意：全程 USB 供电 + adb 在线，**不是**深睡/Doze。
- **真实 GTK4 客户端跑通**：GTK 4.14 的 `gnome-text-editor`（`GSK_RENDERER=ngl`）在 zink/PanVK 的
  X11 上开窗并画出完整界面，X 根窗口 2400×1080 抓图 28 565 种颜色、非黑；同时记录
  `KBASE_IOCTL_MEM_EXEC_INIT` 被拒（EPERM）但渲染正常，原因未查清（evidence 176）。
- **文件管理器实测是个反例**：`nautilus` 46 能开窗、能枚举（日志逐条打出 `file:///etc/*`），
  但内容区整块空白、连列头都不出现；`GSK_RENDERER=cairo` 与 `ngl` 一样、`list-view` 与
  `icon-view` 一样，**同一份 rootfs 换 proot 起也照样空白**，而同一会话里 `gnome-control-center`
  与自写的最小 `GtkColumnView` 程序都正常——所以不是 path-shim 造成的，根因 UNKNOWN
  （evidence 178，`LIMITATIONS` §7/§8）。
- **5 个打不上的 libc hook 核清四条、留一条 UNKNOWN**：反汇编证明 `__readlink_chk` /
  `__readlinkat_chk` / `__realpath_chk` / `__mq_open_2` 的"跳进窗口"是它们自己跳自己，
  真正的活是尾调用已接管的入口；`__openat64_nocancel` 这版没导出符号，标 UNKNOWN
  （evidence 175，`LIMITATIONS` 新增第 9 节）。
- **键盘输入打通到 GTK4 客户端**：XTEST 敲进 `gnome-text-editor`，标题栏与正文都跟着更新、
  右边界折行正常，改窗口尺寸触发全量重绘后内容一致、无残影。只测键盘，没量延迟，
  IME/组合键未测（evidence 180）。
- **`glReadPixels` 的机制量出来了**：目的行距恒为 16 字节（与宽度无关），每行只有前 4 个像素
  落地，写入量 = `H×16 + (W×4 − 16)`；换 `PACK_ROW_LENGTH` 无效、`PACK_ALIGNMENT=1` 更糟。
  **逐行读与逐像素读 100% 正确**，1×H 单列读同样坏。llvmpipe / zink / proot 三处完全一致
  → 不在 path-shim、不在 GPU 后端，落在这版 Mesa 的公共读回层（具体文件 UNKNOWN）
  （evidence 179）。

### 改进

- **CI 入仓**（`.github/workflows/ci.yml`）：`fmt` / `cargo check` / `clippy` / shell 语法 /
  文档链接 / 版本一致性，另有一个 libc 作业——从锁定的 `libc6 2.39-0ubuntu8.12` arm64 包重建
  `libc.so.6` 并校验 sha256 `69ceef37…`，所以「补丁版 libc 与包里的那份是否一致」不再靠人手比对。
- `gl-probe` 新增 `readback [WxH]`：把每像素唯一色的纹理挂到 FBO 上整块读回，用值本身反解
  「这块数据原本是哪一格」，并把「整块 / 逐行 / 逐列 / 单点」四种读法一次报全。这是查
  `glReadPixels` 缺陷的探针，也是 evidence 179 的取证工具。
- `gl-probe` 的 `window` 子命令接受 `WxH` 和帧数（原来是写死的 640×480 / 60），并新增
  `pbuf WxH N` 离屏基线：同一套 EGL/GL、同样 clear+swap，但每帧 `glFinish`，用来把 GPU 渲染
  和呈现分开量。不带 `glFinish` 的离屏版本会量到 0.1 ms/60 帧的假数字。

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
[0.0.6]: https://github.com/xishanp/Husk/releases/tag/v0.0.6
[0.0.5]: https://github.com/xishanp/Husk/releases/tag/v0.0.5
[0.0.4]: https://github.com/xishanp/Husk/releases/tag/v0.0.4
[0.0.3]: https://github.com/xishanp/Husk/releases/tag/v0.0.3
[0.0.2]: https://github.com/xishanp/Husk/releases/tag/v0.0.2
[0.0.1]: https://github.com/xishanp/Husk/releases/tag/v0.0.1
