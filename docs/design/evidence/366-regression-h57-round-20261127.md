# 366 · H57 之后的全量真机回归（0.0.37 收尾轮）

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页 / 2400×1080 横屏
测试宿主：`com.linuxhub.myth.husk`（黑盒运行容器，未改其源码；版本 `Myth-Prometheus`，targetSdk 28）
guest 树：`files/linux/distributions/ubuntu-24.04/rootfs`
Husk：`VERSION=0.0.37`、提交 `c924d620f6de`（工作区未提交）

本文件只记录**这一轮**跑出来的数字。未跑到的写 NOT RUN；失败但原因是外部条件的写“外部阻塞”，不算通过。

## 0 · 结论表

| 门禁 | 结果 | 判定 |
|---|---|---|
| 部署身份自证（`23-runtime-identity.sh`） | `PASS=11 FAIL=0` | FACT |
| `20-tree-integrity.sh`（EXPECT=desktop） | `PASS=23 FAIL=0` | FACT |
| `14-gate-a-acceptance.sh` | `PASS=38 FAIL=0 SKIP=0` | FACT |
| `24-icd-canonical.sh` | `PASS=15 FAIL=0 SKIP=0` | FACT |
| `19-readback-suite.sh` | `PASS=19 FAIL=0`，`READBACK_AUDIT PASS=6 FAIL=0` | FACT |
| `18-gpu-loader-defaults.sh` | `PASS=9 FAIL=0 SKIP=0` | FACT |
| `15-gpu-global-audit.sh` | `PASS=3 FAIL=0 SKIP=0` | FACT |
| `25-self-exe-exec.sh` | `PASS=4 FAIL=0` | FACT |
| `01-functional-matrix.sh` | `#checks=31 fail=0` | FACT |
| `16-umount-legacy.sh` | `PASS=12 FAIL=0` | FACT |
| `21-gpu-soak-audit.sh`（4×300 s×2400×1080） | `PASS=19 FAIL=0 SKIP=0` | FACT |
| `make test-device`（真机 guest 单测） | `UNIT_TESTS: PASS` | FACT |
| 相机 fixture（raw + GStreamer，不需要相机权限） | `camera-fixture: PASS`，640×480 与 1280×720 | FACT |
| `17-gate-a-hw.sh` | `PASS=4 FAIL=11` | 外部阻塞，见 §4 |
| `22-audio-audit.sh`（`make audio-audit`） | `PASS=1 FAIL=3` | 外部阻塞，见 §4 |
| H57 黑楔子（活桌面复核） | 左右工作区缩略图各 0/56052 近黑像素 | FACT |

## 1 · 这次部署的是什么（自证）

```
version=0.0.37
git_commit=c924d620f6de
mode=upgrade
loader_sha256=9a22094281c0eaa66a0d31f8bfcf12dde2011e1a63e26ba40edac1f81b3c0274
libc_sha256=16d5ba2120c1a92c0c9cba62dd8ec244d29d5d30a733d4ac530be4fb1b92933f
gpu_env_sha256=ad230fcc0f8248ccfbf657e0b28e00bcf28cd41f3e644bdc1788f522f4445603
panvk_driver_sha256=5e06324783ec7947d48782441fd1cac23953c90bd0b31663f98f08b22344c1c3
icd_sha256=25f6ccd9662ada7072b43d87aaa45dcf9413519918a30d55679b60042afd0eae
```

`23-runtime-identity.sh` 在设备上把这五项**重新算一遍**再和记录比对，并核对
`VERSION`、`dist/RUNTIME-IDENTITY` 的 commit/version/loader：`PASS=11 FAIL=0`。

### 1.1 上一轮留下的真实问题：身份文件是旧的

本轮开工第一件事就撞上了：`cat rootfs/etc/husk/runtime-identity` 给的是

```
loader_sha256=67f543b9…   panvk_driver_sha256=f73600da…
```

