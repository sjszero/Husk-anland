# 当前状态

最后核验：2026-11-27（CST）
参考设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页
运行域：`untrusted_app`，无 Root、无 Shizuku、未解锁 Bootloader、无硬件虚拟化

本文只记录**当前代码和当前设备上已经复验过的状态**。历史版本变化见
[`CHANGELOG.md`](../CHANGELOG.md)，原始设备输出见 [`design/evidence/`](design/evidence/)。

## 1. 交付边界

Husk 是 Linux ARM64 用户态运行时，不是桌面发行版，也不是恶意代码安全沙箱。

- 交付资产是 loader、path-shim、glibc 补丁、rootfs 安装器、设备接口与 GPU 默认值。
- 资产必须放进 Android 应用私有目录，和 PRoot 一样由宿主 App 解压、安装并启动。
- App 或桌面环境负责窗口系统、输入注入和显示合成；Husk 的运行资产不包含 X server。
- 静态 ELF、raw syscall、不经过 loader 的进程不受 GOT 改写控制，不能用来隔离不可信代码。

## 2. 最终安装树

当前最终树由 `release-tree-v2.tar.gz` 在空目录安装产生，并接入 App 的正式入口：

```text
files/rootfs-full -> release-tree-final/root
```

关键结果：

| 项目 | 结果 | 证据 |
|---|---|---|
| 安装器退出码 | `INSTALL_RC=0` | 308 |
| Tree Integrity | `PASS=23 FAIL=0` | 308、310 |
| Gate A | `PASS=34 FAIL=0 SKIP=1` | 310 |
| A-hw | `PASS=11 FAIL=0` | 310 |
| GPU loader defaults | `PASS=9 FAIL=0 SKIP=0` | 310 |
| dpkg 记录 | 807 个 installed 包 / 807 个 `.list` | 308 |
| locale | `LANG=zh_CN.UTF-8`，413 个 zh_CN `.mo` | 308 |

Gate A 的唯一 SKIP 是离线树没有 apt package index。没有把 SKIP 计成 PASS。

## 3. Linux 用户态

| 能力 | 状态 | 说明 |
|---|---|---|
| aarch64 glibc 用户态 | 已验证 | glibc 2.39 参考树可启动 bash、coreutils、Python、GCC、make |
| ELF 装载与嵌套 `exec` | 已验证 | 动态解释器、shebang 和嵌套 exec 回到同一 loader |
| 路径语义 | 已验证 | 绝对路径落到 guest root；绝对符号链接由安装器改为相对链接 |
| PTY、信号、进程视图 | 已验证 | 门禁 A 覆盖身份、管道、子 shell、进程可见性和 SIGSYS |
| apt / dpkg | 已验证 | dpkg 记录完整；离线树没有 apt index，在线安装需先 `apt-get update` |
| 用户与组管理 | 已验证 | `husk-usermgr` / `husk-su` 替代不可用的发行版工具 |
| 中文默认值 | 已验证 | 未显式设置 locale 时默认 `zh_CN.UTF-8`；显式英文值保留 |
| GPU 默认值 | 已验证 | loader 从 `/etc/husk/gpu-env` 注入 ICD、Zink 和 GLES 设置 |

## 4. 硬件接口

分类口径只有四种：直连、中转、不需要、权限或厂商限制。逐项原始错误与理由见
[`design/hardware-interface-map.md`](design/hardware-interface-map.md)。

| 设备类 | 分类 | guest 接口 | 当前实测 |
|---|---|---|---|
| GPU | 直连 | `/dev/mali0`，PanVK + Zink | Mali-G77 MC9 枚举、compute、离屏 graphics、EGL/GL；Adreno/KGSL 仅源码与主机侧测试，真机 UNKNOWN |
| 相机 | 中转 | V4L2 `/dev/videoN` facade | 2026-11-27 复验：A-hw 11/0；前后摄各 3 帧 1280×720 NV12；V4L2/GStreamer fixture 两档通过；crop/selection 与 monotonic PTS 已验；DMABUF、通用 controls、非整帧裁剪未支持 |
| 音频 | 中转 | PulseAudio → Rust relay → AAudio | 输出与采集真实 PCM；采集探针 196,608 bytes、`peak=285` |
| 视频解码 | 中转 | VA-API → bridge → MediaCodec | H.264 1080p30 连续 600 秒，`dropped=0` |
| 传感器 | 中转 | IIO sysfs 影子树 ← `ASensorManager_*` | 加速度、磁力、陀螺仪、环境光、距离五类可读 |
| 显示 | 应用侧中转 | AHB → dma-buf → SurfaceControl | 3-buffer ring 与真实 `SYNC_FD` 跑到 10,000 帧；只覆盖应用侧接管段 |
| 输入 | 窗口层 | Android 输入 → 宿主 window/X11 | bridge 已实现；端到端依赖宿主窗口系统 |
| 定位 / USB / 蓝牙 | 权限或厂商限制 | 需要 Java 回调/Intent | 当前 `hasCode=false`，需要极小 Java 垫片或明确子集 |
| NFC / 指纹 | 不需要 | 无标准 POSIX 等价接口 | 不应为 Linux 用户态伪造 |

