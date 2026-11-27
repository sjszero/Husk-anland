# 342 · A-hw 息屏假失败与 X11 宿主边界

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：A-hw 第一次运行时设备已处于 `LcdOff`，系统冻结 App；相机 socket 被冻结，麦克风只有静音 PCM，得到 `PASS=11 FAIL=3`。
- **FACT**：唤醒屏幕并重启 App 后，A-hw 得到 `PASS=14 FAIL=0`：3 帧 Camera2、Camera2 session、麦克风 `200704` 字节且 `peak=231`、定位和 crash 检查全部通过。
- **FACT**：修复测试脚本的唤醒步骤后，再先把设备置为息屏，A-hw 仍自动唤醒并得到 `PASS=14 FAIL=0`。
- **FACT**：V4L2/GStreamer 相机门禁通过：640×480 和 1280×720 的 raw、mmap、10 帧流和 `v4l2src` 3 帧都通过。
- **FACT**：GPU 全量读回 `PASS=19 FAIL=0`，loader 默认值 `PASS=9 FAIL=0 SKIP=0`，渲染器为 zink/PanVK。
- **FACT**：`make gpu-audit` 的宿主 X11 部分失败为 `glxinfo: unable to open display :0`，gnome-shell 不存在；因此这是宿主 X server 缺失，不是 PanVK/读回回归。
- **UNVERIFIED**：宿主 App 提供 X server、GNOME shell、多 GUI 并发和 Surface 呈现的完整链路，当前测试壳没有该宿主。

## 1 · A-hw 修前负向结果

第一次运行：

```text
PASS=11 FAIL=3
FAIL capture failed:
FAIL no Camera2 session was opened
FAIL microphone bridge returned no non-silent PCM (bytes=200704 peak=0)
```

现场日志显示：

```text
OplusHansManager: freeze uid: 10303 com.linuxhub.husk ... scene: LcdOff
```

同时 `run/husk/camera.sock` 不存在，证明是 App 被冻结，不是 Camera2 权限或 V4L2 路径失效。

## 2 · A-hw 修复后正向结果

唤醒、重启 App 后：

```text
PASS=14 FAIL=0
captured 3 frame(s) through Camera2 (1382400 bytes each)
app opened a Camera2 session
microphone returned 200704 PCM bytes (peak=231)
gpsd delivered a last-known fix
no crash records for the app process
```

测试脚本的修复：

- 默认 loader 改为 `files/linux/husk/libld.so`，不再指向旧的 `files/linux/libld.so`；
- 在读取权限和硬件桥前执行 `KEYCODE_WAKEUP` + `dismiss-keyguard`。

负向起点复验：先执行 `KEYCODE_SLEEP`，再运行 A-hw，结果仍为 `PASS=14 FAIL=0`。

## 3 · 相机与 GPU 回归

相机门禁：

```text
v4l2test|querycap|driver=husk|card=FIXTURE
v4l2test|done|frames=10|bytes=4608000|fps=93.13
v4l2test|done|frames=10|bytes=13824000|fps=39.99
GST_AUDIT|PASS|640x480|frames=3
GST_AUDIT|PASS|1280x720|frames=3
camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

GPU 读回：

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS=19 FAIL=0
READBACK_AUDIT PASS=6 FAIL=0
```

GPU loader 默认值：

```text
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1: llvmpipe
HUSK_GPU=llvmpipe: llvmpipe
```

## 4 · X11 宿主审计失败

`make gpu-audit` 的读回部分通过，随后：

```text
== existing desktop shell ==
FAIL gnome-shell process not readable (pid=missing)
== loader-default GL client ==
Error: unable to open display :0
FAIL nested-gtk process not readable
GPU_GLOBAL_AUDIT PASS=0 FAIL=3 SKIP=0
```

判断：

- **FACT**：`DISPLAY=:0` 没有可用的宿主 X server；不是 `glxinfo` 看到了软件渲染。
- **UNVERIFIED**：宿主 X server、gnome-shell、多 GUI 进程并发和 Surface 呈现链路。
- 不能把该失败改写成“GPU 通过”；也不能把 X server 塞回 Husk 发行资产来绕过宿主边界。

## 5 · 当前发布状态

本轮只证明运行时、相机、麦克风、定位和 offscreen/loader GPU 路径；不证明宿主 X11 桌面可用。版本仍为 `0.0.36`，没有发版。

## 6 · GPU 长稳测试变量冲突

修前命令：

```text
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk make gpu-soak
```

立即失败：

```text
FAIL  SECONDS must be >= 1
```

原因：`SECONDS` 是 POSIX shell 的自动计时变量，Make 环境里的值被 shell 自己的 `SECONDS=0` 覆盖。改为 `SOAK_SECONDS` 后：

```text
== 2 jobs for 2s at 2400x1080 ==
GPU_SOAK_AUDIT PASS=11 FAIL=0 SKIP=0
```

正式回归：

```text
== 4 jobs for 300s at 2400x1080 ==
p1..p4 renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
all jobs: errors=0, fd=7 stable, RSS delta=272kB
GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0
```

这是测试脚本缺陷，不是 GPU 掉速或驱动崩溃。