而磁盘上的实际文件是 `9a220942…`（带崩溃栈的 loader）和 `5e063247…`（源码构建的
H57 驱动）。原因是 H57 那一轮**只换了文件、没跑安装器**，身份文件停在 8 小时前的
那次 `--upgrade`。这正是 P0 里“设备上必须能证明跑的是这份 Husk”要抓的东西：
文件换了、记录没换，任何一个读身份的门禁都会把结果记到错误的构建上。

处置：`make dist` 重出资产 → 用发行包本身做 `--upgrade` → 让安装器重写身份。
之后 `23-runtime-identity.sh` 才允许出数字。这条规则落成两个脚本：
`26-upgrade-in-app.sh`（唯一被认可的部署入口）和 `23-runtime-identity.sh`（唯一被
认可的自证入口）。

## 2 · 发行产物

```
$ make dist
runtime boundary: PASS (dist/husk-arm64.tar.gz)
ec65fab4d093ea85a57ee5f1631cc63716d14bbd4bc67d976ca630c1c374baac  husk-arm64.tar.gz
```

loader 与上一次构建逐字节相同（`9a220942…`），即 `make loader` 仍可复现。
**注意**：`husk-arm64.tar.gz` 的 SHA 每次重打都会变（gzip 头里的 mtime），
`dist/RUNTIME-IDENTITY` 里的 `loader_sha256` 和 `SHA256SUMS` 才是可比对的字段；
发布时必须连同 `SHA256SUMS` 一起发，不能只报 tarball 的 SHA。

## 3 · 门禁明细（都在 §1 那次部署上跑）

- **20-tree-integrity（EXPECT=desktop）**：23/0。包含 `zh_CN langpack 413 .mo`、
  `gdk-pixbuf 14 loaders`、`dpkg 896 .list / 896 包`、`DING skips unreadable /proc entries`。
- **14-gate-a**：38/0/0。bash 5.2.21、locale（星期一）、useradd/chpasswd、dpkg、apt、
  无宿主路径泄漏、`/proc/self/exe` 干净、真实 SIGSYS 陷阱 0。
- **24-icd-canonical**：15/0/0。清单指向、旧副本清空、驱动 SHA、`gpu-env`、loader 自扫 icd.d。
- **19-readback**：19/0，`READBACK_AUDIT 6/0`，renderer
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，`OpenGL ES 3.2 Mesa 27.0.0-devel`。
- **18-gpu-loader-defaults**：9/0/0。含两个反向控制：`NR_GPU_DEFAULT_OFF=1` → llvmpipe、
  `HUSK_GPU=llvmpipe` → llvmpipe。
- **15-gpu-global-audit**：3/0/0。`gnome-shell pid=… gpu=zink gsk=cairo`、
  `glxinfo` 看到 `zink Vulkan 1.3(Mali-G77)`、嵌套 GTK4 进程同一硬件链。
- **25-self-exe-exec**：4/0。`exec /proc/self/exe` 重新进入本程序、选项错误来自 bash 自己。
- **01-functional-matrix**：31 检查 0 失败。
- **16-umount-legacy**：12/0（legacy 一参数 `umount()` 也走 `umount2`）。
- **21-gpu-soak**：见 §5。
- **相机 fixture**：`camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)`，
  不需要 Camera2 权限，证明 guest 侧 V4L2 门面本身是通的。

## 4 · 外部阻塞（不是 Husk 缺陷，也不算通过）

### 4.1 权限：不是“没点授权”，是**根本没申请**

本轮按用户要求先试了给应用提权，结论是这台设备上走不通，两个原因叠在一起：

1. 宿主 APK（`Myth-Prometheus`）的 `requested permissions` 里**没有** `CAMERA`、
   `RECORD_AUDIO`、`ACCESS_COARSE_LOCATION`。没申请的权限无法被授予。
2. 即使申请了，`adb shell`（uid 2000）在这台 ROM 上也没有授权能力：