“直连”只用于 GPU。相机、音频、视频和传感器都是明确的中转；不能写成“全部直连”。

## 5. GPU

参考机上的 Mali/kbase 路径已经可用：

```text
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1 -> llvmpipe
HUSK_GPU=llvmpipe -> llvmpipe
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

同一份 `gl-probe` 的重负载结果：

| 负载 | GPU | llvmpipe | 结果 |
|---|---:|---:|---|
| 2400×1080，每像素 48 次 `sin`，2 遍 | 23.8 ms/帧（41.9 fps） | 266.7 ms/帧（3.7 fps） | GPU 快 11.2× |
| 1280×720，同一负载 | 14.9 ms/帧（67.1 fps） | 101.9 ms/帧（9.8 fps） | GPU 快 6.8× |

边界：

- `PANVK_DXVK_COMPAT=1` 的 geometry shader 崩溃是已知支持边界，默认值规避不等于修复。
- GLES 3.2 依赖 `MESA_GLES_VERSION_OVERRIDE`，真实应用未完整验证。
- 窗口内容进入宿主 X server 仍是回读 + SHM 拷贝；DRI3 在当前应用权限布局下不可达。
- Adreno/KGSL 没有真机证据，状态是 UNKNOWN。

## 6. 与 PRoot 的现场对比

2026-11-27 在同一台设备、同一份测试负载上做了一次单次采样：

| 工作量 | Husk | PRoot | 结果 |
|---|---:|---:|---|
| `stat` × 20 000 | 741.2 ms | 2,338.7 ms | Husk 快 3.16× |
| `open` + `close` × 20 000 | 980.9 ms | 5,676.3 ms | Husk 快 5.79× |
| shell 循环 × 1,000 | 18.2 ms | 51.5 ms | Husk 快 2.84× |
| `/bin/true` × 50 | 708.5 ms | 323.6 ms | **Husk 慢 2.19×** |

短进程启动是 Husk 当前真实代价：每次都要重新装载并完成 GOT 改写。历史 5 轮轮转数据与完整
方法见 [`BENCHMARKS.md`](BENCHMARKS.md)，原始输出见 evidence 311。

## 7. 显示宿主边界

当前 Husk 运行资产没有 `Xorg`、`Xvfb` 或应用内 X server：

```text
ls: files/rootfs-full/usr/bin/Xvfb: No such file or directory
ls: files/rootfs-full/usr/bin/Xorg: No such file or directory
ls: files/rootfs-full/usr/lib/xorg/Xorg: No such file or directory
```

因此：

- Linux 用户态、设备桥和 GPU 默认值可以独立验收。
- 独立启动 GNOME/X11 桌面属于宿主 App 集成，当前为 UNVERIFIED。
- 不使用其他 App 或旧 PRoot 树的 X socket 冒充 Husk 的结果。

## 8. UNKNOWN / UNVERIFIED

- 16 KiB / 64 KiB 页真机；当前只有 QEMU 证据。
- Adreno/KGSL、其他 Android 版本、其他厂商 seccomp/SELinux 策略。
- 定位、USB、蓝牙的完整链路。
- 传感器通过 `iio-sensor-proxy` 的 D-Bus 输出。
- 相机 DMABUF、通用 V4L2 controls、非整帧裁剪、真实 Camera2 的 GStreamer PTS、跨进程假 fd。
- 窗口内容到 X server 的零拷贝。
- 长时间负载型 soak；现有 30 分钟为空载，GPU 有 300 秒并发探针。

## 9. 发布门禁

发布前必须同时满足：

```sh
make check
DEV=<serial> sh tools/device-tests/20-tree-integrity.sh
DEV=<serial> sh tools/device-tests/14-gate-a-acceptance.sh
DEV=<serial> sh tools/device-tests/17-gate-a-hw.sh
DEV=<serial> sh tools/device-tests/18-gpu-loader-defaults.sh
```

并保证 `VERSION`、`CHANGELOG.md`、APK `versionName/versionCode`、annotated tag、Release 标题、
资产名、`SHA256SUMS` 与 release audit 全部一致。编译成功或单元测试通过不能替代上述真机门禁。