```
pm grant com.linuxhub.myth.husk android.permission.CAMERA
  → SecurityException: grantRuntimePermission: Neither user 2000 nor current process
    has android.permission.GRANT_RUNTIME_PERMISSIONS

appops set com.linuxhub.myth.husk MANAGE_EXTERNAL_STORAGE allow
  → SecurityException: uid 2000 does not have android.permission.MANAGE_APP_OPS_MODES
su -c id   → /system/bin/sh: su: inaccessible or not found
```

所以 `17-gate-a-hw` 的 11 条失败**全部**落在这两件事上，且没有任何合法办法在本仓侧
绕过（本仓禁止把 `pm grant` 冒充成用户点按，也禁止改宿主源码）。

**要真跑通相机/麦克风/定位，只有两条路**，都要用户决定：

- 在宿主 App 里加这三条 `uses-permission` 并在 UI 上弹系统授权框（改的是另一个仓库）；
- 或者用 Husk 自己的 APK（本仓 `app/` 已经声明了 `CAMERA`/`RECORD_AUDIO`/`ACCESS_COARSE_LOCATION`）
  做测试宿主——那就不是“黑盒宿主”了，需要用户确认换宿主。

### 4.2 宿主桥 socket 不存在

`rootfs/run/husk/` 下只有 `ipc/` 和 `mounts`，没有 `audio-in.sock` / `audio-out.sock` /
`camera.sock` / `gps.sock`。`22-audio-audit` 的 3 条失败、`17` 里的麦克风/定位 2 条，
失败原因都是 `connect /run/husk/…sock: No such file or directory`。

### 4.3 传感器

`/sys/subsystem/iio/devices/iio:device4/name` 不存在（这台机器没有 proximity 节点），
叠加 §4.1 的权限缺失，`sensor audit` 报 `errors=18`。

## 5 · GPU soak（4 路 × 300 s × 2400×1080，装后重跑）

见 §6 的实际输出；本轮四条全部 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`、
`errors=0`、fd 稳定、RSS Δ 在限内。

## 6 · H57 在活桌面上的复核（截图见同目录 366-*.png）

- `366-appgrid-thumbs-gpu-desktop-20261127.png`：应用盘（点左下角“显示应用”），
  顶部两块工作区缩略图**完整**。
- `366-appgrid-thumbs-zoom-20261127.png`：缩略图放大 2×。
- `366-overview-workspaces-20261127.png`：Super 打开的 Overview。

像素计数（阈值 R,G,B 全部 < 8 记近黑）：

```
整幅 2400x1080 近黑        0 / 288000   (0.0000)
左缩略图 346x162           0 / 56052    (0.0000)
右缩略图 346x162           0 / 56052    (0.0000)
点右缩略图缩放后整幅        0 / 648000   (0.0000)
```

对比修复前同一块区域：近黑 **0.1490**（evidence 365 §3）。楔子不再出现。

注意：画面上确实有一块**深色斜角**，那是 Ubuntu 壁纸自己的渐变（100% 缩放的桌面上
同样存在，`app2.png` 可见），不是 0 像素的楔子；判据用的是“近黑像素计数”，不是“看起来暗”。

## 7 · 本轮修的三处门禁自身缺陷（假失败）

1. `20-tree-integrity.sh` 的 DING 判据只认单行 `catch (e) { continue; }`，而这棵树里
   的补丁是跨行 try/catch —— 已修好的树被判成未修。现在与安装器同一判据：
   `load_bytes()` 是否落在 try 块内。
2. `17-gate-a-hw.sh` 只认 `ROOTFS_REL`，且相机那行把 `rootfs-full` 写死；用
   `ROOTFS=` 调用时传感器/麦克风/定位三条会去 `files/rootfs-full` 找树，报
   “No such file or directory”——看起来像 Husk 坏了。现在 `ROOTFS_REL` 接受 `ROOTFS`。
3. `15-gpu-global-audit.sh` 的 glxinfo 段继承调用者的 `DISPLAY`，用 `run-guest.sh`
   直接跑就 `unable to open display`，把健康会话判成 2 条 FAIL。现在从
   `gnome-shell` 的 `/proc/<pid>/environ` 里取 `DISPLAY`/`XDG_RUNTIME_DIR`。

三条都在缺陷台账里记为 H58。

## 8 · 仍未验证（照写）

- **Chrome 页面能否真正加载**：NOT RUN（本轮没起浏览器会话）。
- **`KBASE_IOCTL_MEM_EXEC_INIT` 的 EPERM**：仍是 UNKNOWN。
- **完整 Ubuntu 24.04 基础包（4.3 GB）干净装**：设备上没有这份包，NOT RUN；上一轮
  只证明了安装器在空目录上不再因 LIBS 崩塌（evidence 358 §8）。
- **桌面交互帧率**（用户关心的“卡”）：仍是既有的 32 fps 量级，修复排在 0.0.38；
  本轮没有重新量化，只复核了“不黑、不出楔子、能开能缩放”。

## 9 · 按用户要求补做：给测试宿主加权限（H59）

用户口径（2026-11-27 23:4x）：“你直接改这个源码呀……这个 Husk 后缀的是你的，关另一个仓库什么事”。
于是把宿主侧缺的那一半补上——**只动 debug 变体，发布版不受影响**：

新增 `LinuxHub-2.0/app/src/debug/AndroidManifest.xml`（只作用于 `.husk` 后缀的测试构建），
声明 `CAMERA` / `RECORD_AUDIO` / `ACCESS_COARSE_LOCATION` / `ACCESS_FINE_LOCATION`。
`app/src/main/AndroidManifest.xml`（发布版 `com.linuxhub.myth` 用的那份）**一行没动**。

构建与安装：

```
sh gradlew :app:assembleDebug            → BUILD SUCCESSFUL in 48s，app-debug.apk 1.0 GB
aapt2 dump permissions app-debug.apk     → CAMERA / RECORD_AUDIO / ACCESS_COARSE+FINE 都在
apksigner verify --print-certs           → CN=Android Debug，与设备上已装的同一把调试证书
adb install -r -g app-debug.apk          → Success（4.2 GB 树与 identity 原样保留）
```

装完的设备状态（FACT）：

```
CAMERA: granted=true
RECORD_AUDIO: granted=true
ACCESS_COARSE_LOCATION: granted=true
ACCESS_FINE_LOCATION: granted=true
rootfs/etc/husk/runtime-identity：loader 9a220942… / driver 5e063247…（未变）
23-runtime-identity.sh → RUNTIME_IDENTITY PASS=11 FAIL=0
```

`17-gate-a-hw.sh` 从 `PASS=4 FAIL=11` 变成 **`PASS=7 FAIL=8`**：六条权限断言全过
（`granted=true` × 3、`is not USER_FIXED` × 3）。

### 9.1 剩下的 8 条：不是权限，是**宿主根本没有这四座桥**

对着 `LinuxHub-2.0` 源码逐项查过（FACT）：

```
grep -rn "CameraManager|AudioRecord|AAudio|GnssStatus|LocationManager" app rust runtime  → 0 命中
grep -rn "LocalServerSocket|LocalSocket|AF_UNIX|SOCK_STREAM" app/src                    → 0 命中
grep -rn "audio-out.sock|camera.sock|gps.sock|audio-in.sock" .                          → 0 命中
```

`rootfs/run/husk/` 起完桌面后仍然只有 `ipc/` 和 `mounts`，`audio-in.sock` /
`audio-out.sock` / `camera.sock` / `gps.sock` 一个都不出现。

所以剩下 8 条的真实原因是：

| 断言 | 失败输出 | 真实原因 |
|---|---|---|
| app never reported granted | — | 宿主没有读取权限状态的代码 |
| capture failed / no Camera2 session / capability matrix | 空输出 | 没有 camera 桥，`camera-client` 连不上任何 socket |
| audio capture device not present | — | 没有 audio 桥 |
| microphone delivers PCM | `connect /run/husk/audio-in.sock: No such file` | 同上 |
| coarse location delivers TPV | `connect /run/husk/gps.sock: No such file` | 没有 GNSS 桥 |
| five default sensors | `missing /sys/subsystem/iio/devices/iio:device4/name` | 这台机器没有 proximity 节点（与权限无关） |

结论：**相机/麦克风/定位在 0.0.37 内不可验证**，缺的不是授权，是宿主侧四座桥的实现
（约等于 0.0.38 的工作量）。已经能证明的是：权限这一层现在是真的通的，网关的
`camera-fixture`（guest 侧 V4L2 + GStreamer，640×480/1280×720）也是真的通的。

## 10 · 桌面输入（会话内 xdotool）

会话用 `adb input tap` 点“打开桌面”起来（这条一直是外部约束：事件不进宿主窗口，
见 evidence 359），会话内的指针/键盘走 guest 的 `xdotool`：

- `xdotool key super` → Overview 打开（截图 `366-overview-workspaces-20261127.png`）
- `xdotool mousemove 29 1026 click 1` → 应用盘打开（截图 `366-appgrid-*`）
- `xdotool mousemove 2340 876 click --repeat 2 --delay 120 1` → 双击桌面“回收站”图标，
  Nautilus（文件）窗口打开并显示“回收站为空”（截图 `366-input-doubleclick-files-20261127.png`）
- `xdotool getactivewindow` → 8 s 内返回窗口 id `18874372`，会话进程未受影响

**UNKNOWN**：本轮有两次把 `xdotool search --onlyvisible --name .` / `husk-window-wait --list`
用 `| head -5` 接走，命令挂住超过 90 s；其中一次之后 guest 会话已不在（app 进程还在）。
用有界的 `getactivewindow`（后台 8 s 后 kill）复测没有把会话打掉，所以“查询类请求会拖垮
会话”这个假设**不成立**；那次会话消失的原因没有抓到证据，记为 UNKNOWN，不写成结论。

## 11 · 干净装 + 卸载（用本轮 dist，不是旧产物）

用设备上现成的 stock `ubuntu-base-arm64.tar.gz`（29,865,086 B）装到全新目录：

```
run-as <pkg> sh files/linux/husk-stage/install-husk.sh \
  --rootfs /data/local/tmp/ubuntu-base-arm64.tar.gz \
  --dest files/linux/distributions/clean-0037 \
  --loader files/linux/husk-stage/libld.so
→ EXIT=0，== done ==，143 MB
   mode=install  loader 9a220942…  libc 16d5ba21…  gpu_env ad230fcc…  driver 5e063247…  icd 25f6ccd9…
```

在**这棵新树**上跑门禁：

```
20-tree-integrity.sh (EXPECT=base)   PASS=10 FAIL=4
14-gate-a-acceptance.sh              PASS=31 FAIL=2 SKIP=2
```

6 条失败/跳过逐一核对，全部是这棵 29 MB 精简基础包**没有装** `shared-mime-info` /
`gdk-pixbuf` loaders / `locales` + `zh_CN` langpack / `acl` / apt index：

| 断言 | 输出 |
|---|---|
| mime.cache | `missing`（无 shared-mime-info） |
| gdk-pixbuf loaders.cache | `missing or empty` |
| locale-archive | `zh_CN.UTF-8 does not exist`（无 locales 包） |
| etc/default/locale | `LANG=C.UTF-8`（同上） |
| gate A zh_CN charmap / weekday | `ANSI_X3.4-1968` / `Monday`（同上） |
| getfacl / apt policy | SKIP（无 acl 包 / 无 lists） |

即安装器在空目录上把运行时层装对了（身份六项、驱动、ICD、工具、symlink 全在），
缺的是发行版自己该装的包——与 H45 无关。

卸载：`rm -rf files/linux/distributions/clean-0037` 后目录不存在，主树
`ubuntu-24.04/rootfs`（4.2 GB）与 identity 未受影响。
