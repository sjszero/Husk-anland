# Evidence 367 — Husk camera → PipeWire → Snapshot（2026-11-28，进行中）

本文件只记录当前 `.myth.husk` 真机现场；未通过的项目明确写 UNVERIFIED。

## 1. 媒体服务配置权限导致桌面 helper 退出（已定位，源码已修）

**FACT**：桌面日志反复出现：

```text
stage=media-degraded reason=helper_start_failed
husk-media-session: missing /etc/pipewire/pipewire.conf
```

**FACT**：设备上的两个配置文件实际存在，但由 Android `AssetManager` 复制后是 `0600`：

```text
-rw------- root root /etc/pipewire/pipewire.conf
-rw------- root root /etc/wireplumber/wireplumber.conf
pipewire_readable=1
wireplumber_readable=1
```

**INFERENCE**：Husk 的 guest 侧把文件呈现为 root 所有；桌面身份是 `husk(1000)`，因此 shell 的 `test -r` 返回失败。手工把文件规范为 `0644` 后，同一 helper 能启动 PipeWire。

**FACT**：源码已在 `LinuxContainerManager.ensureGnomeSessionAssets()` 中显式设置：

```text
pipewire.conf          0644
wireplumber.conf       0644
husk-media-session     0755
libhusk_v4l2.so        0755
```

**FACT**：重新构建、安装 APK 并进入桌面后：

```text
stage=media-ready pipewire=31046 wireplumber=31059
husk-media-session: pipewire=ready
husk-media-session: wireplumber=ready
```

**FACT**：会话中存在 PipeWire client：

```text
gnome-shell              pid:30953
xdg-desktop-portal       pid:31429
snapshot                 pid:31624
WirePlumber              pid:31059
```

## 2. Video/Source 仍为空，已缩到 SPA V4L2 open/fstat 边界

**FACT**：`wpctl status` 的 Video 段为空；`pw-dump` 只有 Dummy-Driver / Freewheel-Driver。

**FACT**：直接在真实会话环境运行 `spa-inspect`，SPA 自己报：

```text
[I] spa.v4l2 ... device is '/dev/video0'
[E] spa.v4l2 ... /dev/video0 is no device
factory name: 'api.v4l2.source'
media.class = "Video/Source"
media.role = "Camera"
```

**FACT**：同一 LD_PRELOAD 环境下 `stat /dev/video0` 显示 character device `81,0`，shell `exec 3<>/dev/video0` 返回 0。因此 V4L2 的 open 路径本身可用。

**FACT**：PipeWire 1.0.5 的 `spa_v4l2_open()` 先 `open()`，随后 `fstat()` 并检查 `S_ISCHR`。当前 `runtime/crates/v4l2-preload` 只导出路径型 `stat/statx`，没有导出 `fstat/fstat64`；打开的相机 fd 实际是 memfd，所以 `fstat` 看到普通文件，SPA 打印 “is no device”。

**当前结论**：这是 V4L2 facade 与 SPA 消费者之间缺少 fd 身份收敛，不是权限、Camera2 枚举或 WirePlumber 启动失败。

## 3. 两套宿主路径仍未合并

**FACT**：App 的 `LinuxContainerManager.startDesktop()` 仍调用旧的 `ContainerAudioBridge.start()`（PulseAudio FIFO `/run/doro-audio/pulse-output.pcm`），同时 `HuskBridgeController` 启动新的统一 helper，后者绑定 `run/husk/audio-out.sock`、`audio-in.sock`。

**FACT**：桌面脚本仍启动独立 PulseAudio，并只配置 `module-pipe-sink`；没有把 `run/husk/audio-out.sock` 接成唯一音频后端。

**结论**：当前硬盘上的代码不是“已完成融合”的最终形态。必须先收敛为一个 V4L2 状态机和一条音频路径，再用 Snapshot/音频应用做真机回归。

## 4. 尚未验证

- Snapshot 枚举到 Camera 并显示真实画面：**UNVERIFIED**
- `wpctl status` 出现 `Video/Source`：**UNVERIFIED**
- 音频经单一 `run/husk/audio-*.sock` 路径：**UNVERIFIED**
- 音频/相机以外硬件桥的完整回归：**UNVERIFIED**

## 5. 2026-11-28 02:27 真机身份边界（只记录事实）

- 设备 `UKHMMB8PYTGECMNV`，新版 App 包 `com.linuxhub.myth.husk`，主进程 PID `29001`。
- `files/linux/husk/libld.so` SHA-256 `0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94`。
- rootfs 的 `libhusk_v4l2.so` 为 200592 bytes，mtime `2026-11-28 02:07`；两个媒体配置为 `0644`，媒体 helper 为 `0755`。
- 新版 `husk-bridges` 父进程是 PID `29001`；旧 `com.linuxhub.husk` uid `u0_a303` 仍有残留进程，不能混入新版结论。
- 当前新版桌面为 `STOPPED`；只有新版 `pipewire` PID `30047`，没有新版 `wireplumber` 或 `gnome-shell`。
- `wpctl Video/Source`、Snapshot 画面、音频单一路径仍为 **UNVERIFIED**。

## 6. WirePlumber 相机枚举与 socket 第一次握手（2026-11-28，继续中）

**FACT**：在设备 rootfs 手工创建以下三个真实空目录后：

```text
rootfs/sys/class/video4linux
rootfs/sys/bus/video4linux
rootfs/sys/subsystem/video4linux
```

新版权威 `.myth.husk` UID 下的 WirePlumber 首次枚举出两个 V4L2 设备：

```text
39. BACK  [v4l2]
40. FRONT [v4l2]
```

`pw-dump` 同时出现两个 `Video/Device`，路径为 `/dev/video0`、`/dev/video1`。

**FACT**：`Video/Sources` 仍为空，尚未出现可消费的 `Video/Source`；Snapshot 真实画面仍为 **UNVERIFIED**。

**FACT**：WirePlumber debug 日志已经推进到：

```text
spa.v4l2 check_access: /dev/video0 accessible:1
pw.context load factory api.v4l2.device
spa.v4l2 spa_v4l2_open: device is '/dev/video0'
```

之后节点激活停住；同时 App camera helper 记录：

```text
camera|client|read: failed to fill whole buffer
camera|client|done
```

**INFERENCE**：客户端已经连上 `camera.sock`，但在完成首条命令前关闭或协议数据未完整到达。下一步必须同时对账 `v4l2-shim` 的 `connect/send/recv_line/open_session`、preload 的 ABI 转发和 App 的 `handle_client`，排除立即关闭、`INFO` 握手竞争和多实例抢 socket。

**源码回归项**：`prepare_v4l2_tree()` 当前只生成影子树 `sys-v4l2/*`。上述三个真实目录必须写回源码并配测试，手工状态不计最终完成。

## 7. camera.sock 双向等待的内核现场（2026-11-28）

**FACT**：同一时刻的系统调用现场如下：

```text
App camera TID 4849: syscall 207 recvfrom, wchan unix_stream_read_generic
WirePlumber TID 6557: syscall 63 read, fd 30, wchan unix_stream_read_generic
```

`/proc/net/unix` 中对应：

```text
listener inode 38308194
App accepted peer inode 38381673
WirePlumber client inode 38367720
```

三者均属于同一个 camera.sock 连接。

**INFERENCE**：连接本身成立，双方确实处于双向等待；不是 socket 路径、权限或 accept 失败。下一步给 `query_info` 增加发送/接收审计点，确认服务端是否已经处理 `INFO`、客户端是否收到完整终止行。

## 8. camera.sock 审计点与 preload 构建（2026-11-28）

**FACT**：源码加入仅由 `NR_V4L2_AUDIT=1` 启用的首轮握手 trace：

```text
camera.sock connected; sending INFO
INFO sent; waiting for reply
INFO reply: <line>
```

App 侧 `INFO` 分支加入 `camera|client|INFO`、`INFO|ok`、`INFO|write_fail` 日志；写失败不再静默吞掉。

**FACT**：`make preloads NDK=...` 成功，`audit-preload-exports.sh` PASS；新 `libhusk_v4l2.so` 已构建。App 日志改动尚未进入 APK，当前诊断只替换新版 rootfs 的 preload。

## 9. 根因确认：camera.sock 串行 accept 被旧连接永久堵死（2026-11-28）

**FACT**：新 preload trace：

```text
[husk v4l2] camera.sock connected; sending INFO
[husk v4l2] INFO sent; waiting for reply
```

WirePlumber 当前 camera 客户端 fd 30 的 inode 为 `38416164`。

**FACT**：App camera 线程 `4849` 的 fd 20 仍指向旧 accepted inode `38381673`；旧对端 inode `38367720` 已不在 `/proc/net/unix`，但该线程仍阻塞在旧 stream 的 `read_command()`。当前 WirePlumber 连接尚未被 accept。

**结论**：根因是服务端单客户端串行 accept，且首轮读没有超时。一个死连接就能让所有后续 `/dev/video*` 客户端永久等待。修复方向：

- 每个 accepted client 有界并发处理；
- 首轮命令短读超时，超时回收；
- 首条合法命令后取消超时，允许 WirePlumber 持久空闲；
- 客户端握手也加超时，失败向 SPA 返回错误而不是挂死。

## 10. camera.sock 并发修复、客户端超时与 V4L2 树单测（2026-11-28）

**FACT**：修复落在源码：

- 每客户端有界并发，最多 16 个 accepted client；
- 首轮握手 3 秒超时；
- 合法 `INFO`/`OPEN` 后取消读超时，允许 WirePlumber 持久空闲；
- 单 capture session 互斥，避免共享回调状态互相踩；
- 客户端 socket 收发 5 秒超时。

**FACT**：V4L2 sysfs facade 抽到 `camera-v4l2-tree` crate，并创建真实根目录：

```text
sys/class/video4linux
sys/bus/video4linux
sys/subsystem/video4linux
```

宿主单测结果：

```text
2 passed; 0 failed
```

覆盖完整生成与“相机撤销后清理旧影子节点、保留真实根目录”的负向回归。

**FACT**：Android 目标 `cargo check`、`make preloads`、`cargo build --release --bin husk-bridges` 均通过。

新产物：

```text
husk-bridges       c0663333698fc7e55b961a29fbf47c01191bc75e4726b98ae63276080b5c2d75
libhusk_v4l2.so    ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f
```

**边界**：尚未装入 `.myth.husk` APK，`Video/Source` 和 Snapshot 画面仍为 **UNVERIFIED**。

## 11. APK 资产打包自证（2026-11-28）

`assembleDebug` 成功。APK：

```text
path   LinuxHub-2.0/app/build/outputs/apk/debug/app-debug.apk
sha256 b6ce34a0c565d59052042f953961ae5e6e27a90138180e3988defa56400a9fec
bytes  1049110926
```

从 APK 内直接复核：

```text
husk-bridges    c0663333698fc7e55b961a29fbf47c01191bc75e4726b98ae63276080b5c2d75
libhusk_v4l2.so ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f
libld.so        0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94
```

尚未覆盖安装和运行验证。

## 12. 新桌面、运行身份与三个并发 INFO 客户端（2026-11-28）

**FACT**：从 App 入口启动后的截图 `/tmp/husk-desktop-ready.png` 显示完整 GNOME 桌面：顶栏、Dock、桌面图标、壁纸均存在，桌面可输入。

**FACT**：运行身份：

```text
package        com.linuxhub.myth.husk
App PID        30645
UID            10265
husk-bridges   12008, PPID 30645
gnome-shell    12192, PPID 30645
rootfs preload ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f
XDG_RUNTIME_DIR /tmp/runtime-husk
```

**FACT**：logcat 连续出现：

```text
camera|client|INFO
camera|client|INFO|ok
camera|client|done|handshake=ok
```

三个客户端均完成首轮握手，证明新的每客户端并发处理已真正运行，而不是只通过源码或编译检查。

**FACT**：当前 HUSK 会话内 `wpctl status`：

```text
PipeWire 1.0.5, cookie:2103683262
32/33 WirePlumber pid:12320
38 gnome-shell pid:12192
39 xdg-desktop-portal pid:12731
Video/Devices: 37. BACK [v4l2]
Video/Sources: <empty>
```

**FACT**：PID `30047` 是旧 `XDG_RUNTIME_DIR=/tmp/runtime-doro` 的 PipeWire，与当前 `runtime-husk` 不是同一实例；不计入 HUSK 当前媒体栈，验证前清理。

**边界**：`Video/Source` 与 GNOME Snapshot 真实画面仍为 **UNVERIFIED**。下一步必须以 `NR_V4L2_AUDIT=1` 重启唯一当前 WirePlumber，定位 `/dev/video0 is no device` 发生在哪一层。

## 13. 根因收敛：PipeWire 进程没有加载 V4L2 preload（2026-11-28）

**FACT**：`NR_V4L2_AUDIT=1` 下，WirePlumber 对前后两摄的 `INFO` 均完整成功，返回 16 组尺寸并以 `OK` 结束；App 侧对应记录 `camera|client|INFO|ok`。

**FACT**：同一 rootfs 与 preload 下，`v4l2-test /dev/video0 0 NV12 ...` 返回 `OK`，实际通过：

```text
stat chr=81:0
QUERYCAP driver=husk card=BACK caps=0x5000001
ENUM_FMT / ENUM_FRAMESIZES
S_FMT 640x480 NV12
REQBUFS=4 / mmap / STREAMON / STREAMOFF
```

**FACT**：直接运行 `spa-inspect .../spa-0.2/v4l2/libspa-v4l2.so` 退出码 `0`，SPA 枚举出 `media.class=Video/Source`、`media.role=Camera` 和 `video/raw` 格式，不再报 `is no device`。

**FACT**：当前 WirePlumber PID `13332` 的 environ 含：

```text
LD_PRELOAD=<rootfs>/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so
```

同一会话 PipeWire PID `13311` 的 environ 没有 `LD_PRELOAD`；PipeWire 日志仍出现：

```text
[E] spa.v4l2 ... spa_v4l2_open()] /dev/video0 is no device
```

**结论**：`libspa-v4l2.so` 由 PipeWire 在运行时 dlopen，Loader 的启动期 GOT shim 无法覆盖；只给 WirePlumber 注入 preload 不够。PipeWire 守护进程与 WirePlumber 必须注入同一个 V4L2 preload。

**下一步**：把 `LD_PRELOAD="$V4L2_PRELOAD"` 同时用于 PipeWire 启动，重部署唯一媒体栈后重新验证 `Video/Source` 和 Snapshot 真实画面。

## 14. 双侧 preload 修复后 Video/Source 真机出现（2026-11-28）

**修复**：`husk-media-session.sh` 不再只给 WirePlumber 注入 preload；PipeWire 守护进程也使用同一个：

```sh
LD_PRELOAD="$V4L2_PRELOAD" \
pipewire -c "$CONFIG" >"$PIPE_LOG" 2>&1 &
```

原因：`libspa-v4l2.so` 由两个 daemon 在运行时 dlopen，Loader 的启动期 GOT shim 覆盖不到；少一个进程就会让 PipeWire 侧重新打开真实 guest `/dev/video0` 并报 `is no device`。

**FACT**：新脚本部署到 rootfs 后 SHA-256：

```text
93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5
```

**FACT**：当前会话重启后：

```text
PipeWire      pid 13891 ready
WirePlumber   pid 13903 ready
```

**FACT**：`wpctl status` 首次出现真实 video sources：

```text
Video
 ├─ Devices:
 │      38. BACK                                [v4l2]
 │      39. FRONT                               [v4l2]
 └─ Sources:
        41. BACK (V4L2)
        43. FRONT (V4L2)
```

**判定**：`Video/Source` 已从 UNVERIFIED 升级为设备实测 PASS。下一步必须启动 GNOME Snapshot 并截到真实取景画面；相机应用与拍照/录像仍为 **UNVERIFIED**。

## 15. Snapshot active link 但 DQBUF EAGAIN：session ready 顺序错误（2026-11-28）

**FACT**：Snapshot 窗口已显示相机控件，PipeWire 状态为：

```text
org.gnome.Snapshot
  input_1 < BACK:capture_1 [active]
```

但窗口预览仍为黑屏。

**FACT**：关闭 Snapshot 后在同一媒体栈上运行 raw 3 帧：

```text
v4l2test|streamon
v4l2test|FAIL|DQBUF 0: Resource temporarily unavailable (os error 11)
```

`STREAMON` 成功，`DQBUF` 没有帧。

**FACT**：App 日志中只有：

```text
camera|session|open|id=0|1920x1080|repeat_rc=0
```

没有任何 `camera|callback|session_ready`。这违反了 Camera2 NDK 的生命周期：必须先等 `on_ready`，再提交 repeating request。提前提交可能返回 0，但不会产出帧。

**修复**：`CameraSession::open()` 现在：

1. 创建 session 前清 `SESSION_READY`；
2. 用当前 client thread 的 Looper 泵最多 3 秒，等待 `on_ready`；
3. 未 Ready 时明确失败并关闭 session/output/device/reader；
4. Ready 后才创建并提交 repeating request；
5. `try_frame()` 首次遇到取帧 status 变化时记录 `AImageReader_acquireNextImage` 返回码，恢复后记录 recovered，成功首帧及每 30 帧记录一次。

**静态验证**：Android target `cargo check` 通过。尚需构建新 `husk-bridges` 并在真机重跑 raw 与 Snapshot；当前结果仍为 **UNVERIFIED**。

## 16. 纠正错误假设：本机不能把 onReady 当成功门禁（2026-11-28）

**FACT**：回查 `docs/design/evidence/89-camera-ndk-capture.md`，本机 Android 13 / MT6893 上
`ACameraCaptureSession_stateCallbacks.onReady` 始终没有投递；旧版成功路径是创建 session 后
直接调用 `setRepeatingRequest`，以返回码和真实帧结果判定，而不是等待 `onReady`。

**FACT**：当前 `camera.rs` 第 15 节加入的 3 秒 `onReady` 硬等待与本机历史行为冲突；真机日志
已经出现 `camera|session|ready|false` 和 `camera|client|open_fail|capture session did not become ready`。

**判定**：第 15 节中“必须等 onReady 才允许提交”的推断只符合通用 NDK 文档，不适用于本 ROM 实测 ABI。
下一步恢复“直接提交、记录 ready_observed、靠取帧超时和返回码判定”的策略；不得永久阻塞或把
`onReady=false` 单独判为相机失效。

### 16.1 已核对运行现场

```text
husk-bridges     PID 15066，运行目录 SHA 41652ffa0145da3470b5f53740ed988240f6c36daeb36f60de2c311b6e048b50
App              PID 30645
PipeWire         PID 15140
WirePlumber      PID 15150
```

### 16.2 onReady 硬等待移除

- 修改：删除 `CAMERA_SESSION_READY_TIMEOUT=3s`；创建 session 后只做一次非阻塞 Looper poll，记录 `camera|session|ready_observed|true/false`，随后直接 `setRepeatingRequest`。
- 不把 `ready_observed=false` 判成失败；真实成功条件仍是 `repeat_rc=0` 加可取帧。
- 操作记录：首次 `cargo fmt` 因从 LinuxHub 工作区传入 `app-rs/src/camera.rs` 相对路径而失败（文件不在该工作区）；随后从 Husk 仓库根目录重跑。
- 验证：Husk 根目录 `cargo fmt` 成功。
- 验证：Android 目标 `cargo check --manifest-path app-rs/Cargo.toml --target aarch64-linux-android --lib` 成功（EXIT=0）；仍有 20 条既有/非阻断 warning，其中 camera 有 unused unsafe、dead code 等，未发现本次改动的类型/借用错误。
- 构建：Android release `husk-bridges` 成功（37.34s）。
- 产物：`app-rs/target/aarch64-linux-android/release/husk-bridges`，SHA-256 `983ac6396112967e6b2de48aa87164dfda0ff9589e18939193a42c1bbbe1a86f`。
- 操作偏差：第一次在 Husk 根目录查询 `target/...` 路径失败；实际 Cargo target 位于 `app-rs/target/...`，重新查找后已取得 SHA，不影响产物。

### 16.3 新 helper 部署到 `.myth.husk` 调试运行目录

- 操作：停止旧 `husk-bridges` PID 15066，原子替换 `files/linux/runtime/husk-bridges`，以原环境变量启动。
- FACT：设备运行文件 SHA-256 为 `983ac6396112967e6b2de48aa87164dfda0ff9589e18939193a42c1bbbe1a86f`，与宿主构建产物一致。
- FACT：新 PID 15490，UID `u0_a265`，`HUSK_PARENT_PID=30645`；父进程仍为 1，说明这是 `run-as` 手工调试启动，不是 App 正式管理身份，发布前必须重建 APK 复验。
- FACT：使用当前会话环境执行 `husk-media-session status` 成功，显示 PipeWire PID 15140、WirePlumber PID 15150 均 ready。
- 操作偏差：媒体栈重启命令把包含空格的 adb 命令存入 zsh 标量后直接执行，zsh 报 `command not found: adb -s UKHMMB8PYTGECMNV` 两次；媒体栈实际未重启。随后改用完整 adb 命令重试。

### 16.4 首次直接提交仍不出帧，根因缩到 session 配置竞态

- 操作：媒体栈 stop/start 成功，新 PipeWire PID 15638、WirePlumber PID 15648 均 ready。
- 验证：`v4l2-test /dev/video0 3 NV12 /tmp/husk-raw 640 480` 完成 INFO、QUERYCAP、格式/尺寸、REQBUFS、mmap、STREAMON，但第一次 DQBUF 返回 `EAGAIN`，不是 PASS。
- FACT：App 日志为 `camera|session|ready_observed|false`、`repeat_rc=0`，随后 `AImageReader_acquireNextImage rc=-30001`（NO_BUFFER_AVAILABLE）。
- FACT：系统日志显示 Camera HAL 在 `02:58:04.565` 才完成 `configureStreams`，而 helper 在 `02:58:04.546` 已下发 repeating request；本次提交早于流配置完成。
- 判定：不能把 `setRepeatingRequest rc=0` 当成功。下一步仍不硬等 onReady，但给它一个有界的配置窗口（300ms），窗口结束后无条件继续提交，避免本 ROM onReady 永不投递时永久失败。
- 修改：新增 `CAMERA_SESSION_CONFIG_GRACE=300ms`；创建 session 后最多泵 Looper 300ms 提前接收 onReady，但到时不投递也继续创建 request 并提交，不再 fail closed。`cargo fmt` 成功。
- 验证：带 300ms 有界配置窗口的 Android `cargo check` 成功（EXIT=0），仍为同样 20 条非阻断 warning。
- 构建：300ms 有界窗口版本 release 成功；产物 SHA-256 `fbd3bdde37023d440d153974f81e393dff928f23e865fb388025327872943f35`。

### 16.5 one-shot 主线程自测同样无帧，发现旧包进程残留

- FACT：在不经过 socket 的 helper one-shot 主线程自测中，`frames=0`、`elapsed_ms=12125`，8 组 `camera|selftest|no_frame`；说明不是客户端线程或 Looper 绑定差异。
- FACT：`ps` 发现旧包 `com.linuxhub.husk` 仍有 UID `u0_a303` 的大量进程（含旧 loader/桌面/logd 等）。当前 Camera2 active client 虽只显示 `.myth.husk`，但这违反不得混用旧包的硬约束。
- 操作：执行 `am force-stop com.linuxhub.husk` 清掉旧包进程，再复测当前 `.myth.husk` 主线程自测。旧包数据不读取、不修改。
- FACT：`am force-stop com.linuxhub.husk` 后仍留有 PPID 1 的 UID 10303 孤儿进程；该包已卸载，`run-as com.linuxhub.husk` 返回 `unknown package`，非 root shell 无法清掉。它们是否影响相机当前为 UNKNOWN；若后续仍失败，需要重启设备取得干净现场。
- 下一步：仍先做不影响架构的 A/B：在长时间无帧时每 500ms 重发同一个 repeating request，验证 Camera HAL 是否吞掉首次提交。
- 修改：抽出 `submit_repeating`；首帧到达前每 500ms 重发同一 repeating request，并记录 `camera|session|repeat_retry|attempt=N|rc=...`。成功后停止重发。格式化通过。
- 构建：repeating 重发版本 release 成功，SHA-256 `735cce17b24afc13847a5601452d9a913b9ec0fd2094d16a6dd472087297ee5c`。
- 验证：repeating 重发版本连续尝试 40 次均 `rc=0`，但 `AImageReader_acquireNextImage` 始终 `-30001`，raw DQBUF 仍 EAGAIN；重发不是修复。
- 判定：当前失败已排除首次提交时机。旧包卸载后的 UID 10303 孤儿进程与 HAL 脏状态无法通过非 root 清理，下一步重启设备取得干净运行现场。

### 16.6 设备重启后的干净 A/B 与 PREVIEW 模板测试

- FACT：设备重启后旧 UID 10303 孤儿进程已消失；Husk App PID 6025，helper PID 9589 为 App 子进程，运行文件仍是 APK 内旧 helper `c0663333...`。
- FACT：干净现场下旧 helper 的 raw 1280x720 仍是 DQBUF EAGAIN，排除卸载残留。
- FACT：系统相机 `com.oplus.camera` 打开 Camera 0 后 dumpsys 显示 Request ID counter 5，且 `Latest received frame` 元数据已分配；硬件采集链路本身可用，问题位于 Husk/Camera2 NDK 请求路径。
- 单变量 A/B：把请求模板从 `ACAMERA_TEMPLATE_RECORD` 改为 `ACAMERA_TEMPLATE_PREVIEW`，其余不变，验证 MTK HAL 是否拒绝 RECORD 模板的 YUV ImageReader 流。
- 验证：PREVIEW 模板仍是 DQBUF EAGAIN，40 次 repeating 重发均 rc=0，排除 capture intent 模板。已恢复 RECORD，并给 ImageReader/session output/request/target 全链路 return code 与 null 状态加实证日志。

### 16.7 全链路 setup rc=0 但 reader 始终无 buffer（2026-11-28）

- FACT：最新 helper 实测日志为：
  ```text
  camera|session|output_setup|container_rc=0|output_rc=0|add_rc=0
  camera|session|request_setup|request_rc=0|target_rc=0|add_target_rc=0
  camera|session|open|id=0|1280x720|repeat_rc=0
  camera|frame|acquire|rc=-30001|image_null=true
  ```
- FACT：`-30001` 对应 `AMEDIA_IMGREADER_NO_BUFFER_AVAILABLE`；setup API 返回码均为 0，不能据此判相机可用。
- FACT：raw `v4l2-test` 的 INFO、QUERYCAP、格式、REQBUFS、mmap、STREAMON 全部成功，第一次 DQBUF 返回 `EAGAIN`；前后摄、640x480、1280x720 均失败。
- FACT：已排除 socket 客户端线程问题（one-shot 主线程自测同样零帧）、首次 request 被吞（连续重发 40 次均 `rc=0`）、PREVIEW/RECORD 模板、权限和旧包孤儿进程。
- FACT：系统相机打开 Camera 0 后 `dumpsys media.camera` 显示 request counter 增长、Latest received frame 元数据已分配，说明 Camera/HAL 硬件本身正常。
- 可疑但不未证实：MTK HAL 日志出现 `size have been reduce to 0x0(0x0) by enque` 与 `mpEnqRoleSequence is empty!`；尚不能判定为根因。
- 下一步：增加 capture/device 回调，读取 request sequence、capture failure 与 device error；若回调不到，再对照 `d2bd77b` 历史成功路径做隔离。

### 16.8 capture/image-reader 回调诊断已写入源码（2026-11-28）

- 修改：`camera.rs` 注册 `AImageReader_setImageListener`，记录 `on_image_available`；不再给 `setRepeatingRequest` 传空 capture callbacks。
- 修改：补齐 `onCaptureStarted`、`onCaptureProgressed`、`onCaptureCompleted`、`onCaptureFailed`、`onCaptureSequenceCompleted`、`onCaptureSequenceAborted`、`onCaptureBufferLost`，记录 frame/reason/sequence/image_captured。
- 修改：`setRepeatingRequest` 传入 sequence id 输出指针，首次提交与 retry 都打印 `camera|session|...|sequence=N`；`on_session_active` 也记录。
- 目前是诊断改动，不增加新重试、不把日志出现冒充相机通过。下一步：格式化、Android cargo check、release 构建、真机复测。

### 16.9 回调诊断版静态检查（2026-11-28）

- 验证：Android `cargo check --bin husk-bridges` 实际输出 `Finished dev profile`，编译/类型检查成功；仍为既有的 20 条 warning。
- 操作偏差：命令包装脚本用 zsh 只读变量名 `status` 保存退出码，导致 `zsh: read-only variable: status`，包装脚本自身退出 1；这不等于 Rust 检查失败。将改用非保留变量名重跑并留最终 EXIT。
- 验证：重跑 Android `cargo check --bin husk-bridges`，明确 `CARGO_CHECK_EXIT=0`；20 条 warning 未阻断。
- 下一步：构建 release `husk-bridges`，记录产物 SHA，再替换调试 helper 做真机 capture 回调复测。
- 构建：release `husk-bridges` 成功，`CARGO_BUILD_EXIT=0`；产物 656784 bytes，SHA-256 `d3ffa158afdd2ac75d35e4f72163aee7f7c2519dc52d6c7605e6fc9d46d89cd7`。
- 下一步：原子替换 `.myth.husk` 的运行 helper，创建 3 帧 selftest marker，清空日志后重启并观察 image/capture 回调。

### 16.10 回调实测推翻 repeating 重发实验：重发会 abort 前一条序列（2026-11-28）

- 部署：新 helper SHA `d3ffa158...` 已原子替换 `.myth.husk` 调试运行目录，设备复算一致；调试 PID 13986（PPID 1，仍非正式 App 管理身份）。
- FACT：`AImageReader_setImageListener rc=0`；session output/request/target 仍全部 rc=0。
- FACT：`setRepeatingRequest` 首次返回 `rc=0|sequence=0`；之后每 500ms 重发一次，返回新的 sequence=1/2/3...。
- FACT：没有 `camera|capture|started`、`camera|capture|completed`、`camera|image_reader|available`；`AImageReader_acquireNextImage` 始终 `-30001`。
- FACT：每个 sequence=N 在下一轮重发后立即出现 `camera|capture|sequence_aborted|sequence=N`；3 帧 selftest 最终 `frames=0`，`files/camera-frames` 为 0 bytes。
- 判定：500ms repeating 重发会替换并 abort 上一条重复序列，之前的“连续 40 次 rc=0”实验不成立；这是实验代码污染，不能当作 HAL 吞请求的证据。
- 修复：移除 repeating 重试定时器和字段，只保留一次 `setRepeatingRequest`；保留 capture/image callback 观察，随后用新的 sequence 复测。
- 修改：已删除 repeating retry 定时器、`last_repeat_attempt`、`repeat_attempts` 与 `repeat_retry` 日志；`setRepeatingRequest` 现在只执行一次。
- 验证：Android `cargo check` EXIT=0；release 构建 EXIT=0；新产物 656432 bytes，SHA-256 `59359aaca99b82b671b2ae7f64a5bd54c2e8ef00777195d8fce5f28a7654c8e7`。
- 下一步：部署该单次 repeating 版本并重新跑 3 帧 selftest。

### 16.11 单次 repeating 真机复测仍零帧（2026-11-28）

- 部署：单次 repeating 版本 SHA `59359aac...` 已替换调试 helper，设备 PID 14217，设备复算一致。
- FACT：`camera|reader|listener|rc=0`、setup 全链路 rc=0、`camera|session|open|id=0|1280x720|repeat_rc=0|sequence=0`。
- FACT：`camera|callback|session_active` 到达；但 12 秒内没有 `capture|started`、`capture|completed`、`capture|failed`、`image_reader|available`。
- FACT：`camera|frame|acquire|rc=-30001|image_null=true`；selftest `frames=0`；帧目录无文件。
- FACT：HAL 日志显示 `stream size is 0 x 230771904`，随后 `Bypass roundBufferDimensionNearest ... width 1280 height 720`；MTK 已分配 NV12 pool，但持续出现 `size have been reduce to 0x0` / `mpEnqRoleSequence is empty!`。
- 判定：setup API 成功不等于 request 进入有效 capture 序列；需要 active session 的 CameraService/HAL dump 与前置相机 A/B，不能继续盲改。
- 下一步：跑 camera=1（前置）3 帧；若前后都失败，查公共 request/output 路径；若仅后置失败，查 camera 0/HAL 配置。

### 16.12 前置 camera=1 A/B 同样零帧（2026-11-28）

- FACT：camera=1、1280x720、单次 repeating，`reader listener/setup/repeat_rc` 全为 0，`session_active` 到达。
- FACT：12.113s 内没有任何 capture started/completed/failed 或 image-available；`frames=0`，帧目录无文件。
- 判定：不是 camera 0 后置特例，问题位于两者共用的 Camera2 NDK output/request 路径。
- 下一步：保持 session 活跃，抓 `dumpsys media.camera` 的 request counter、stream/queue 与 HAL 状态，再决定修改 native window geometry、request 或 session 创建顺序。

### 16.13 Active session dump：request 到达 service，但没有 frame（2026-11-28）

- FACT：camera=0 长测 active 时 `dumpsys media.camera` 显示：
  ```text
  Device 0 is open. Client PID: 14608
  Current client UID 10265
  Request ID counter: 1
  Stream[0]: Output
    Consumer name: ImageReader-1280x720f23u3m3-14608-0
  Latest received frame:
    Dumping camera metadata array: Not allocated
  ```
- FACT：`dumpsys SurfaceFlinger --list` 没有对应 ImageReader surface 的可见条目；CameraService 已接受 request，但没有任何 `onCaptureStarted`/`onCaptureCompleted`。
- FACT：HAL 日志同时有 `createStream : stream size is 0 x 230771904`，之后才出现 `Bypass ... width 1280 height 720`；这个异常尺寸需要单独验证，不能直接猜成 HAL 根因。
- 下一步：给 `AImageReader_getWindow` 后的 native window 加 width/height/format 读回，并显式调用 `ANativeWindow_setBuffersGeometry(1280,720,YUV_420_888)`，做单变量 A/B。
- 修改：新增 `ANativeWindow_getWidth/getHeight/getFormat` 读回、`ANativeWindow_setBuffersGeometry(1280,720,YUV_420_888)`，并记录 `request_set_u8` 的 control/ae 返回码。
- 验证：Android cargo check EXIT=0，release EXIT=0；新产物 657312 bytes，SHA-256 `e7d4dffeb0b64540b158b89b225015b8c460219b5cc4fd12e49e26ca0b196f24`。

### 16.14 native window geometry 与 request entry 全部正常（2026-11-28）

- FACT：`camera|reader|geometry|before=1280x720/35|set_rc=0|after=1280x720/35`；`ANativeWindow_setBuffersGeometry` 不是修复点。
- FACT：`camera|session|request_entries|control_mode_rc=0|ae_mode_rc=0`；request 的 control/AE entry 写入成功。
- FACT：上述版本仍无 capture callback、无 image-available、无帧。
- 判定：输出窗口尺寸/格式和 request entry 返回码都正常，排除这两个候选。
- 下一步单变量：移除 300ms `CAMERA_SESSION_CONFIG_GRACE` 等待，创建 session 后立即提交 repeating request，复现历史成功顺序。
- 验证：立即提交版本 cargo check EXIT=0、release EXIT=0；产物 657224 bytes，SHA-256 `0a9f07edce4f9b4a8d04860227891a9882ccdcc43348fdd7bcc96c5db67aad1a`。

### 16.15 立即提交仍失败，准备 one-shot capture A/B（2026-11-28）

- FACT：移除 300ms 等待后，创建 session 后立即提交 repeating；`ready_observed=false`、`repeat_rc=0`、`sequence=0`，仍无 capture/image-available、无帧。
- 判定：配置等待不是根因，且当前重复提交路径与历史成功路径的时序差异不足以解释零帧。
- 修改（诊断）：新增 `HUSK_CAMERA_ONESHOT=1` 才启用的 `ACameraCaptureSession_capture` 一次调用分支；不改变默认生产路径。
- 目的：one-shot 若能出帧，问题在 repeating；若仍零帧，问题在两个 API 共用的 request/output/HAL 路径。
- 验证：one-shot 诊断版 cargo check EXIT=0、release EXIT=0；产物 657576 bytes，SHA-256 `a3fe5861a18b8f90b7f8201326e440990bbec6839896e2d2675bcf16eafa0f20`。
- 下一步：用 `HUSK_CAMERA_ONESHOT=1` 启动，做 one-shot capture A/B。

### 16.16 one-shot capture 同样零帧（2026-11-28）

- FACT：`HUSK_CAMERA_ONESHOT=1`、`ACameraCaptureSession_capture` 返回 `rc=0|sequence=0`，`session_active` 到达。
- FACT：仍无 capture started/completed/failed、无 image-available；12.101s 后 `frames=0`。
- 判定：不是 `setRepeatingRequest` 特例；one-shot 与 repeating 共用路径都没有产生 buffer，问题在 request/output/HAL 绑定或 HAL 当前状态。
- 下一步黑盒对照：启动系统相机确认同一 HAL 当前能出帧；完成后立刻 force-stop 系统相机，避免双客户端混用。

### 16.17 one-shot 对照失败，系统相机对照成功（2026-11-28）

- FACT：`HUSK_CAMERA_ONESHOT=1` 的 `ACameraCaptureSession_capture` 返回 rc=0/sequence=0，仍 12 秒无帧。
- FACT：同一设备随后启动 `com.oplus.camera`，CameraService dump 显示 `Request ID counter: 4`，`Latest received frame` 已分配；系统相机冷启动 737ms 正常。
- FACT：系统相机已立即 force-stop，未与 Husk 并行保留。
- 判定：当前 HAL/硬件正常，问题确定在 Husk 的 Camera2 NDK session/request/output 路径。
- 下一步：补 `SESSION_ACTIVE` 观察；创建 session 后最多 200ms 用 16ms Looper poll，超时仍提交，验证历史成功路径中的 active 时序是否是差异。
- 验证：session-active 观察版 cargo check EXIT=0、release EXIT=0；产物 657760 bytes，SHA-256 `c6e16d4aed8218f56b53b5a660e3a6c98b4b3436b20a0bb137aa76393ad3e961`。
- 下一步：默认 repeating 启动该版本，记录 `state_observed` 的真实 active/ready 与是否出帧。

### 16.18 session_active 时序不是修复（2026-11-28）

- FACT：创建 session 后最多 200ms 的 16ms poll 期间 `state_observed|ready=false|active=false`；request 提交后下一毫秒才收到 `session_active`。
- FACT：该版本仍 12.115s 零帧。
- 判定：本 ROM 的 active 回调天然晚于 request 提交，不能把等待 active 作为门禁；该时序不是根因。
- 下一步：提高 Camera3/CameraDeviceClient 日志级别，抓 request 进入 HAL 后的 result/buffer 错误。

### 16.19 恢复现场核对（2026-11-28）

- FACT：已重新读取长期目标文件，硬约束仍是只改 Husk、不提交、相机以 Snapshot 真图为准、每步记账。
- FACT：Husk 工作区仍在 `main@c924d62`，未提交改动保持原状；未执行 commit/push/tag。
- FACT：设备 `UKHMMB8PYTGECMNV` 上 App PID=`6025`，前台为 `com.linuxhub.myth.husk/com.java.myapplication.MainActivity`。
- FACT：相机 helper PID=`16649`，exe 为 `./husk-bridges`，PPID=1；这是手工 `run-as` 调试身份，不是 App 正式管理身份，不能作为发布验收身份。
- 下一步：从现有 verbose Camera3/CameraDeviceClient 日志提取 request 提交到 MTK HAL 的完整调用链，判断是 CameraService 没下发还是 HAL 没回 buffer。

### 16.20 verbose 日志定位：request 返回成功但未形成 HAL 帧请求（2026-11-28）

- FACT：`03:24:08.577` MTK P1 只有开流内部 `<DRV.EnQ> MagicNum(1) FrameNum(-1) RequestNum(-1)`，对应 HAL 初始化而不是用户帧。
- FACT：Husk 在 `03:24:08.704` 收到 `setRepeatingRequest rc=0|sequence=0`，下一毫秒收到 `session_active`；之后日志中没有新的 `DRV.EnQ`、没有 capture started/completed/failed、没有 HAL result。
- FACT：关闭时 `Camera3-Device` 报 `waitUntilDrainedLocked ... timed out (-110)`；`dumpActiveComponents` 显示 RequestThread 仍 active。
- 判定：CameraService 接受了 request，但 request 没有形成完整的 HAL capture request；断点在 Camera3 request queue 到 output buffer/HAL 提交之间，不在 PipeWire、WirePlumber 或 Snapshot。
- 下一步：对照 AOSP Camera3Device request queue 与历史 `d2bd77b` 成功路径，重点检查 output stream buffer 获取和 request callback 绑定。

### 16.21 识别到运行身份 A/B 缺口（2026-11-28）

- FACT：历史 Evidence 89/90 的 Camera2 成功帧是在 Husk 自有 `NativeActivity` 进程（旧包 `com.linuxhub.husk`）内取得。
- FACT：当前失败路径是黑盒宿主 `com.linuxhub.myth.husk` UID 下、PPID=1 的独立 `husk-bridges` PID 16649；CameraService 仍把它识别为该包 UID 并允许开相机。
- FACT：当前 helper 运行的二进制 SHA-256 确为 `c6e16d4aed8218f56b53b5a660e3a6c98b4b3436b20a0bb137aa76393ad3e961`，排除了“跑错旧二进制”。
- 判定：必须用同一份当前相机源码在 Husk 自有 NativeActivity 进程做 A/B，区分 Camera2 代码回归与独立 helper 进程边界问题。
- 下一步：构建/安装 Husk 自有 APK，预先写 `camera-selftest`，从 NativeActivity 启动并记录是否出帧。

### 16.22 自有 APK 构建首次命令路径错误（2026-11-28）

- 失败：`NDK=/Users/l/Library/Android/sdk/ndk/28.2.13676358 make apk` 报 `tools/loader/build-embedded-loader.sh: no clang at .../28.2.13676358/bin/clang`。
- 原因：Makefile 的 `NDK_BIN` 接受的是实际 toolchain 的 `bin` 目录，不是 NDK 根目录。
- 纠正：改用 `/Users/l/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64/bin` 重试；不记源码失败。

### 16.23 当前源码自有 APK 构建成功（2026-11-28）

- FACT：使用 NDK toolchain `28.2.13676358/darwin-x86_64/bin` 执行 `make apk`，Gradle `BUILD SUCCESSFUL`。
- 产物：`app-debug.apk` 1,824,616 bytes，SHA-256 `577d962b3f1dea6276b40ff7d90c05a27ae386184d144d821a9d355ac902f669`。
- 原生件：`libhuskapp.so` SHA-256 `59f02961fc4b24645dabd0a553e9c2892ccdaa2e7490adaeffe0a2951886b505`；`libld.so` SHA-256 `003ca38427e00ea139d2b12210a3f454fd3ad168060026dc3c59e6d097f7c1ed`。
- 下一步：停止黑盒宿主的独立 helper，安装独立包 `com.linuxhub.husk`，预置 dummy rootfs 与相机 marker，从 NativeActivity 进程做同源码 A/B。

### 16.24 自有 APK 安装首轮挂起（2026-11-28）

- 失败：`adb install -r -g app-debug.apk` 超过 90 秒无输出，已手动中止；不记安装成功。
- 现场：ADB 设备仍在线；下一步拆成 `adb push` + 设备侧 `pm install`，区分传输与 PackageManager。

### 16.25 自有 APK 已安装，安装拦截恢复（2026-11-28）

- FACT：OPPO 的 `com.oplus.appdetail` 拦截页无直接确认动作；临时 `disable-user` 后由 AOSP `com.android.packageinstaller` 页面完成安装，已立即重新启用 `com.oplus.appdetail`。
- FACT：目标包 `com.linuxhub.husk` 已出现在 `pm list packages`，安装命令最终返回 `Success`。
- 操作偏差：后续组合命令把 `adb` 误写进设备 shell，后半段 EXIT=127；只影响该次拆分执行，不影响已完成的安装。
- 下一步：确认 CAMERA 权限/marker/dummy rootfs，然后从 `android.app.NativeActivity` 启动当前源码做相机 A/B。

### 16.26 自有 APK 相机权限已通过系统 UI 授予（2026-11-28）

- FACT：shell 被 ROM 禁止 `pm grant` 与 `appops set`；通过应用详情 → 权限管理 → 摄像头 →“使用时允许”完成授权。
- FACT：`dumpsys package com.linuxhub.husk` 显示 `android.permission.CAMERA: granted=true`，AppOps `CAMERA: foreground`。
- 下一步：清 logcat，从 NativeActivity 启动同一当前源码，等待 3 帧自测结果。

### 16.27 A/B 决定性结果：同一源码在 NativeActivity 出 3 帧，独立 helper 零帧（2026-11-28）

- FACT：独立 helper PID 16649、SELinux `runas_app`、PPID=1；Camera2 request `rc=0` 后无 capture callback，12 秒 `frames=0`。
- FACT：同一个 `camera.rs` 构建的自有 APK，PID 20802 从 `android.app.NativeActivity` 启动；Camera2 先 `acquire rc=-30001`，随后 `capture|started`、`acquire|recovered`、`capture|completed`。
- FACT：自有进程实收 3 帧：`1280x720`、`1,382,400 bytes/frame`、`mean=42.84`、`fps=30.00`；设备文件 `frame-00/01/02.nv12` 均存在。
- 判定：当前 Camera2 源码、request、ImageReader 和 HAL 链路均正常；故障由独立 helper 的运行身份/启动边界触发，重点转向 SELinux 域、进程来源与宿主生产启动路径。
- 下一步：查黑盒宿主正常启动 Linux 时的子进程、SELinux 域和 helper 启动方式，禁止再把手工 `run-as` helper 结果当生产结果。

### 16.28 找到宿主正式硬件桥入口（2026-11-28）

- FACT：宿主主页有“重启硬件桥”按钮；点击后新 `husk-bridges` PID=21490，PPID=6025（宿主 App 进程），不是手工 `run-as` 的 PPID=1。
- FACT：尝试用 `run-as` 读 PID 21490 的 `/proc/.../attr/current` 被拒绝，而手工 helper 可读；说明正式入口启动的进程 SELinux/进程域与 `runas_app` 不同。
- 判定：正式入口很可能恢复 `untrusted_app` 域，是应使用的生产路径。
- 下一步：保持宿主前台，写 `camera-selftest`，通过“重启硬件桥”启动，验证真实帧。

### 16.29 正式入口仍失败，并发现 helper 版本不一致（2026-11-28）

- FACT：宿主“重启硬件桥”启动 PID 21626、PPID=6025；CAMERA 权限已授权且 AppOps allow。
- FACT：PID 21626 日志只输出 `camera|session|open|id=0|1280x720|repeat_rc=0`，缺少当前源码必有的 `mode=repeating|sequence`、state/callback 日志，12 秒仍 `frames=0`，drain 再次 `-110`。
- 判定：正式入口启动的 helper 不是当前已构建的 `c6e16d4a...` 日志形态，存在旧 helper 副本/旧部署路径；必须先定位实际执行文件，不能把这次记为当前代码失败。
- 下一步：限制扫描 `files/linux` 非 rootfs 区域，列出所有 `husk-bridges*` 并逐份 SHA-256。

### 16.30 宿主启动时覆盖最新 helper（2026-11-28）

- FACT：设备上找到三份 helper：`files/husk-bridges` 456,880 bytes / SHA `2b12ee03...`；`files/linux/runtime/husk-bridges` 652,904 bytes / SHA `c0663333...`（mtime 03:37，被宿主启动过程刷新）；pre-callback 备份 654,496 bytes。
- FACT：当前源码构建产物应是 657,760 bytes / SHA `c6e16d4a...`，已不在宿主正式运行路径。
- 判定：宿主“重启硬件桥”从旧资产恢复/覆盖了 runtime，导致正式入口实际运行旧 helper；这是必须修的部署一致性问题。
- 下一步：将当前构建原子替换到 `files/linux/runtime/husk-bridges`，立即从正式入口重启，再比对 SHA 是否被覆盖。

### 16.31 确认旧 helper 来自宿主 APK 资产（2026-11-28）

- FACT：宿主 `base.apk` 内存在 `assets/husk/arm64-v8a/husk-bridges`，大小 652,904 bytes，与宿主启动后覆盖到 runtime 的 `c0663333...` 完全对应。
- 判定：正式入口每次启动都从宿主 APK 旧资产解包覆盖 latest runtime；这是宿主黑盒的资产版本覆盖问题，不通过改宿主源码规避。
- 实验：把当前 `c6e16d4a...` 写回正式路径，并将 runtime 目录/文件临时设为只读，阻止解包覆盖；测完立即恢复权限。

### 16.32 运行中 helper 替换触发 Text file busy（2026-11-28）

- 失败：向宿主正式运行路径覆盖当前 `husk-bridges` 时，目标文件仍被进程映射，出现 `Text file busy`。
- 判定：不能把运行中 inode 的覆盖当成部署成功；必须先结束 helper，再原子替换并校验 SHA。

### 16.33 临时只读 runtime 无法阻止宿主资产覆盖（2026-11-28）

- 失败：将 runtime 目录/文件临时设为只读后，替换当前 helper 出现 `Permission denied`；恢复写权限后，宿主正式入口仍能从 APK 旧资产恢复 runtime。
- FACT：只读目录没有形成对宿主进程的有效隔离，不能作为版本锁定手段。
- 判定：宿主黑盒的资产覆盖必须在官方 Runtime 更新入口或新版宿主资产层解决，禁止通过改宿主源码规避。

### 16.34 最新 helper 可短暂写入，但宿主启动后恢复旧版（2026-11-28）

- FACT：强制停止宿主、恢复 `700` 权限、删除目标并复制当前 helper 后，`files/linux/runtime/husk-bridges` 的 SHA 曾为 `c6e16d4aed8218f56b53b5a660e3a6c98b4b3436b20a0bb137aa76393ad3e961`。
- FACT：点击“重启硬件桥”后，该路径再次变回 `c0663333698fc7e55b961a29fbf47c01191bc75e4726b98ae63276080b5c2d75`。
- 判定：手工替换不能建立可复现的正式运行身份；这条路径不能用于当前源码的相机验收。

### 16.35 正式入口 PID 22155 仍是旧 helper 且零帧（2026-11-28）

- FACT：宿主正式入口拉起 `husk-bridges` PID `22155`，实际文件仍是旧 SHA `c0663333...`。
- FACT：该进程仍表现为旧日志字段，12 秒 `frames=0`。
- 判定：该结果只证明旧 helper 失败，不能归因于当前相机源码；不计入当前版本回归。

### 16.36 宿主重启后的设备现场（2026-11-28）

- FACT：宿主 `com.linuxhub.myth.husk` 当前 PID `20925`，位于前台；当前没有 `husk-bridges` 子进程。
- FACT：宿主 runtime 为旧 `husk-bridges`，652,904 bytes，SHA `c0663333...`；runtime 目录已恢复 `700`。
- FACT：宿主 CAMERA `granted=true`，AppOps 为 `foreground`；Husk 自有包 `com.linuxhub.husk` 的 CAMERA 也已授予。
- FACT：设备 marker 为 `frames=3 camera=0`；OPPO 安装拦截组件 `com.oplus.appdetail` 已恢复启用。

### 16.37 后摄黑帧的证据边界（2026-11-28）

- FACT：NativeActivity PID `20802` 实收 3 帧 NV12；当时后摄朝下，Y 平面 `min=0 max=2 mean=0.264`，画面接近全黑。
- 判定：buffer 生命周期与 Camera2 回调已证明可用，但黑帧不能冒充 Snapshot 真图验收。
- 下一步：切前置 `camera=1`，先证明非黑帧；再解决正式 helper 部署一致性与 PipeWire/WirePlumber 会话入口，最终以 GNOME Snapshot 出图作为相机 PASS。

### 16.38 当前待验证假设（2026-11-28）

- 推断（尚未验证）：独立 `husk-bridges` 没有 Android Runtime 的 binder 线程池，Camera2 callback/result 无法正常进入该进程。
- 单变量实验：动态加载 `libbinder_ndk.so`，在 Camera2 manager/session 启动前调用 `ABinderProcess_startThreadPool()`；仍通过 `run-as` 启动当前 helper，并与未调用版本做正负对照。
- 边界：该实验只能验证进程边界假设；成功后仍需解决宿主官方 runtime 更新，不能把 `run-as` 结果冒充正式发布验收。

### 16.39 独立 helper Binder 线程池单变量改动启动（2026-11-28）

- 修改范围：`app-rs/src/camera.rs` 的独立 helper 相机启动路径。
- 改动：动态加载 `libbinder_ndk.so`，解析 `ABinderProcess_startThreadPool`；在 `ACameraManager_create` 前调用一次并记录结果。
- 不变项：Camera2 template/request/output、V4L2 façade、PipeWire/WirePlumber、宿主 APK 均不改。
- 验证计划：先 Android target `cargo check`，再 release 构建记录 SHA；随后 `run-as` 启当前 helper，与 SHA `c6e16d4a...` 的未调用版本作负向对照。

### 16.40 Binder 线程池版静态检查通过（2026-11-28）

- FACT：Android `cargo check --bin husk-bridges` 输出 `Finished dev profile`，`CARGO_CHECK_EXIT=0`。
- FACT：无新增编译错误；仍有既有 20 条 warning（含 camera 未使用 unsafe/dead code）。
- 下一步：release 构建并复算产物 SHA。

### 16.41 Binder 线程池版 release 构建成功（2026-11-28）

- FACT：Android release 构建 `Finished release profile`，EXIT=0。
- 产物：`app-rs/target/aarch64-linux-android/release/husk-bridges`，658,240 bytes。
- SHA-256：`c44f23f97618224d655ba4d9c75ff17a4891fda7d98fb0fb06a25a6c1a54ab3f`。
- 下一步：将该二进制部署到宿主黑盒的调试运行路径，通过 `run-as` 启动；先确认 Binder 调用日志出现，再判断是否出帧。

### 16.42 Binder 线程池版已部署到调试路径（2026-11-28）

- FACT：通过 `adb push` + `run-as` 部署到宿主 App 调试路径 `files/linux/runtime/husk-bridges.binder-pool`；没有覆盖正式 runtime。
- 设备复算：658,240 bytes，mode `700`，owner `u0_a265`，SHA-256 `c44f23f97618224d655ba4d9c75ff17a4891fda7d98fb0fb06a25a6c1a54ab3f`，与构建产物一致。
- 下一步：使用同一 `camera-selftest=frames=3 camera=0` marker，清日志后经 `run-as` 启动并等待 12 秒。

### 16.43 决定性结果：Binder 线程池修复独立 helper 相机回调（2026-11-28）

- FACT：设备进程 PID `22523` 运行的是 `husk-bridges.binder-pool`，UID `u0_a265`，PPID `1`，SELinux 为 `runas_app`；不是 NativeActivity，也不是宿主正式入口。
- FACT：启动日志首先出现 `camera|binder|thread_pool_started`，随后 Camera2 manager/list 正常，枚举到后摄 id=0 与前摄 id=1。
- FACT：`setRepeatingRequest` 返回 `rc=0|sequence=0` 后，实收：
  ```text
  camera|callback|session_active
  camera|capture|started|count=1|timestamp=2246686569000
  camera|image_reader|available|count=1|reader_null=false
  camera|frame|acquire|recovered
  camera|frame|ready|seq=1|1280x720|ts=2246686569000
  camera|capture|completed|count=1
  camera|selftest|DONE|frames=3|fps=20.00|span_ms=99.99|elapsed_ms=385
  ```
- FACT：3 帧均为 `1,382,400` bytes；Y 平面统计分别为 `min=0 max=128 mean=42.847/42.859/42.862`，不是上一轮近全黑画面。
- FACT：帧 SHA-256：`frame-00 6cd5a6ae269dd3588b0ccc0c3c4f904d009bd72366f40d0ec297be2b7f34ef14`；`frame-01 390c32bdd4ce4dae9a703b79f8af3e566b17f8a808930ab349794ef9bbf84e22`；`frame-02 1eeeb361a4602858df1d99e35af4837a4ee108813673c96f2758ea8596142580`。
- 判定：先前“独立 helper 零帧”的根因确定为缺少 binder 线程池。未调用版本 `c6e16d4a...` 为零帧负向对照；调用版本 `c44f23f9...` 出 3 帧正向结果。
- 边界：这证明相机桥进程边界已修复，不等于 Snapshot 已可用；继续验证 camera.sock、V4L2、PipeWire/WirePlumber 与 GNOME Snapshot 全链路。

### 16.44 Snapshot 前定位到设备媒体脚本旧版（2026-11-28）

- FACT：点击“进入桌面”后 GNOME/X11 会话启动；PipeWire PID `23274`、WirePlumber PID `23287` 均运行。
- FACT：PipeWire 日志报 `spa.v4l2 ... /dev/video0 is no device`；WirePlumber 的 V4L2 monitor 未形成可用相机。
- FACT：设备 `/proc/23274/maps` 不含 `libhusk_v4l2.so`，environ 不含 `LD_PRELOAD`；而 `/proc/23287/maps` 含 preload、environ 有正确宿主路径。
- FACT：设备安装的 `/usr/local/bin/husk-media-session` 旧版启动 PipeWire 时直接执行 `pipewire -c ...`，没有源码中的 `LD_PRELOAD="$V4L2_PRELOAD"`；源码版本已包含该修复。
- 判定：这是当前 rootfs 的部署不一致，不是新的 Camera2 回归。下一步仅更新 `husk-media-session`，停止并重启两个媒体服务后复查 V4L2 枚举。

### 16.45 媒体脚本更新后 V4L2 枚举成功，Snapshot 仍黑屏（2026-11-28）

- 修复部署：源码 `husk-media-session.sh`（5880 bytes，SHA-256 `93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5`）已复制到 rootfs `/usr/local/bin/husk-media-session`，设备 SHA 一致。
- FACT：重启后 PipeWire PID `24437`、WirePlumber PID `24447` 均存活；两者 `/proc/<pid>/maps` 都加载 `libhusk_v4l2.so`，environ 都有宿主 rooted `LD_PRELOAD` 与 `NR_ROOTFS`。
- FACT：`wpctl status` 的 Video 列出：
  ```text
  39. BACK   [v4l2]
  40. FRONT  [v4l2]
  36. BACK (V4L2)
  42. FRONT (V4L2)
  ```
- FACT：启动 GNOME Snapshot 46.2 后窗口出现，但预览区域为纯黑、左下角持续转圈；进程 PID `24600` 存活（loader 显示为 `libld.so`，`/proc/24600/status` Name=`snapshot`）。
- FACT：Husk 日志出现 `camera|capture|failed|count=30|frame=44|reason=1|sequence=0|image_captured=0`；说明 Camera2 stream 已运行过若干帧后发生捕获失败，不能据此判定整链路成功。
- 下一步：用 `GST_DEBUG` 重启 Snapshot，确认黑屏是格式协商、PipeWire 取帧、NV12 转换还是 X11 呈现；保留该黑屏截图作为失败现场，不勾相机 PASS。

### 16.46 直连 camera.sock 正对照仍为 3 帧非黑（2026-11-28）

- FACT：对 Binder 线程池修复 helper（PID `24304`）直接运行 `camera-client`，收到 3 帧，输出：
  ```text
  camera-client|FRAME|seq=1|1280x720|bytes=1382400|min=0|max=128|mean=42.86
  camera-client|PASS|frames=3
  ```
- 判定：Camera2 → helper → `camera.sock` → `camera-client` 的帧数据仍然正常，黑帧不是 Camera2 不产帧。

### 16.47 Snapshot 已连接 PipeWire，但预览仍为黑屏（2026-11-28）

- FACT：GNOME Snapshot 46.2 的 PipeWire node `46 org.gnome.Snapshot` 已建立输入 port `47 input_1 < BACK:capture_1 [active]`，显示请求为 `1920x1080`；不是“没枚举到相机”。
- FACT：窗口正常出现，但预览区域纯黑并持续转圈；Camera2 同时记录 capture started / image available / capture completed，随后出现 `capture|failed|reason=1`。
- 判定：问题已缩小到 V4L2 消费/传输路径，不能继续归因于 Camera2 枚举或 Snapshot 入口。

### 16.48 V4L2 `read()` 路径正对照正常（2026-11-28）

- FACT：`v4l2-test` 使用 `read` 模式取得：
  ```text
  v4l2test|read|frame|0|bytes=1382400|min=0|max=128|mean=42.87
  ```
- 判定：V4L2 协议 façade、格式协商、camera.sock 取帧和非黑数据在 `read()` 路径上均正常。

### 16.49 V4L2 `mmap + DQBUF` 路径收到全黑帧（2026-11-28）

- FACT：1280x720 mmap 测试的 REQBUFS / QUERYBUF / QBUF / DQBUF 全部返回成功，但保存的 PPM RGB 统计 `R/G/B max=0`。
- FACT：1920x1080 mmap 测试同样完成协议流程，RGB extrema 为 `R/G max=0, B max=2`。
- 判定：全黑发生在 `mmap + DQBUF` 数据落点或映射一致性上；不是 `read()` 所走的 socket 数据源问题。

### 16.50 mmap 黑帧定位实验（2026-11-28）

- 重点链路：`v4l2-shim::dqbuf()` 写 `QueryBuf.addr`；`v4l2-shim::mmap()` 另建匿名映射并登记地址；`path-shim` 分别暴露 libc `mmap` 和 libv4l2 `v4l2_mmap` 入口。
- 待验证假设一：guest 的 `mmap@GLIBC_2.17` 符号未进入 shim，调用方拿到 memfd 零页，而 shim 写入了另一份匿名映射。
- 待验证假设二：`v4l2_mmap()` 返回给调用方的地址与 DQBUF 实际写入的 `QueryBuf.addr` 不一致。
- 下一步：以 `NR_V4L2_AUDIT=1` 重跑同一 1280x720 mmap 用例，确认日志出现 `core mmap` / `libv4l2 v4l2_mmap`，并记录调用方地址、QUERYBUF offset、DQBUF index 与写入地址。

### 16.51 当前 loader 下 1280x720 mmap 正结果（2026-11-28）

- FACT：使用设备当前桌面 loader `/data/data/com.linuxhub.myth.husk/files/linux/husk/libld.so`（1,066,496 bytes，SHA-256 `0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94`）重跑：
  ```text
  v4l2test|setfmt|1280x720|3231564e|sizeimage=1382400
  v4l2test|buf|0|len=1382400|off=0
  ...
  v4l2test|frame|0|idx=0|bytes=1382400|seq=1|ts=2994.673435
  v4l2test|done|frames=3|bytes=4147200|fps=9.93
  v4l2test|OK
  ```
- FACT：保存 raw `1,382,400` bytes，SHA-256 `8ef42cf081d6e30db0c4b3a5a897452e15a454f0ccb5daa7781b8df05b93eed4`；统计 `min=0 max=128 mean=42.9102`，非黑。
- 判定：当前 loader 下 `13908` 的 V4L2 mmap 分支不是必然黑屏。之前“1280x720/1920x1080 mmap 全黑”必须补记所用 loader 身份，不能继续作为当前版本结论。
- 下一步：核对当前 GNOME/PipeWire 实际 loader，和旧黑帧实验做 SHA 单变量对照；再用当前 loader 重启 Snapshot。

### 16.52 需要继续区分的变量（2026-11-28）

- 当前正结果来自 `files/linux/husk/libld.so`，不是 rootfs 父目录的 `distributions/ubuntu-24.04/libld.so`（869,888 bytes，SHA-256 `67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`）。
- 未验证：旧黑帧实验是否使用了旧 loader、旧 `libhusk_v4l2.so`、或不同 helper/socket；必须先复核命令与设备二进制身份，再决定改哪一层。
- 约束：不把“新 loader 正常”直接当成 Snapshot 已修复；最终仍以 Snapshot 真图为准。

### 16.53 GStreamer fixture 测试挂起，先恢复 camera.sock（2026-11-28）

- FACT：使用当前 loader 运行 `/root/camera-fixture --gst 1280 720 3`，输出停在 `GST_AUDIT|pipeline|1280x720|frames=3`；50 秒超时 EXIT=124，没有 EOS、PASS 或错误消息。
- FACT：该 fixture 默认使用 `/run/husk/camera.sock`，测试期间替换了生产 listener 的路径；测试被 timeout 终止后留下测试 socket，不能继续给 Snapshot 使用。
- 判定：GStreamer 路径存在新的挂起/协商阻塞；这是失败，不是 PASS。先恢复正式 helper socket，再抓 `GST_DEBUG`/PipeWire 协商证据。

### 16.54 正式 helper 恢复与直连正对照（2026-11-28）

- FACT：清理 GStreamer fixture 留下的测试 socket 后，使用当前 debug helper `husk-bridges.binder-pool` 重新启动正式 listener；PID `26299`，PPID 切回 `1`，`camera.sock` 已重建且 owner/UID 正确。
- FACT：恢复后运行 `camera-client`，收到 3 帧：
  ```text
  camera-client|OPEN|OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=0
  FRAME|mean=42.86
  FRAME|mean=42.89
  FRAME|mean=42.89
  camera-client|PASS|frames=3|elapsed_ms=310|bye=BYE|seqs=1,2,3
  ```
- 判定：socket/helper 已恢复；下面的 GStreamer 测试必须使用独立失败现场，结束后再次确认正式 listener。

### 16.55 当前 loader 下干净 V4L2 1280x720 复跑 PASS（2026-11-28）

- FACT：清理 fixture 进程后重建正式 helper，使用当前 loader 再跑 `v4l2-test /dev/video0 1 NV12`，完成 QUERYCAP、格式协商、REQBUFS、mmap、STREAMON、DQBUF、STREAMOFF，输出 `v4l2test|OK`。
- FACT：当前单帧 raw 为 `1,382,400` bytes，时间戳和序列号正常；因此当前 loader 的核心 V4L2 mmap 路径仍可正向运行。
- 边界：这次没有打印 `NR_V4L2_AUDIT` 行，审计开关没有在 loader 环境生效；不能把缺少 audit 行解释成 mmap 未命中，需后续修正审计注入。

### 16.56 GStreamer 无 preload 时在 `gst_parse_launch` 卡死（2026-11-28）

- FACT：`/root/camera-fixture --gst 1280 720 3` 在 GStreamer 初始化后停在：
  ```text
  GST_AUDIT|pipeline|1280x720|frames=3
  ... gst_parse_launch_full: parsing pipeline ...
  ... gst_plugin_feature_load: loading plugin video4linux2
  ... gst_v4l2_probe_and_register: Probing devices
  ```
- FACT：进程主线程 `wchan=unix_stream_read_generic`，`/proc/<pid>/syscall` 为 `read`；并存在 V4L2 socket 连接等待。
- 判定：失败发生在插件探测/设备连接阶段，不是 `set_state(PLAYING)` 后的帧消费阶段。此结果不能直接归因于 Snapshot，但证明当前测试路径未覆盖真实完整 GStreamer。

### 16.57 正确 preload 后揭示 fixture 只服务一个客户端（2026-11-28）

- FACT：用 host-path `LD_PRELOAD=/data/.../rootfs/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so` 重跑，trace 显示：
  ```text
  [husk v4l2] camera.sock connected; sending INFO
  [husk v4l2] INFO reply: CAM k=0 id=fixture facing=back name=FIXTURE ...
  [husk v4l2] INFO reply: OK
  [husk v4l2] camera.sock connected; sending INFO
  [husk v4l2] INFO sent; waiting for reply
  ```
- FACT：第二次连接是同一 GStreamer 探测对下一个 `/dev/videoN` 的 INFO，fixture server 仍停留在第一个 `serve_client`，没有 accept/响应第二客户端；50 秒 timeout。
- 判定：当前 fixture 测试存在“双设备 / 单客户端”缺陷，不能作为生产相机失败证据。需要修夹具，使两个设备都可响应并继续验证 GStreamer。

### 16.58 修复测试夹具后 GStreamer 1280x720 PASS（2026-11-28）

- 修复 `runtime/crates/camera-fixture/src/main.rs`：INFO 返回 `CAM k=0` 与 `CAM k=1` 两条；`--gst` server 改为并发 accept，每个探针连接独立服务，并在完整握手后允许 EOF/超时正常收尾。
- 验证：`cargo test -p camera-fixture` PASS；交叉编译 `aarch64-unknown-linux-gnu` release PASS；产物 625,072 bytes，SHA-256 `f8efd0c6387000670687fbc64fa384ce5fa998b46255be28f7b8e27b2ec69ce2`。
- 部署：rootfs `/root/camera-fixture` 的 SHA 与构建一致。
- FACT：带 host-path `libhusk_v4l2.so` 重跑，实际得到：
  ```text
  [husk v4l2] INFO reply: CAM k=0 ...
  [husk v4l2] INFO reply: CAM k=1 ...
  FIXTURE|frame|seq=1|bytes=1382400
  FIXTURE|frame|seq=2|bytes=1382400
  FIXTURE|frame|seq=3|bytes=1382400
  GST_AUDIT|EOS|1280x720
  GST_AUDIT|PASS|1280x720|frames=3
  ```
- 判定：V4L2 + libv4l2 preload + GStreamer 1280×720 路径已通过；此前挂起是 fixture 单客户端/单 camera index 缺陷，不是生产链路结论。
- 边界：这是合成 fixture，不是真实 Camera2 或 Snapshot 画面；下一步恢复正式 helper，启动 Snapshot 验证真实窗口预览。

### 16.59 Snapshot 实机黑屏，且运行身份落在旧 loader（2026-11-28）

- FACT：GNOME Snapshot 进程 PID `27595` 仍在运行，窗口正常但预览为纯黑并持续转圈；截图 `/tmp/husk-snapshot-20261128.png` 为 2400×1080。
- FACT：日志 `/data/data/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/tmp/snapshot-run.log` 约 84 KB，主要是 `glupload` modifier 警告，没有可直接定案的相机错误。
- FACT：Snapshot 已持有 PipeWire memfd、`/dev/mali0` 和 PipeWire socket；说明媒体服务与 GPU 设备至少已接上。
- FACT：`/proc/27595/maps` 与实际环境显示 Snapshot 使用 `files/linux/distributions/ubuntu-24.04/libld.so`，大小 `869,888` bytes，SHA-256 `67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`。
- 对照：当前桌面、debug helper、PipeWire/WirePlumber 使用 `files/linux/husk/libld.so`，大小 `1,066,496` bytes，SHA-256 `0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94`。
- 判定：本次 Snapshot 黑屏是**旧 bundle loader 应用现场**，不能作为当前 Husk runtime 的最终相机 FAIL/PASS；但它暴露了正式桌面入口的运行身份不一致缺陷，必须先通用修复。
- 下一步：追踪 `husk-session.sh -> /usr/local/bin/doro-gpu-run snapshot -> execve/loader` 的实际选择链路，不加 Snapshot 特例。

### 16.60 运行身份差异已定位到 Snapshot 的 NR_LD 环境（2026-11-28）

- FACT：当前 GNOME Shell、PipeWire、WirePlumber 进程的 `NR_LD` 均为 `/data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so`；三者 `/proc/<pid>/maps` 均命中该文件。
- FACT：Snapshot PID `27595` 的 `environ` 明确为 `NR_LD=/data/data/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/libld.so`，不是该进程内动态回退产生。
- 代码对照：path-shim 的 `execve` 在动态 ELF 重入时优先读取 `envp` 里的 `NR_LD`；只有缺失时才回退到 rootfs 内的 `usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1`。
- 判定：当前第一问题是 Snapshot 启动环境本身带着旧 `NR_LD`，必须找出它从哪个启动入口继承；不能先改 loader 选择逻辑掩盖错误环境。
- 下一步：核对 `/tmp/doro-gpu-runtime.log`、进程启动时间/cwd/exe 和桌面图标启动路径，区分“旧手工命令现场”与“正式桌面入口缺陷”。

### 16.61 旧 Snapshot 现场更像旧手工启动，不等于桌面图标入口（2026-11-28）

- FACT：PID `27595` 的 `PPid=1`、`cwd=.../rootfs/root`、`exe=.../distributions/ubuntu-24.04/libld.so`，cmdline 使用 `/data/data/com.linuxhub.myth.husk/...` 路径。
- 对照：当前正式桌面进程使用 `/data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so`，GNOME Shell/PipeWire/WirePlumber 均为新 loader。
- FACT：`tmp/doro-gpu-runtime.log` 有多条不同时间、不同命令拼写的 Snapshot 启动记录（`snapshot`、`/usr/bin/snapshot`），没有时间戳，单靠日志不能区分入口。
- 判定：旧进程的全路径和父进程形态符合直接调试启动遗留现场，尚不能断言“桌面图标启动必落旧 loader”。必须关掉旧进程并从当前桌面实际重开，才能定性。
- 下一步：从 X11 当前窗口关闭 PID 27595 的 Camera 窗口，再点击左侧 Snapshot 图标，核对新 PID 的 `NR_LD`/maps/环境。

### 16.62 桌面 `.desktop` 复现先暴露 DISPLAY 继承缺口（2026-11-28）

- FACT：关闭旧 Camera 窗口后，Dock 中该运行中应用图标消失；按固定坐标点击实际打开了“设置”，没有启动 Snapshot。旧 PID `27595` 已关闭。
- FACT：用 `husk-session.sh` 执行 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 时，Snapshot 输出：
  ```text
  Snapshot (org.gnome.Snapshot)
  Version: 46.2
  Gtk-WARNING: Failed to open display
  ```
- 代码对照：`husk-session.sh` 最终用 `env -i`，固定环境表没有自动携带 `DISPLAY`、`XDG_RUNTIME_DIR`、`DBUS_SESSION_BUS_ADDRESS`；直接通过新 session wrapper 调 `.desktop` 与现有 GNOME 会话环境不等价。
- 判定：这次失败是测试启动环境不完整，不能归因于 Camera/PipeWire；也提示正式宿主若从无图形环境的入口拉起 GUI，必须补齐显示/session bus 环境。
- 下一步：在命令层显式恢复当前 GNOME 会话的 `DISPLAY=:0`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、`DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk`，再走同一 `.desktop` 的 `Exec=/usr/local/bin/doro-gpu-run snapshot`。

### 16.63 正确桌面入口复测：运行身份已闭环，Snapshot 仍黑屏（2026-11-28）

- 操作：补齐当前 GNOME 会话的 `DISPLAY`、`XDG_RUNTIME_DIR`、session bus 后，通过 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 走原有 `Exec=/usr/local/bin/doro-gpu-run snapshot`。
- FACT：新 Snapshot PID `28996`，`exe=/data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so`；maps 命中同一新 loader。
- FACT：新进程 environ 为 `NR_LD=.../files/linux/husk/libld.so`、`NR_ROOTFS=.../ubuntu-24.04/rootfs`、`DISPLAY=:0`，旧 bundle loader 不再出现。
- FACT：maps 已加载 `usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so`；fd 8/9 指向 `/dev/mali0`；存在多个 `pipewire-memfd`，其中 fd 26 约 `33,177,984` bytes。
- FACT：实机截图仍为 Camera 窗口纯黑、底部转圈，没有预览画面。
- 判定：**运行身份问题已闭环**；相机黑屏不能再用旧 loader 解释。当前失败在 PipeWire/WirePlumber、V4L2 数据流或 Snapshot 呈现链路。
- 下一步：抓 `wpctl status`、PipeWire node/port 状态、`camera.sock` 客户端握手和 Camera2 capture 日志，先把“有帧到 PipeWire”与“有帧到 Snapshot”分开。

### 16.64 PipeWire 已连到 BACK，但 STREAMON 被 helper 拒绝为 busy（2026-11-28）

- FACT：`wpctl status` 中 Snapshot 客户端 PID 为 `28996`；Video 设备为 BACK/FRONT，Streams 为 `org.gnome.Snapshot -> input_1 < BACK:capture_1 [active]`。
- FACT：`pw-link -l` 显示链路存在：
  ```text
  v4l2_input._sys_class_video4linux_video0:capture_1
    |-> org.gnome.Snapshot:input_1
  ```
- FACT：`pw-top -b -n 1` 中 `v4l2_input video0`、`video1` 和 Snapshot 的 QUANT/RATE 均为 `0`，没有实际帧速率。
- FACT：`tmp/husk-pipewire.log` 明确记录：
  ```text
  [v4l2] STREAMON: app said: ERR busy
  spa.v4l2 ... '/dev/video0' VIDIOC_STREAMON: Device or resource busy
  pw.node ... idle -> error (Start error: Device or resource busy)
  ```
- 代码对照：V4L2 shim 收到 `ERR busy` 后返回 `-EBUSY`；helper 在 `OPEN` 时如果已有同一客户端会话，或全局 `CAMERA_SESSION_OWNER` 已被其他客户端持有，就返回 `ERR busy`。
- FACT：helper PID `27473` 仍持有多个 Camera2 `ImageReader-1920x1080` dmabuf 和 ashmem fd；logcat 持续出现 `ACameraDevice: Device error received, code 3`，Camera3 报 `Can't dequeue next output buffer: Connection timed out (-110)`。
- 判定：Snapshot 已走到 V4L2 STREAMON；失败点是 helper/Camera2 会话占用与缓冲回收，不是 Camera2 枚举、PipeWire 发现或 V4L2 façade 缺失。
- 下一步：受控关闭 Snapshot、媒体服务和当前 helper，重新启动干净 helper + 媒体服务，再走同一 `.desktop`；如果仍 busy，检查 helper 的会话释放和 Camera2 buffer 生命周期。

### 16.65 第一次受控清场失败：helper 启动路径写错（2026-11-28）

- 操作：关闭 Snapshot，停止媒体服务，终止旧 helper PID `27473`，清理 `run/husk/camera.sock`，随后尝试启动新 helper。
- FAIL：启动日志为：
  ```text
  setsid: exec ./files/linux/runtime/husk-bridges.binder-pool: No such file or directory
  ```
  原因是命令已 `cd` 到 `$HUSK_FILES`，仍使用相对路径 `./files/...`。
- FACT：helper 没有启动，`camera.sock` 不存在；随后媒体服务只得到 `pipewire=ready`，但 `wireplumber=not-ready`。
- 判定：这是清场脚本的路径错误，不是相机链路新结论；本次重置无效，不能用于验收。
- 纠正：用绝对路径 `$FILES/linux/runtime/husk-bridges.binder-pool` 启动 helper，确认 socket 后再重启媒体服务。

### 16.66 helper 已干净重启；WirePlumber 需要完整桌面 session 环境（2026-11-28）

- 纠正后使用绝对路径启动 helper：PID `29913`，PPID `1`，`run/husk/camera.sock` 于 04:13 重建。
- FACT：新 helper 启动日志只有正常的 `husk-bridges|start|...`，初始 fd 中未再出现上一轮残留的 1920×1080 Camera2 dmabuf。
- 操作：通过 `husk-session.sh` 只补 `XDG_RUNTIME_DIR=/tmp/runtime-husk` 启动媒体栈；PipeWire PID `30098` 起来，WirePlumber PID `30114` 报 not-ready。
- FAIL：`tmp/husk-wireplumber.log` 记录：
  ```text
  Failed to connect to session bus: Unable to autolaunch a dbus-daemon without a $DISPLAY for X11
  Error acquiring bus address: 无法在没有 X11 $DISPLAY 的情况下自动启动 D-Bus
  wireplumber ... on_disconnected: disconnected from pipewire
  ```
- 代码对照：`husk-desktop.sh` 的 `guest_env` 会同时设置 `DISPLAY`、`XDG_RUNTIME_DIR`、`DBUS_SESSION_BUS_ADDRESS` 和 `DBUS_SYSTEM_BUS_ADDRESS`；手工 session wrapper 只传 XDG 与桌面内实际调用不等价。
- 判定：WirePlumber 未启动是测试环境缺口；helper/socket 已恢复。下一步按 `guest_env` 等价环境重启媒体栈。

### 16.67 完整桌面环境下干净媒体栈 PASS（2026-11-28）

- 操作：按 `husk-desktop.sh::guest_env` 等价设置 `DISPLAY=:0`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、session/system bus 后停止并重启媒体栈。
- FACT：
  ```text
  husk-media-session: started pipewire pid=30273
  husk-media-session: started wireplumber pid=30285
  husk-media-session: pipewire=ready pid=30273
  husk-media-session: wireplumber=ready pid=30285
  ```
- FACT：`wpctl status` 列出 BACK/FRONT 两个 V4L2 设备与 BACK/FRONT 两个 Source，未见旧 Snapshot 流。
- 判定：干净 helper + 正确 session 环境可让 PipeWire/WirePlumber 完整就绪；这是下一步真实 Snapshot 复测的有效基线。

### 16.68 干净重置后 Snapshot 仍触发 STREAMON busy（2026-11-28）

- 基线：helper PID `29913` 全新启动，初始无 Camera2 dmabuf；媒体栈 PID `30273/30285` 均 ready；无旧 Snapshot 流。
- 操作：清空 logcat 后，从同一 `.desktop`（`doro-gpu-run snapshot`）启动 Snapshot，PID `30412`。
- FAIL：`wpctl status` 仍有 `input_1 < BACK:capture_1 [active]`，但 `pw-top` 三行速率仍全为 0。
- FAIL：`tmp/husk-pipewire.log` 重新出现：
  ```text
  [v4l2] STREAMON: app said: ERR busy
  '/dev/video0' VIDIOC_STREAMON: 设备或资源忙
  (v4l2_input...-36) idle -> error (Start error: 设备或资源忙)
  ```
- FACT：新 helper 在没有任何成功预览前就持有 1920×1080 `ImageReader` dmabuf；logcat 再次出现 Camera3 output buffer timeout 和 `camera|capture|failed|reason=1`，截图仍纯黑。
- 判定：busy 不是旧 helper 残留，而是可重复的会话所有权/缓冲消费问题。需要判断首个 Camera2 session 是 Snapshot 的 PipeWire source 还是 WirePlumber 自动探测持有。
- 下一步：关闭 Snapshot，观察 helper 是否释放 dmabuf/CameraSession；再单独停 WirePlumber 做对照。

### 16.69 关闭 Snapshot 后 helper 会话正常释放（2026-11-28）

- 操作：关闭 Snapshot PID `30412`，等待 6 秒后统计 helper PID `29913` 的 fd/thread。
- FACT：
  ```text
  before_dmabuf=15
  before_ashmem=15
  before_threads=19
  after_dmabuf=0
  after_ashmem=0
  after_threads=16
  ```
- FACT：此后 `ps` 不再有 Snapshot 进程。
- 判定：helper 的 CameraSession、ImageReader 和 dmabuf 会在 Snapshot 退出后释放；问题不是退出泄漏。
- 推断：busy 发生在 Snapshot 活跃期间的同一条 PipeWire 取流链内部，可能是探测/重配置产生的第二次 `OPEN`，或首个 Camera2 session 未完成 buffer 消费。
- 下一步：以 `NR_V4L2_AUDIT=1` 重启媒体栈，记录 PipeWire 进程中每次 `open/ioctl/STREAMON/close`，定位重复 OPEN 的来源。

### 16.70 审计开启后真实 Snapshot 仍黑屏；busy 发生在活跃取流链内部（2026-11-28）

- 基线：干净 helper PID `29913`（PPID `1`、`husk-bridges.binder-pool`），PipeWire PID `11003`，WirePlumber PID `11015`；媒体进程已开启 `NR_V4L2_AUDIT=1`、`PIPEWIRE_DEBUG=3` 和 host-path `LD_PRELOAD=.../libhusk_v4l2.so`。
- 操作：从同一 Snapshot `.desktop` 入口再次启动 GNOME Snapshot；最新截图捕获到 `/tmp/husk-snapshot-audit.png`。
- FAIL：截图亲眼确认 Snapshot 窗口与快门/切换控件正常显示，但预览区纯黑并持续转圈，不能判定相机 PASS。
- FACT：审计日志 `rootfs/tmp/husk-pipewire.log` 显示 Camera2 本会话曾成功启动与取图：
  ```text
  camera|session|open|id=0|1920x1080|mode=repeating|repeat_rc=0|sequence=0
  camera|callback|session_active
  camera|capture|started|count=30
  camera|image_reader|available|count=30
  camera|capture|completed|count=30
  ```
- FACT：随后仍持续出现：
  ```text
  Camera3-OutputStream: Can't dequeue next output buffer: Connection timed out (-110)
  ACameraDevice: Device error received, code 3
  ```
- FACT：PipeWire 向 V4L2 façade 请求 `YUYV 1920x1080 30/1`，链路激活后审计仍记录：
  ```text
  [v4l2] STREAMON: app said: ERR busy
  '/dev/video0' VIDIOC_STREAMON: 设备或资源忙
  node idle -> error
  ```
- FACT：审计期间 `[husk v4l2]` 约 135 行，`STREAMON` 出现 2 次；SPA 在初始化/重配置时多次 open/close/INFO，说明存在重复设备重开。
- 判定：Camera2 底层不是完全不出帧；当前真实阻塞是活跃 Snapshot 会话内的 Camera2 输出缓冲超时与 V4L2 session 重入冲突。最终相机门禁仍为 **FAIL**。
- 下一步：对齐第一个/第二个 `STREAMON` 的完整时间线，给 helper 增加通用会话生命周期日志，确认是旧 CameraSession 未及时 STOP，还是首个 V4L2 会话未消费 DQBUF，再做通用原生修复。

### 16.71 图已连接但 PipeWire 两路 RATE=0，数据环路卡在等待事件（2026-11-28）

- FACT：用完整的 `XDG_RUNTIME_DIR=/tmp/runtime-husk`、DISPLAY 和 session bus 调用 guest `pw-top -b -n 2`，输出为：
  ```text
  R 36 ... RATE 0 ... YUY2 1920x1080 v4l2_input._sys_class_video4linux_video0
  R 46 ... RATE 0 ... YUY2 1920x1080 org.gnome.Snapshot
  ```
- FACT：`wpctl status` 确认真实 Snapshot PID `11151` 的 `org.gnome.Snapshot / input_1` 连在 `BACK:capture_1 [active]`；`pw-link -l` 双向链路都存在。
- FACT：PipeWire PID `11003` 和 `pw-data-loop` 线程当时都在 `do_epoll_wait`；helper PID `29913` 保留 19 threads、15 个 1920x1080 ImageReader dmabuf 和 ashmem fd，说明 Camera2 session 仍活着。
- FACT：当前 `husk-pipewire.log` 只有一次 `STREAMON: ERR busy`；5 秒后 SPA 重配，后续节点进入 `suspended -> running`，但没有帧率。helper logcat 只剩 `capture|failed|count=210|frame=125|reason=1`，与 ImageReader 缓冲无人及时消费一致。
- 推断：第一次 busy 是 SPA 短视频/重配与 helper 异步 session teardown 的竞态；后续 `RATE=0` 是 V4L2 façade 的事件/缓冲驱动路径没有继续触发 `DQBUF`，不能仅用 Camera2 出帧成功解释。
- 下一步：给 `v4l2-shim` 增加通用 ioctl/mmap 审计，重点记录 `REQBUFS`、`QBUF`、`STREAMON`、首个 `DQBUF`、`STREAMOFF`；重编并只部署本轮测试资产后复测。禁止为 Snapshot 增加特例。

### 16.72 已加入通用 V4L2 缓冲审计并完成 loader/preload 构建（2026-11-28）

- 代码：`runtime/crates/v4l2-shim/src/lib.rs` 增加有界审计，仅在 `NR_V4L2_AUDIT=1` 时输出；记录 `MMAP`、`REQBUFS`、前 8 次及每 30 次 `QBUF`/`DQBUF`、`STREAMON`、`STREAMOFF`、设备 `destroy`。没有 Snapshot 特例。
- 工具链：`NDK=/Users/l/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64`；`make loader preloads` PASS。
- 新构建身份：
  ```text
  libld.so 1066496 bytes sha256=f2591622141554286b9da939a0afed5044ee6fb33f728298ab1f4180e16ad465
  libhusk_v4l2.so 200600 bytes sha256=b824e5bbf5f9f8a64ff10198ff6d6dab84166203961b7c4faba94ced74e8c991
  ```
- 设备旧身份：
  ```text
  libld.so sha256=0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94
  libhusk_v4l2.so sha256=ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f
  ```
- 下一步：关闭 Snapshot 和媒体栈，部署这两个 debug 资产并各自保留旧副本；重启 helper/media 后走同一 `.desktop`，用新审计定位事件路径。

### 16.73 debug 资产已部署，helper 与媒体栈干净重启（2026-11-28）

- 操作：终止旧 Snapshot `11151`、PipeWire `11003`、WirePlumber `11015`、helper `29913`；旧 loader/preload 分别保留为 `*.pre-16.72`。
- FACT：设备上的新身份与构建产物一致：
  ```text
  files/linux/husk/libld.so
  f2591622141554286b9da939a0afed5044ee6fb33f728298ab1f4180e16ad465
  files/linux/distributions/ubuntu-24.04/rootfs/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so
  b824e5bbf5f9f8a64ff10198ff6d6dab84166203961b7c4faba94ced74e8c991
  ```
- FACT：第一次 helper 重启因未设置 `HUSK_FILES` panic；补齐 `HUSK_FILES`/`HUSK_ROOTFS` 后 helper PID `12139` 启动，`run/husk/camera.sock` ready。
- FACT：部署 `husk-media-session.sh` 后用完整桌面等价环境启动：PipeWire PID `12283`、WirePlumber PID `12299`，二者 ready；PipeWire/WirePlumber 均带 `NR_V4L2_AUDIT=1`。
- 下一步：从同一 Snapshot `.desktop` 启动，读取新审计的 `MMAP/REQBUFS/QBUF/DQBUF/STREAMON`。

### 16.74 新审计未命中 V4L2：残留 selftest 延迟 camera.sock，媒体栈抢先枚举（2026-11-28）

- FACT：新 helper `12139` 启动日志显示，因为 `files/camera-selftest` 仍存在，启动时先执行自测：
  ```text
  camera|selftest|DONE|frames=3|fps=20.00|span_ms=99.99|elapsed_ms=325
  camera|session|closed
  camera|LISTEN|.../run/husk/camera.sock
  ```
- FACT：该轮 PipeWire PID `12283` 在 helper 创建 listener 之前已经启动并完成设备枚举；新 `husk-pipewire.log` 没有 `[v4l2] MMAP/REQBUFS/QBUF/DQBUF`，`pw-top` 只剩 Dummy/Freewheel driver，没有 V4L2 input 与 Snapshot 节点。
- FACT：Snapshot PID `12344` 仍存活，但媒体图里没有相机，不能作为相机失败证据；本轮属于测试基线竞态。
- 判定：必须移除 `camera-selftest` 测试残留，并固定启动顺序为 helper `camera.sock` ready → PipeWire/WirePlumber 枚举出 BACK/FRONT → 再走 `.desktop` 启动 Snapshot。正式桌面入口也要按这个依赖顺序实现，不能靠 sleep 猜测。
- 下一步：受控停止 Snapshot/media/helper，移除 selftest marker，按依赖顺序重启并重新抓审计。

### 16.75 清掉 selftest 后 helper 先 ready；媒体栈误用 guest root 导致 V4L2 未注册（2026-11-28）

- 操作：停止 Snapshot PID `12344`、旧 media `13728/13742`（上轮）和 helper `12139`；删除设备侧 `files/camera-selftest`；重启 helper `13662`，等待 `camera.sock` ready；随后启动媒体栈。
- FACT：helper `13662` 启动日志不再有 selftest，camera socket ready；PipeWire PID `13728`、WirePlumber PID `13742` 启动成功。
- FAIL：`wpctl status` 的 Video 区域为空；`husk-wireplumber.log` 没有 V4L2 设备注册。PipeWire 日志也没有 `[v4l2]` INFO/audit。
- 对照：之前能列出 BACK/FRONT 时，PipeWire/WirePlumber 的运行身份是 guest `husk`；本轮 `husk-session.sh` 未带 `--user husk`，媒体栈以默认 guest `root` 启动。这是启动身份不一致，不是 V4L2 façade 失败。
- 下一步：按桌面真实用户 `--user husk` 重启媒体栈，先验证 `wpctl status` 有 BACK/FRONT，再启动 Snapshot。

### 16.76 loader/preload A/B：新组合仍无 V4L2 Video（2026-11-28）

- 操作：在 helper `camera.sock` ready、以 guest `husk` 启动媒体的基线上，换回本轮新构建 loader `f2591622...` 与新 preload `b824e5bb...`，重启 PipeWire/WirePlumber 后读取 `wpctl status`。
- FAIL：Video 区域仍为空，WirePlumber 未注册 `BACK`/`FRONT`；Snapshot 继续显示 “No Camera Found”。
- FACT：该轮不能证明新 loader 或新 preload 的审计改动造成枚举回归；失败发生在 V4L2 device factory 注册前。
- 记录：每步同步写入本 evidence 与 LinuxHub 总进度；不提交 Git。

### 16.77 loader 回退 A/B：旧 loader + 新 preload 仍无 V4L2 Video（2026-11-28）

- 操作：仅将设备 loader 回退为 `0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94`，保留新 preload `b824e5bb...`，完整重启媒体栈。
- FAIL：`wpctl status` Video 区域仍为空，未恢复到 BACK/FRONT。
- 判定：当前枚举回归不由新 loader 单独触发。

### 16.78 loader + preload 全部回退 A/B：旧组合仍无 V4L2 Video（2026-11-28）

- 操作：将 loader 回退为 `0d47231e...`，preload 回退为 `ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f`，再次完整重启媒体栈。
- FACT：`/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/usr/local/bin/husk-media-session.sh` SHA-256 为 `93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5`，与已知可工作脚本一致。
- FACT：PipeWire/WirePlumber environ 均含正确 `NR_ROOTFS` 和绝对 `LD_PRELOAD`。
- FAIL：`wpctl status` Video 区域依然为空；因此暂排除最近 loader/preload 改动，回归优先指向当前 rootfs `/sys` / udev 现场或媒体启动状态。
- 当前设备恢复为旧 loader `0d47231e...` + 旧 preload `ff1b4b55...`；两个 `.pre-16.72` 备份均保留。
- 下一步：用 `WIREPLUMBER_DEBUG=5` 只启动一个 WirePlumber，确认 V4L2 enumerator 是否返回设备、是否出现旧成功日志中的 `check_access /dev/video0`，并对比完整初始化差异。

### 16.79 跨仓库进度记录补写（2026-11-28）

- FAIL：首次补写 11.78 时在 Husk 仓库内使用了相对的 `LinuxHub-2.0/...` 路径，目标不存在，总进度未写入。
- 纠正：改用绝对路径 `/Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/docs/research/linuxhub-2.0/00-GOAL_AND_PROGRESS.md`。
- 此问题只影响记录写入，不影响 16.76–16.78 的设备 A/B 事实。

### 16.80 当前媒体栈身份正确，但 V4L2 monitor 枚举为空（2026-11-28）

- FACT：`camera.sock` 存在，selftest 残留不存在；helper `13662` 存活。
- FACT：PipeWire PID `14852`、WirePlumber PID `14866` 均以 `USER=husk`、`HOME=/home/husk`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、`DISPLAY=:0`、session bus 运行。
- FACT：媒体进程实际 loader 为 `0d47231e...`（旧版），preload 为 `ff1b4b55...`（旧版），`NR_ROOTFS` 与绝对 `LD_PRELOAD` 正确；`NR_V4L2_AUDIT=1`。
- FACT：`husk-wireplumber.log` 已加载 `30-v4l2-monitor.lua` 和 `50-v4l2-config.lua`，启用 `script:monitors/v4l2.lua`，SPA 日志出现 `start_inotify: start inotify`。
- FAIL：仍没有 `api.v4l2.enum.udev`，没有 `spa.v4l2 check_access: /dev/video0`，也没有 BACK/FRONT 设备注册；`wpctl` 无法继续作为相机链路测试。
- FAIL（诊断命令）：一次在 Android 外层 shell 提前设置 guest `LD_PRELOAD`，导致 `/bin/sh` 报 `cannot locate symbol "__errno_location"`；另两条嵌套 `run-as sh -c` 因引号错误未执行。以上是命令封装错误，不是当前 V4L2 枚举结果。
- 下一步：核对 `sys-v4l2` 影子树内容与 preload 对 `/sys/class/video4linux*` 的实际重定向，再选择受控探针验证。

### 16.81 当前 V4L2 影子树结构（2026-11-28）

- FACT：`rootfs/sys/class/video4linux`、`rootfs/sys/bus/video4linux`、`rootfs/sys/subsystem/video4linux` 均为空目录。
- FACT：`rootfs/sys-v4l2` 存在且包含 `bus`、`class`、`subsystem` 子树；当前设计依赖 preload/path-shim 将标准 sysfs 路径重定向到该影子树。
- 下一步：检查 `camera-v4l2-tree` 生成内容、preload 的 `opendir/readlink/stat/access` 覆盖，以及 WirePlumber 的 libudev 枚举是否实际经过这层重定向。

### 16.82 媒体栈实际运行身份分裂：PipeWire/WirePlumber 用了旧 bundle loader（2026-11-28）

- FACT：`/proc/14852/exe` 与 `/proc/14866/exe` 都是 `/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/libld.so`。
- FACT：该 loader SHA-256 为 `67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`；桌面/helper 使用的 `files/linux/husk/libld.so` SHA-256 为 `0d47231e8f2e172f7fe4d80ec50f9be0f8b9ca3cd99b74fb5adee8e155026b94`。
- FACT：PipeWire/WirePlumber 的 `NR_LD` environ 也指向 distribution 旧 loader，不是 husk loader。此前 16.76–16.78 的“旧 loader 回退”只验证了 `husk/libld.so`，没有覆盖这个 distribution loader。
- 判定：当前媒体枚举结果属于 **错误 bundle loader 现场**，不能用来否定新 V4L2 改动；必须让桌面媒体入口只使用当前 runtime 的 loader。
- 下一步：检查 `husk-desktop.sh`、`husk-session.sh`、`husk-media-session.sh` 的 `NR_LD/BASE` 解析，定位媒体子进程为何继承 distribution 旧的 loader，并做通用修复。

### 16.83 影子树内容与 helper 生成已核实（2026-11-28）

- FACT：`sys-v4l2/class/video0|video1`、`sys-v4l2/subsystem/video4linux/devices/video0|video1`、`sys-v4l2/bus/video4linux/devices/video0|video1` 均存在。
- FACT：`video0/name=BACK`、`video1/name=FRONT`，`uevent` 含 `MAJOR=81`、`DEVNAME=videoX`，`run/udev/data/c81:*` 已生成。
- FACT：`adb logcat` 有 `Husk: camera|v4l2|tree|devices=2`。
- 结论：当前枚举为空不是影子树缺失，而是媒体进程用旧 loader 的路径解析 / 枚举路径没有命中该树；先修运行身份再复测。

### 16.84 旧 bundle-loader 媒体栈已受控停止（2026-11-28）

- 操作：使用显式 `NR_LD=files/linux/husk/libld.so` 的测试环境调用 `/usr/local/bin/husk-media-session stop`。
- FACT：脚本报告 stopped WirePlumber PID `14866`、stopped PipeWire PID `14852`。
- 停止后进程核对：
```
<empty>
```
- 下一步：同一正确 loader 环境启动媒体栈，核对 exe/environ 与 `wpctl status`。

### 16.85 正确 husk loader 下 V4L2 枚举恢复（2026-11-28）

- 操作：停止旧 bundle-loader 媒体栈后，使用显式 `NR_LD=files/linux/husk/libld.so`、guest `husk`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、现有 session bus 和 `NR_V4L2_AUDIT=1` 启动 `/usr/local/bin/husk-media-session start`。
- FACT：PipeWire PID `15282`、WirePlumber PID `15293` 均 ready；`/proc/<pid>/exe` 均为 `files/linux/husk/libld.so`，`NR_LD` 也一致。
- FACT：两进程仍加载 `rootfs/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so`，environ 保留 `XDG_RUNTIME_DIR=/tmp/runtime-husk`、session bus、`NR_V4L2_AUDIT=1`。
- FACT：`wpctl status` 的 Video 区域恢复：
  ```text
  Devices:
    39. BACK  [v4l2]
    40. FRONT [v4l2]
  Sources:
  * 36. BACK (V4L2)
    42. FRONT (V4L2)
  ```
- 判定：16.80 的空 Video 是错误 bundle loader 现场；正确 runtime loader 可完成 V4L2 sysfs 枚举。下一步用同一桌面入口启动 GNOME Snapshot，验证真实取流而非只认 `wpctl`。

### 16.86 官方入口已识别相机，但 Snapshot 取流遇到 STREAMON busy（2026-11-28）

- 操作：在正确 loader 媒体栈 ready、`wpctl` 已列出 BACK/FRONT 后，通过 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 走正式 `Exec=/usr/local/bin/doro-gpu-run snapshot`。
- FACT：Snapshot 日志确认 `gstpipewiredeviceprovider` 发现节点 `29/30/36/42`，随后建立 `snapshot-46 -> v4l2_input._sys_class_video4linux_video0` 链路。
- FAIL：首次 `Spa:Pod:Object:Command:Node:Start` 后立即报 `res:-16 (设备或资源忙): Start error: 设备或资源忙`；`spa.v4l2` 报 `/dev/video0 VIDIOC_STREAMON: 设备或资源忙`。
- FACT：helper `13662` 的 Camera2 session 同时继续报 `Camera3 output buffer timeout` 和 `Device error code 3`，最终 `camera|capture|failed|count=60|frame=74|reason=1`。
- FACT：`wpctl`/WirePlumber 的枚举与默认 Video Source 正常，当前问题已从“发现不到相机”推进到“首个 Camera2 会话没有被数据消费端正确推进”。
- FAIL（诊断命令）：`adb exec-out screencap` 卡住，未生成 `/tmp/husk-snapshot-16.86.png`；本轮不能写“有截图”，只能记录 Snapshot 进程日志现场。
- 下一步：部署本轮带 `REQBUFS/QBUF/DQBUF` 审计的新 preload，在同一链路抓首次 buffer 时序；同时核对 helper 的 CameraSession owner/OPEN/STOP 生命周期。

### 16.87 helper 当前仍持有旧 preload 身份（2026-11-28）

- FACT：当前媒体进程加载的 `libhusk_v4l2.so` SHA 仍是 `ff1b4b55...`，即 16.72 之前的旧 preload；该版本没有本轮新增的 `REQBUFS/QBUF/DQBUF` 审计。
- FACT：因此当前 `husk-pipewire.log` 能看到 INFO/STREAMON，但不能据此判定缓冲阶段没有发生。
- 下一步：只替换为已构建的新 preload `b824e5bb...`，保留 `.pre-16.72` 备份，重启媒体栈后重复同一 Snapshot 入口。

### 16.88 根因收敛：STOP 应答早于 Camera2 session 释放（2026-11-28）

- FACT：PipeWire 日志显示首个 `snapshot-46` 探测节点在 `04:31:35.72` 建立、`04:31:35.73` Start 成功，`04:31:36.38` 被销毁；随后正式 `org.gnome.Snapshot-46` 在 `04:31:36.45` Start。
- FACT：正式流 Start 时 helper 返回 `ERR busy`，此时距首流销毁约 70ms；不是首次枚举，也不是 `/dev/video0` 打不开。
- 代码证据：`app-rs/src/camera.rs` 的 `STOP` 分支先写 `BYE\n` 再 `break`；`CameraSession::close(api)` 和 `session_owner` 释放发生在这之后。V4L2 shim 收到 BYE 就认为 STREAMOFF 完成，于是紧接着的第二次 STREAMON 会撞上尚未释放的全局 `CAMERA_SESSION_OWNER`。
- 判定：这是通用的 V4L2 STOP/OPEN 生命周期竞态，不是 Snapshot 特例。正式流被误判 busy，进而导致 Camera2 输出缓冲无人消费和 timeout。
- 修复：STOP 命令不再提前应答；先关闭 session、释放 owner，再回 `BYE`。不增加 Snapshot 判断，不改变 IPC 命令格式。
- 下一步：改源码、格式检查、构建并上机复测同一 `.desktop`。

### 16.89 STOP 生命周期修复已写入源码（2026-11-28）

- 修改：`app-rs/src/camera.rs` 的 `handle_client` 增加 `stop_requested`；收到 STOP 后先退出命令循环，执行 `CameraSession::close(api)`、释放 `CAMERA_SESSION_OWNER`，最后才发送 `BYE\n`。
- 范围：只改通用 camera.sock STOP/OPEN 生命周期，未增加 Snapshot 名称/进程判断，未改 IPC 格式。
- FAIL（命令）：首次在 Husk 仓库根目录执行 `cargo fmt --all --check`，因根目录没有 `Cargo.toml` 失败；不是代码错误。
- 下一步：分别对 `app-rs` 和 `runtime` 两个 Cargo workspace 做格式检查、测试与 Android helper 构建。

### 16.90 源码格式检查通过（2026-11-28）

- `cargo fmt --manifest-path app-rs/Cargo.toml -- --check` PASS。
- `cargo fmt --manifest-path runtime/Cargo.toml --all -- --check` PASS。
- 代码改动仍在未提交工作区；下一步构建 `husk-bridges` Android helper 与 preload。

### 16.91 helper 与 preload 构建通过（2026-11-28）

- 工具链：NDK `28.2.13676358`，target `aarch64-linux-android`。
- `cargo build --release --manifest-path app-rs/Cargo.toml --target aarch64-linux-android --bin husk-bridges` PASS（既有 warnings，无 errors）。
- `make NDK=<ndk>/bin preloads` PASS；`preload exports OK`。
- 本轮产物：
  ```text
  husk-bridges 4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  libhusk_v4l2.so b824e5bbf5f9f8a64ff10198ff6d6dab84166203961b7c4faba94ced74e8c991
  ```
- 下一步：关闭 Snapshot/媒体/helper，备份设备旧 helper，部署这两个新产物；camera.sock ready 后重启媒体并核对 BACK/FRONT。

### 16.92 复测前清场完成（2026-11-28）

- FACT：停止前 helper `13662` 的环境为 `HUSK_FILES=/data/user/0/com.linuxhub.myth.husk/files`、`HUSK_ROOTFS=.../distributions/ubuntu-24.04/rootfs`；没有残留 Snapshot 进程。
- FACT：旧设备 helper SHA `c44f23f97618224d655ba4d9c75ff17a4891fda7d98fb0fb06a25a6c1a54ab3f`。
- 操作：停止媒体栈 WirePlumber `15293`、PipeWire `15282`，再停止 helper `13662`。
- 下一步：备份旧 helper/preload，部署新 helper `4e039660...` 与 preload `b824e5bb...`，等待新 camera.sock ready。

### 16.93 新 helper/preload 已部署并 ready（2026-11-28）

- FACT：设备 helper `files/linux/runtime/husk-bridges.binder-pool` SHA `4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f`，与构建产物一致。
- FACT：设备 preload `rootfs/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so` SHA `b824e5bbf5f9f8a64ff10198ff6d6dab84166203961b7c4faba94ced74e8c991`。
- FACT：旧文件已保留为 `husk-bridges.binder-pool.pre-16.91` 和 `libhusk_v4l2.so.pre-16.91`。
- FACT：新 helper PID `16138` 由 `setsid` 启动，`camera.sock` 已重建；启动日志为：
  ```text
  husk-bridges|start|files=/data/user/0/com.linuxhub.myth.husk/files|rootfs=.../distributions/ubuntu-24.04/rootfs
  ```
- 下一步：用 `NR_LD=husk/libld.so` 启动媒体栈，确认 WP 枚举仍为 BACK/FRONT，并观察新 preload 的 buffer audit。

### 16.94 新 helper + 新 preload 媒体栈 ready（2026-11-28）

- FACT：PipeWire PID `16169`、WirePlumber PID `16180` 启动并 ready，均使用 `husk/libld.so`、新 preload `b824e5bb...`。
- FACT：`wpctl status` 的 Video 区域列出 BACK/FRONT 两个 V4L2 Device 和两个 Source。
- 下一步：正式 `.desktop` 启动 Snapshot，读取新 preload 的 REQBUFS/QBUF/DQBUF 审计并验证画面。

### 16.95 新 helper 后首次 `.desktop` 启动未留下 Snapshot 进程（2026-11-28）

- 操作：媒体栈 ready 后通过 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 启动；启动器命令本身退出。
- FAIL：8 秒后没有 `usr/bin/snapshot` 进程；`/tmp/husk-pipewire-16.94.log` 仅出现旧 probe 的 `destroy`，没有新 `REQBUFS/QBUF/DQBUF/STREAMON`。
- FACT：`/tmp/husk-snapshot-launch-16.94.log` 为空，不能据此判断应用是否已经进入主循环。
- 下一步：检查 `doro-gpu-runtime.log`、`snapshot-run.log`、logcat 和 `gio launch` 退出码，确认是启动环境、DBus 还是应用提前退出。

### 16.96 STOP 修复后 EBUSY 消失，但 Snapshot 仍黑屏（2026-11-28）

- FACT：新 helper `16138` + 新 preload `b824e5bb...` 下，正式流没有再次报 `ERR busy`；probe 和 `org.gnome.Snapshot` 都进入 running。
- FACT：新审计出现：
  ```text
  REQBUFS ×3
  QBUF ×8
  STREAMON
  DQBUF
  ```
- FACT：`/tmp/husk-snapshot-16.96.png` 已成功保存，截图肉眼确认 Snapshot 窗口和控件正常，但预览区纯黑并转圈。
- FAIL：helper 在约 30 秒后报 `camera|capture|failed|count=30|frame=44|reason=1`，说明 Camera2 输出缓冲仍未被持续消费。
- 判定：STOP 竞态修复解决了 probe→正式流的 EBUSY，但相机门禁仍为 FAIL；下一问题是 STREAMON 后首个 DQBUF 未持续完成。

### 16.97 日志缓冲不足导致完整 helper 生命周期被冲掉（2026-11-28）

- FACT：`adb logcat -d -s Husk:V` 只剩 `capture|failed`，没有 `session|open`、`image_reader|available`、`frame|ready` 等历史行。
- 判定：Android logd 默认环形缓冲被 GStreamer/PipeWire 调试日志覆盖，不能据此推断这些事件未发生。
- 下一步：先把 logcat 缓冲扩到 16MB，再清理并重启同一 Snapshot 路径，抓完整 helper 日志。

### 16.98 断点恢复：DQBUF 可能早于 STREAMON，当前相机门禁仍 FAIL（2026-11-28）

- FACT：完整 helper 日志中已有 `camera|session|open`、`callback|session_active`、`capture|started|count=30`、`image_reader|available|count=30`、`capture|completed|count=30`，但没有 `camera|frame|ready`。
- FACT：V4L2 audit 中观察到 `DQBUF` 早于 `STREAMON` 的时间线；`v4l2-shim::dqbuf()` 在 `!streaming` 时直接返回 `EINVAL`，之后未看到第二个 `DQBUF`，也没有 helper `FRAME` 事务。
- FACT：当前 Snapshot PID `16618`，PipeWire 当前连接 `46. org.gnome.Snapshot -> 47. input_1 < FRONT:capture_1 [active]`；`pw-top` 中 video0/video1/Snapshot 均为 `RATE 0`。
- FACT：`/tmp/husk-snapshot-16.96.png` 肉眼确认窗口正常但预览区黑屏转圈，因此相机门禁仍是 **FAIL**，不能按“桥通过”冒充应用可用。
- 下一步：先核对 PipeWire 1.0.5 SPA V4L2 的真实 DQBUF/STREAMON 调用契约，再加有界审计记录 `streaming/nbuffers/queued/errno`，按证据做通用修复，不把 DQBUF 简单改成假成功。

### 16.99 开始核对 PipeWire 1.0.5 SPA V4L2 调用契约（2026-11-28）

- 目标：确认 SPA 的 `DQBUF -> STREAMON` 是合法调用、错误重试或 shim 语义偏差；修的是通用 V4L2 façade，不做 Snapshot 特例。
- 同时审计 helper `camera|frame|ready` 缺失点：Camera2 ImageReader available 后，到 helper 成功 acquire/转换并发送 `FRAME\n` 的哪一段中断。
- 本步只读取源码/运行日志并记录证据，未改默认性能路径。

### 16.100 PipeWire 1.0.5 源码契约已确认；异常时序需要 shim 侧审计（2026-11-28）

- FACT：设备包版本为 `pipewire 1.0.5-1ubuntu3.3`、`gstreamer1.0-pipewire 1.0.5-1ubuntu3.3`、`wireplumber 0.4.17-1ubuntu4.1`。
- FACT：PipeWire 1.0.5 `spa/plugins/v4l2/v4l2-utils.c` 的 Start 路径固定为 `VIDIOC_STREAMON` → `spa_loop_add_source(fd)`；`v4l2_on_fd_events()` 在 fd 可读后调用 `mmap_read()`，后者才发 `VIDIOC_DQBUF`。SPA 不会主动先 `DQBUF`。
- FACT：当前 shim 的 `DQBUF` 入口只记录一行，不记录设备号、`streaming`、队列状态或 EINVAL 分支；因此异常顺序的具体触发者仍不能定案。
- 判定：先加 `NR_V4L2_AUDIT=1` 下的有界审计，确认是同一设备的旧 source 回调、并发 fd 状态，还是 memfd 可读性/事件源语义偏差。不能把 `DQBUF` 简单改成成功，也不能绕过 `STREAMON`。
- 下一步：修改 `runtime/crates/v4l2-shim/src/lib.rs`，只在审计模式输出前 8 次/周期采样的入口与分支结果；格式检查和 CI 后构建新 loader/preload，上机重跑真实 Snapshot。

### 16.101 V4L2 DQBUF/STREAMON 有界审计已写入源码（2026-11-28）

- 修改：`runtime/crates/v4l2-shim/src/lib.rs` 在 `NR_V4L2_AUDIT=1` 时记录前 16 次及周期采样的 `dev/streaming/nbuffers/queued`，并区分 `bad_type/not_streaming/no_queued/send FRAME/NOFRAME/ok` 等返回路径。
- `STREAMON` 审计增加设备号与成功/失败结果；所有格式化仅在 `trace=true` 时执行，生产默认路径不做额外输出。
- 验证：`cargo fmt --manifest-path runtime/Cargo.toml --all -- --check` PASS。
- 下一步：运行 `v4l2-shim` 单测和 `make loader preloads`，确认审计分支不影响现有 ioctl 行为。

### 16.102 主机目标单测失败：AArch64 专用汇编不能在本机编译（2026-11-28）

- FAIL（命令）：`cargo test --manifest-path runtime/Cargo.toml -p v4l2-shim`。
- 原样错误：`invalid register x8/x0/x1/x2/x3/x4/x5: unknown register`，来自 `crates/v4l2-shim/src/lib.rs` 的 AArch64 `svc` 内联汇编。
- 判定：测试目标错误。该 shim 本来就只面向 guest AArch64；下一步改用仓库 Makefile 的交叉目标与环境，不把这次主机编译失败算代码回归。

### 16.103 v4l2-shim 交叉编译与 loader/preload 构建通过（2026-11-28）

- `cargo test --target aarch64-unknown-linux-gnu -p v4l2-shim --no-run` PASS；仅保留既有 warnings，无 errors。未把“能链接测试二进制”冒充为已执行单测。
- `make NDK=... GUEST_SYSROOT=/tmp/nr-toolchain/root loader preloads` PASS。
- `preload exports OK`，包括 `ioctl@GLIBC_2.17`、`fstat@GLIBC_2.17`、`husk_v4l2_*` 等必需符号。
- 下一步：计算新 loader/preload SHA，部署到测试宿主，开 `NR_V4L2_AUDIT=1` 重跑同一 Snapshot 路径。

### 16.104 新审计构建产物身份已固定（2026-11-28）

- loader：`app/app/src/main/jniLibs/arm64-v8a/libld.so` SHA `2f1e8038da2748743396150da68a5450836d732e5e70b3f7a1d602fe92d2019e`（1,066,496 B）。
- preload：`runtime/target/aarch64-unknown-linux-gnu/release/libhusk_v4l2.so` SHA `903bc98fbdaa39c6239ce7c87f68a0121f223919d3c3919b73d08215c83be947`（200,600 B）。
- FACT：设备当前 Snapshot/PipeWire/WirePlumber 均未运行，仅 helper PID `16138` 存活；因此本轮可替换 loader/preload 而不影响活动媒体流。
- 下一步：通过 `run-as` 备份并部署这两个文件，SHA256 核对后再启动 helper/媒体栈。

### 16.105 loader 热替换失败：helper 映射导致 ETXTBSY（2026-11-28）

- FAIL：`cp ... files/linux/husk/libld.so: Text file busy`。两个新文件已 push 到 `/data/local/tmp`，但 app 沙箱内的 loader 尚未替换。
- 推断：helper PID `16138` 仍加载旧 `libld.so`，文件被映射时不能覆盖写入。
- 下一步：先停 helper，确认 guest 桌面/媒体/Snapshot 无残留使用者，再备份并部署；部署成功后以设备 SHA256 为准。

### 16.106 loader 占用范围已确认（2026-11-28）

- FACT：PID `16618` 为 Snapshot，PID `16169/16180` 为 PipeWire/WirePlumber，另有 GNOME/DBus/gvfs 等大量 guest 进程都映射 `files/linux/husk/libld.so`。
- FACT：三者环境均为 `NR_LD=.../husk/libld.so`、`NR_ROOTFS=.../distributions/ubuntu-24.04/rootfs`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、`NR_V4L2_AUDIT=1`。
- 判定：只停 helper 不够；需要在保留宿主 App/X server 的前提下结束 Husk guest 会话，替换 loader 后从唯一 App 入口重开。
- 下一步：读取宿主公开 activity/进程入口，按顺序关闭 Snapshot、媒体栈和 guest 会话，再部署并核对 SHA。

### 16.107 新 loader/preload 已部署并 SHA256 对上（2026-11-28）

- FACT：停止 helper 与全部 guest `libld.so` 进程后，`pidof libld.so` 和 `pidof husk-bridges.binder-pool` 均为空；宿主 `com.linuxhub.myth.husk` PID `20925` 与 `:x11` PID `22724` 保留。
- 备份：`libld.so.pre-16.107`、`libhusk_v4l2.so.pre-16.107`。
- FACT：设备 SHA 与构建产物一致：
  ```text
  libld.so        2f1e8038da2748743396150da68a5450836d732e5e70b3f7a1d602fe92d2019e
  libhusk_v4l2.so 903bc98fbdaa39c6239ce7c87f68a0121f223919d3c3919b73d08215c83be947
  ```
- 下一步：从宿主 App 的公开入口重开 Husk 会话，先验证新 loader 身份，再启动媒体栈和 Snapshot。

### 16.108 停止 guest 会话后宿主进入 GNOME 退出页（2026-11-28）

- FACT：`am start` 将 MainActivity 拉到前台后截图 `/tmp/husk-app-after-deploy-16.107.png` 显示“GNOME 已退出，退出码 0”和“返回 LinuxHub”按钮。
- 判定：这是预期的会话结束状态，不是新 loader 启动失败；下一步点公开按钮返回 Husk 首页并从 Husk 入口重开 Ubuntu。

### 16.109 宿主首页已恢复，但残留“桌面需要检查”状态（2026-11-28）

- FACT：点击“返回 LinuxHub”后截图 `/tmp/husk-app-home-16.109.png` 显示 Husk 首页、Ubuntu 24.04/Husk/7 个硬件端点，以及“桌面需要检查”状态。
- 判定：宿主 UI 对刚被结束的会话仍保留“Linux 已经在这里运行”的旧文案；实际 guest `libld.so` 进程当前为空，必须以重新进入后的真实进程身份为准。
- 下一步：点击首页公开的“桌面”入口重新启动 Ubuntu，等待 guest 进程出现后核对 loader SHA/maps。

### 16.110 点击首页桌面图标没有拉起会话（2026-11-28）

- FAIL：点击坐标 `(1318,655)` 的桌面图标后等待 12 秒，`pidof husk-bridges.binder-pool` 与 `pidof libld.so` 仍为空；截图 `/tmp/husk-desktop-relaunch-16.110.png` 没有进入桌面。
- 判定：该图标不是启动动作，或宿主处于陈旧“运行中”状态拦截了启动；下一步读取当前窗口的可访问性节点，找到真正的公开点击目标，不猜坐标。

### 16.111 可访问性节点找到真实“打开桌面”入口（2026-11-28）

- FACT：`uiautomator dump` 显示公开节点 `content-desc="打开桌面"`，bounds `[1488,713][1651,876]`；上一轮点击的 `(1318,655)` 实际落在“设置”附近，不是启动动作。
- 下一步：点击“打开桌面”节点中心 `(1569,794)`，等待会话并核对 helper/loader 身份。

### 16.112 “打开桌面”已拉起 guest 进程，但 15 秒截图仍黑屏且 helper 未启动（2026-11-28）

- FACT：点击 `(1569,794)` 后 guest `libld.so` PID 已大量出现（首枚 `17623`），说明新会话确实启动。
- FAIL：15 秒截图 `/tmp/husk-desktop-relaunch-16.112.png` 全黑；`pidof husk-bridges.binder-pool` 为空。
- 判定：现在不能证明桌面成功，也不能做相机验收。先读 `husk-desktop.status/trace` 和进程状态，确认是仍在首帧等待、会话失败，还是宿主没有拉起 helper。

### 16.113 新会话桌面已正常出现，helper 以 `husk-bridges` 启动（2026-11-28）

- FACT：等待约 27 秒后截图 `/tmp/husk-desktop-relaunch-16.113.png` 显示完整 GNOME 桌面（顶栏、Dock、桌面图标、壁纸），可视为首帧与 WM 已起来。
- FACT：宿主实际启动的 helper 进程名是 `husk-bridges` PID `17620`，不是手动测试时改名的 `husk-bridges.binder-pool`；此前 `pidof husk-bridges.binder-pool` 为空并不代表 helper 未启动。
- 下一步：核对 `17620` 的 exe/maps/SHA 与 `libld.so` 身份，确认宿主运行的是本轮新 runtime，再检查媒体栈是否由 `husk-desktop.sh` 自动拉起。

### 16.114 宿主重开会话时把 helper/preload 覆盖回旧 runtime（2026-11-28）

- FACT：桌面成功后再次核对设备 SHA：
  ```text
  husk/libld.so                              2f1e8038...（本轮新 loader，保留）
  rootfs/.../libhusk_v4l2.so                ff1b4b55...（APK 旧 preload，不是本轮 903bc98f...）
  runtime/husk-bridges                      c0663333...（APK 旧 helper，不是本轮 4e039660...）
  ```
- FACT：宿主启动的 helper PID `17620` exe 为 `files/linux/runtime/husk-bridges`，camera.sock 已 ready；GNOME PID `17623` 由本轮 `husk/libld.so` 启动。
- 判定：宿主重开桌面时从 APK 生成/覆盖 runtime，直接把新版 helper 和 preload 换回旧版。当前会话不能用于验证 STOP 修复和 DQBUF 审计。
- 下一步：保留宿主/X11/桌面，停止旧 helper，部署新 helper；停止媒体后用设备现有测试入口重启 PipeWire/WirePlumber，并再次核对三者 SHA。最终必须把该入口迁回 Husk 源码，不能依赖临时脚本。

### 16.115 新 helper 与审计 preload 已重新部署并启动（2026-11-28）

- 操作：停止旧 helper PID `17620`，备份为 `runtime/husk-bridges.pre-16.115` 与 `libhusk_v4l2.so.pre-16.115`，部署本轮产物。
- FACT：设备 SHA 已对上：
  ```text
  runtime/husk-bridges                      4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  rootfs/.../libhusk_v4l2.so                903bc98fbdaa39c6239ce7c87f68a0121f223919d3c3919b73d08215c83be947
  ```
- 新 helper PID `18515`，启动日志为 `husk-bridges|start|files=...|rootfs=...`，`camera.sock` 已 ready。
- 下一步：使用 `guest-correct.sh` 的正确 loader/env 启动媒体栈，核对 `wpctl` 与进程环境后再启动 Snapshot。

### 16.116 新 helper 下媒体栈已 ready，但首次枚举只看到 Devices（2026-11-28）

- FACT：`husk-media-session start` 返回 PipeWire PID `18547`、WirePlumber PID `18565`，两者均 `ready`。
- FACT：首次 `wpctl status` 的 Video 区列出 `39. BACK [v4l2]`、`40. FRONT [v4l2]`；Sources 暂为空，不能立即判定枚举完全稳定。
- 下一步：等待 WirePlumber monitor 完成，重新查 Sources；同时核对 `18547/18565` 的 `NR_LD`、`LD_PRELOAD` 和 preload SHA，再决定启动 Snapshot。

### 16.117 已定位 Sources 为空：设备上的媒体启动脚本是旧版，PipeWire 未加载 preload（2026-11-28）

- FACT：源码 `tools/guest/husk-media-session.sh` SHA `93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5`；设备 `/usr/local/bin/husk-media-session` SHA `bff08695a37bf385a588c746ca2753eef6c1f1f428902fa8e0da29322cb469be`，不是同一版本，且不含当前 `LD_PRELOAD` 启动行。
- FACT：PipeWire PID `18547` environ 无 `LD_PRELOAD`，maps 中 `libhusk_v4l2.so` 映射数为 `0`；WirePlumber PID `18565` 有 preload、映射数为 `7`。因此只有 WirePlumber 看到 V4L2 Device，PipeWire 没有对应的 Source。
- 判定：Sources 为空不是 shim 又一次失败，而是设备跑的是旧媒体启动脚本。先部署源码版脚本，再重启媒体栈复测。

### 16.118 媒体脚本替换组合命令引号失败，未改动设备（2026-11-28）

- FAIL：一条组合 `adb shell run-as` 命令返回 `/system/bin/sh: no closing quote`，因此 stop/备份/复制没有作为一个整体成功执行；不能声明设备脚本已更新。
- 下一步：拆成独立命令执行 stop、push、备份复制和 SHA 核对，避免嵌套引号导致部分动作状态不明。

### 16.119 stop 成功，但同一外层命令的后续检查仍被 shell 展开污染（2026-11-28）

- FACT：旧媒体脚本已报告 `stopped wireplumber pid=18565`、`stopped pipewire pid=18547`，停止动作本身成功。
- FAIL：同一条外层命令中的 `pidof` 被主机 zsh 提前展开为 `command not found`，并产生 `/Users/.../adb: inaccessible`；后续检查无效。
- 纠正：后续每步只发一条 `adb` 命令，设备内检查放到单独调用，不再把嵌套命令塞进同一字符串。

### 16.120 源码版媒体脚本已部署，媒体栈重新 ready（2026-11-28）

- FACT：停止确认 PipeWire/WirePlumber 为空、helper `18515` 保持运行；部署源码版脚本后设备 SHA 为 `93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5`，旧版备份为 `.pre-16.118`。
- FACT：`media-reload-correct.sh start` 返回 PipeWire PID `18697`、WirePlumber PID `18708`，均 ready。
- 下一步：核对两个进程均有 `LD_PRELOAD`/`libhusk_v4l2.so` 映射，并确认 `wpctl` Video Sources 恢复。

### 16.121 两个媒体进程均已吃上本轮 preload，Video Sources 恢复（2026-11-28）

- FACT：PipeWire `18697` 与 WirePlumber `18708` 均含 `LD_PRELOAD=.../libhusk_v4l2.so`、`NR_LD=.../husk/libld.so`、`NR_ROOTFS=.../ubuntu-24.04/rootfs`、`NR_V4L2_AUDIT=1`，各自 `libhusk_v4l2.so` 映射数为 `7`。
- FACT：`wpctl status` Video 区恢复 BACK/FRONT 两个 Device 与 BACK/FRONT 两个 Source。
- 下一步：清空 Android logcat、记录 PipeWire 日志当前行号，通过正式 `org.gnome.Snapshot.desktop` 启动应用并等待真实首帧。

### 16.122 Snapshot 审计确认根因：memfd 不能进 epoll，SPA 退化到 idle 后不再触发（2026-11-28）

- FACT：新 helper + 新 preload 的正式 Snapshot 审计连续出现：
  ```text
  [v4l2] DQBUF enter dev=0 streaming=0 nbuffers=8 queued=8
  [v4l2] DQBUF EINVAL not_streaming
  [v4l2] STREAMON ok dev=0
  [v4l2] DQBUF enter dev=1 streaming=0 nbuffers=8 queued=8
  [v4l2] DQBUF EINVAL not_streaming
  [v4l2] STREAMON ok dev=1
  ```
  之后没有第二次 DQBUF，也没有 helper `camera|frame|ready`；Camera2 在 30 秒后继续报 output timeout。
- FACT：SPEC/源码链路为 `STREAMON → spa_loop_add_source(fd) → epoll_wait → DQBUF`。当前 guest fd 是 memfd（regular file），Linux 对 regular file 的 `epoll_ctl(EPOLL_CTL_ADD)` 返回 `EPERM`；PipeWire 的 `loop.c` 会把这种 fd 转成 idle fallback source。
- 判定：第一次早到的 idle `DQBUF` 返回 EINVAL 后，活跃的 PipeWire graph 不再执行 idle fallback，所以不会产生第二次 DQBUF。根因是“memfd 天然可 poll/epoll”的底层假设错误，不是 SPA 主动乱序。
- 修复方向（通用）：保留 memfd 作为 guest 可见 fd 身份，在每个 Dev 内创建内部 eventfd；preload 的 `epoll_ctl` 对 fake V4L2 fd 透明映射到 eventfd，`STREAMON` 和 `DQBUF` 成功/可重试返回时重新写事件。SPA、Snapshot、应用名均不做特例。
- 下一步：实现 eventfd 适配与导出/负向测试，构建新 loader/preload，然后重跑同一 `.desktop` 路径。

### 16.123 eventfd epoll 适配已写入源码（2026-11-28）

- 修改 `runtime/crates/v4l2-shim/src/lib.rs`：每个 V4L2 Dev 在 memfd 之外创建内部 `EFD_CLOEXEC|EFD_NONBLOCK` eventfd；`STREAMON` 成功后写入一次事件，`DQBUF` 入口消费事件，成功或 `EAGAIN` 时重新武装；`STREAMOFF`/销毁时清理。
- 修改 `runtime/crates/v4l2-preload/src/lib.rs`：导出 versioned `epoll_ctl@GLIBC_2.17`，仅对可通过 fd-identity 判定的 fake V4L2 fd 把内核监听目标透明换成内部 eventfd；普通 fd 原样透传。
- 修改 `exports.map` 与 `audit-preload-exports.sh`，把 `epoll_ctl@GLIBC_2.17` 纳入构建门禁。
- 设计口径：不修改 SPA、GNOME Snapshot 或应用名判断；不把 `DQBUF` 假成功；不增加会话、拷贝或额外设备节点。
- 下一步：交叉编译 shim 测试、构建 loader/preload，运行导出审计，再部署到设备。

### 16.124 preload 不能作为 test harness 编译（预期 no_std 限制）（2026-11-28）

- FAIL（命令）：`cargo test --target aarch64-unknown-linux-gnu -p v4l2-shim -p v4l2-preload --no-run`。
- 原样错误：`found duplicate lang item panic_impl`，`std` 的 test harness 与 preload 自带 panic handler 冲突。
- 判定：这是 no_std preload 的测试目标限制，不是 eventfd 代码错误。纠正为：shim 跑交叉 test 编译；preload 用正式 release 链接和导出审计验证。

### 16.125 双文件记账首次写入路径错误，Husk 条目未落盘（2026-11-28）

- FAIL：从 LinuxHub 仓库工作目录执行了相对路径 `docs/design/evidence/367-husk-camera-pipewire-20261128.md`，zsh 返回 `no such file or directory`，Husk 条目没有写入；同一 shell 随后仍写入了 LinuxHub `11.89`，造成两边暂时不一致。
- 纠正：Husk 记录必须使用绝对路径，或单独以 `/Users/l/Documents/Codex/2026-08-10/husk` 为工作目录执行。

### 16.126 eventfd 版构建已通过，进入上机验证（2026-11-28）

- 恢复长期任务目标并确认当前范围仍是 Husk 0.0.37：先完成相机链路与回归门禁，不进入 0.0.38，不提交 Git。
- FACT：`cargo fmt --all -- --check`、AArch64 `v4l2-shim` 测试目标编译、`make loader preloads` 均 PASS；`epoll_ctl@GLIBC_2.17` 导出审计 PASS。
- FACT：本轮产物 SHA256：
  ```text
  libld.so             354a0aa70d78857155f9abec939b9c3ea432ecd0a1217dcc629c782a442db708
  libhusk_v4l2.so      5518df7d7ca8dd4c63714f61a65bb3cd538484565f9ab7661721c668f6ab60fa
  husk-bridges         4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  ```
- 运行结论仍为 **NOT VERIFIED**：尚未部署 eventfd 版 loader/preload，也尚未走正式 Snapshot 路径。
- 下一步：核对 adb 与本轮宿主进程现场，停止旧 helper/旧 guest loader 进程，部署新产物并从公开入口重开桌面。

### 16.127 设备现场存在旧桌面/旧媒体进程，不能直接验证 eventfd 版（2026-11-28）

- FACT：adb 设备 `UKHMMB8PYTGECMNV` 在线；宿主包 `com.linuxhub.myth.husk` 已安装，`versionName=Myth-Prometheus`，`lastUpdateTime=2026-11-28 02:42:34`。
- FACT：设备仍有 GNOME PID `17623`（由 `libld.so` 启动）、多个 guest `libld.so` 子进程及残留媒体相关进程；当前会话不是本轮 eventfd 产物。
- 判定：这些旧进程必须先清场，否则无法证明新 loader/new preload maps，也不能把后续 Snapshot 结果算作本轮。
- 下一步：只通过宿主公开入口重开会话，随后部署新产物并核对运行身份；不改黑盒宿主 APK，不读其源码。

### 16.128 旧宿主/桌面/媒体会话已清场（2026-11-28）

- 操作：执行宿主公开生命周期 `am force-stop com.linuxhub.myth.husk`，随后结束仍由同一 UID 存活的旧 helper 和 guest PipeWire PID `18697`。
- FACT：force-stop 后 helper `18515`、GNOME `17623` 退出；PipeWire 以 guest loader 进程名 `libld.so` 残留，单独结束；最终按精确进程名核对无 `libld.so`、`husk-bridges`、`pipewire`、`wireplumber`。
- 下一步：从宿主公开入口启动并进入“打开桌面”，再核对 APK 重建出的 runtime 身份。

### 16.129 宿主已从公开 Activity 冷启动，公开“打开桌面”入口可定位（2026-11-28）

- FACT：`am start -W -n com.linuxhub.myth.husk/com.java.myapplication.MainActivity` 返回 `Status: ok`、`LaunchState: COLD`、`TotalTime: 1089`；首页显示 `Runtime 就绪`。
- FACT：`uiautomator dump` 找到公开节点 `content-desc="打开桌面"`，可点击子节点 bounds 为 `[1488,754][1651,917]`。
- 下一步：只在上述公开节点执行点击，不用内部 Activity/私有命令代入桌面试跑。

### 16.130 通过公开入口已重新进入桌面，旧产物覆盖现象待核对（2026-11-28）

- 操作：点击公开节点 “打开桌面” 中心 `(1570,835)`，等待 18 秒。
- FACT：出现新 helper PID `19513`，GNOME/桌面根进程 PID `19628`，并派生多个 guest `libld.so` 进程；宿主 MainActivity 仍为 top resumed。
- 判定：新会话已启动，但此时仍不能假定加载的是 `354a0aa7...`；必须核对设备 runtime 文件 SHA 与进程实际 maps。
- 下一步：只读核对 `files/linux/husk/libld.so`、rootfs 内 `libhusk_v4l2.so`、helper 的 SHA 和关键进程 maps。

### 16.131 宿主公开入口启动后把 runtime 覆盖回旧版（2026-11-28）

- FACT：设备当前 SHA 与本轮产物不一致：
  ```text
  files/linux/husk/libld.so                       2f1e8038da2748743396150da68a5450836d732e5e70b3f7a1d602fe92d2019e
  files/linux/runtime/husk-bridges                 c0663333698fc7e55b961a29fbf47c01191bc75e4726b98ae63276080b5c2d75
  rootfs/.../libhusk_v4l2.so                       ff1b4b55739fc4fbb6fe0ce4cf5c1ec6f9f7c1343c4833a348a7c22c3bacda1f
  ```
- 判定：当前 PID `19513/19628` 会话均由宿主公开流程重建出的旧 runtime 启动，不能用于 eventfd 验证；这是黑盒宿主覆盖 Husk 配置的外部阻塞，不修改宿主 APK。
- 下一步：保留宿主进程，只结束旧 helper/guest loader 进程；部署三个本轮产物到宿主实际读取路径与 staging 路径，再通过同一公开入口启动并核对是否仍被覆盖。

### 16.132 清场命令首个 PID 成功、其余 PID 因换行展开失败，随后纠正（2026-11-28）

- FAIL：把 `ps` 的多行 PID 直接传给 `run-as ... kill`，只有第一个 PID `19513` 被 kill，后续 PID 被外层 shell 当成独立命令，原样报 `19628: inaccessible or not found` 等；不能声明一次清场成功。
- 纠正：将 PID 列表用 `tr '\n' ' '` 合并为单行后重发；helper 与全部 `libld.so` 进程最终精确核对为空。
- 下一步：把本轮三个产物部署到 `files/linux/husk`、`files/linux/runtime`、rootfs 实际库路径及宿主 staging 路径，再核对 SHA。

### 16.133 部署暂存后因本机 zsh glob 失败，设备 runtime 尚未改动（2026-11-28）

- FAIL：`adb push` 三个产物到 `/data/local/tmp/husk-eventfd-16/` 均成功，但随后 `adb shell chmod 644 /data/local/tmp/husk-eventfd-16/*` 未加引号，zsh 在主机展开并报 `no matches found`；`set -e` 中止，后续备份/覆盖没有执行。
- 判定：设备 `files/linux` runtime 仍是旧版；暂存文件已存在，但不能声明部署完成。
- 纠正：远端 glob 必须整体加引号，且部署后立即核对四个目标 SHA。

### 16.134 本轮三个产物已覆盖部署，四个目标 SHA 全部对上（2026-11-28）

- 操作：先备份旧文件为 `*.pre-16.132`，再把 eventfd 版同时部署到 `files/linux/husk/libld.so`、`files/linux/husk-stage/libld.so`、`files/linux/runtime/husk-bridges`、rootfs `libhusk_v4l2.so`。
- FACT：设备回读 SHA：
  ```text
  husk/libld.so             354a0aa70d78857155f9abec939b9c3ea432ecd0a1217dcc629c782a442db708
  husk-stage/libld.so       354a0aa70d78857155f9abec939b9c3ea432ecd0a1217dcc629c782a442db708
  runtime/husk-bridges      4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  rootfs/libhusk_v4l2.so    5518df7d7ca8dd4c63714f61a65bb3cd538484565f9ab7661721c668f6ab60fa
  ```
- 下一步：保持宿主进程，重新定位并点击公开“打开桌面”，随后立刻核对文件是否再次被宿主覆盖，以及新进程实际 maps。

### 16.135 eventfd runtime 已在公开入口启动中生效（2026-11-28）

- 操作：从宿主首页公开“打开桌面”节点再次点击；点击后 2 秒回读三份目标 SHA，全部仍为本轮版本。
- FACT：新 helper PID `20629`；桌面根进程 PID `20633`，其 maps 明确加载：
  ```text
  /data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so
  ```
  helper `/proc/20629/exe` 为 `files/linux/runtime/husk-bridges`。
- 判定：本次会话已越过前一次“公开入口覆盖回旧版”的问题；运行身份与文件 SHA 一致，可以继续验证 eventfd 相机链路。
- 下一步：核对 helper camera socket 与日志，再启动源码版 media session，确认本会话 PipeWire/WirePlumber 同时映射 eventfd 版 preload。

### 16.136 helper socket、相机/录音权限与新会话身份均已核对（2026-11-28）

- FACT：rootfs 内 `/run/husk/camera.sock` 已存在并是 Unix socket；helper PID `20629` maps 指向本轮 `files/linux/runtime/husk-bridges`，并加载 Android `libcamera2ndk.so`。
- FACT：宿主包 `android.permission.CAMERA`、`RECORD_AUDIO`、精确/粗略定位、通知均为 `granted=true`；相机门禁不会被权限缺失卡住。
- 说明：`/proc/20629/environ` 被 Android procfs 权限拒绝，不能用环境变量自证；进程 maps、socket 和目标 SHA 已足够证明本轮 helper 运行。
- 下一步：执行设备上的源码版 `media-reload-correct.sh start`，核对 PipeWire/WirePlumber 的 loader、preload maps 和 `wpctl status`。

### 16.137 媒体栈 ready，但 PipeWire 未加载 preload，Sources 仍为空（2026-11-28）

- FACT：`media-reload-correct.sh start` 返回 PipeWire PID `21508`、WirePlumber PID `21533`，两者 exe 均为本轮 `files/linux/husk/libld.so`。
- FAIL：PID `21508` maps 无 `libhusk_v4l2.so`；只有 PID `21533` maps 了 `5518df7d...` preload。`wpctl status` 仅列出 BACK/FRONT 两个 v4l2 Device，Video Sources 为空。
- 根因：设备 rootfs 的 `/usr/local/bin/husk-media-session` 仍是旧版，`start_pipewire()` 没有 `LD_PRELOAD`；当前源码 `tools/guest/husk-media-session.sh` 已修复，但宿主重建 runtime 后没有带上该版本。
- 下一步：把当前源码脚本部署到 rootfs，停止并重启媒体栈，再复核 PipeWire/WirePlumber 双进程 maps 与 Sources。

### 16.138 源码版媒体脚本已部署，PipeWire/WirePlumber 双进程加载本轮 preload（2026-11-28）

- FACT：`tools/guest/husk-media-session.sh` SHA `93e2b448ff4ae3e4a9c83d76d10ce074dbf554dd49cf8a13d54feb70551466b5`，部署到 rootfs `/usr/local/bin/husk-media-session` 后设备 SHA 完全一致；旧版备份为 `.pre-16.137`。
- FACT：重启后 PipeWire PID `21635`、WirePlumber PID `21645` 的 exe 均为本轮 `files/linux/husk/libld.so`；两进程 `libhusk_v4l2.so` maps 数均为 `7`，loader maps 数均为 `6`。
- FACT：`wpctl status` Video 区恢复 BACK/FRONT 两个 Device，以及 `* 36. BACK (V4L2)`、`42. FRONT (V4L2)` 两个 Source。
- 下一步：清空 Android logcat，通过正式 `org.gnome.Snapshot.desktop` 启动应用，等待真实首帧并同时抓 V4L2/helper/pw-top/截图证据。

### 16.139 正式 Snapshot 首轮 eventfd 实测仍黑屏，DQBUF 未启动（2026-11-28）

- 操作：清空 Android logcat，通过 `./guest-correct.sh /usr/bin/gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 启动正式 Snapshot，等待并抓取 PipeWire/helper/pw-top/截图。
- FAIL：Snapshot 进程 PID `21717` 存活并加载本轮 `husk/libld.so`；窗口可见但画面为纯黑。截图 `367-snapshot-eventfd-ebusy-20261128.png`，SHA `c713a26e3d9c798e2455fc67a5138c3c0d941835e4a89104d7c18f998deb26a2`。
- FACT：前两次 `STREAMON /dev/video1` 在 back 探测流释放前返回 `ERR busy`；第三次 front 节点进入 running，但 `pw-top` 中 V4L2 与 Snapshot 的 `RATE=0`；helper 无 `camera|frame|ready`，V4L2 日志无新一轮 `DQBUF`。
- 推断（强证据）：本轮常量使用了 x86_64 编号：`eventfd2=290`、`epoll_ctl=233`；AArch64 通用 ABI 分别是 `19`、`21`。因此内部 eventfd 创建与 `epoll_ctl` 映射都没有按设计发生，SPA 仍走不可用的 fallback。
- 下一步：把两个 syscall 编号改为 AArch64 常量并写明来源，重跑格式/构建/导出审计，再部署复测；当前相机门禁仍为 **FAIL**。

### 16.140 AArch64 syscall 编号已由设备头文件证实并修正源码（2026-11-28）

- FACT：设备 guest `/usr/include/asm-generic/unistd.h` 明确给出 `__NR_eventfd2 19`、`__NR_epoll_ctl 21`；原代码的 `290` 在该头文件对应 `pkey_free`，`233` 对应 `madvise`。
- 修改 `runtime/crates/v4l2-shim/src/lib.rs`：`SYS_EVENTFD2` 从 `290` 改为 `19`，并标注禁止复制 x86_64 编号；同步修正顶部 memfd/epoll 说明。
- 修改 `runtime/crates/v4l2-preload/src/lib.rs`：`SYS_EPOLL_CTL` 从 `233` 改为 `21`，并标注 AArch64 来源。
- 下一步：执行 `cargo fmt --check`、shim AArch64 测试编译、`make loader preloads` 与导出审计；通过后计算新 SHA 并部署。

### 16.141 shim 测试首次从仓库根调用错误 linker，未得到代码结论（2026-11-28）

- FAIL：在 Husk 仓库根执行 `cargo test --manifest-path runtime/Cargo.toml ...` 时，Cargo 未采用 `runtime/.cargo/config.toml` 的 `rust-lld`，调用主机 `cc`，报 `ld: unknown options: --fix-cortex-a53-843419 ...`。
- 判定：这是 Cargo 配置发现目录错误，不是 v4l2 修改失败；`cargo fmt --check` 已 PASS。
- 纠正：进入 `runtime/` 后执行同一测试命令，使 `.cargo/config.toml` 生效。

### 16.142 仅读取 rust-lld 配置仍缺 guest libc sysroot（2026-11-28）

- FAIL：进入 `runtime/` 后测试走到 `rust-lld`，但报 `unable to find library -lgcc_s/-lutil/-lrt/-lpthread/-lm/-ldl/-lc`。
- 判定：`runtime/.cargo/config.toml` 只解决 linker，测试链接还需要 `--target=aarch64-linux-gnu --sysroot=/tmp/nr-toolchain/root`。
- 纠正：按 Makefile `TEST_LINK` 口径设置 NDK clang、目标 triple 和 guest sysroot 后重跑，不再从裸 `rust-lld` 取结论。

### 16.143 AArch64 syscall 修正版构建与导出审计 PASS（2026-11-28）

- FACT：按 Makefile `TEST_LINK` 口径重跑 `v4l2-shim` AArch64 `--no-run` PASS；仅有既有 dead-code/unused-assignment warnings。
- FACT：`make ... loader preloads` PASS；导出审计明确列出 `libhusk_v4l2.so epoll_ctl@GLIBC_2.17`，最终 `preload exports OK`。
- FACT：新产物 SHA256：
  ```text
  libld.so             a4724e3a23faf5674784a88710d23902b9241569bc3acb5587390d18bcb14f8a
  libhusk_v4l2.so      606c37cbce5189cac7ccc77913ff91412b469d048645a1769b0646da3e3df86f
  husk-bridges         4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  ```
- 下一步：停止当前 Snapshot/媒体/桌面会话，部署新 loader/preload，重新走公开入口和正式 Snapshot；运行验证仍为 NOT VERIFIED。

### 16.144 旧 Snapshot、媒体栈与 guest 会话已清场（2026-11-28）

- 操作：结束 Snapshot PID `21717`，执行 `media-reload-correct.sh stop`，再结束全部残留 `libld.so`/helper 进程。
- FACT：WirePlumber `21645`、PipeWire `21635` 正常停止；精确进程名核对后 `libld.so`、`husk-bridges`、`pipewire`、`wireplumber` 均为空。
- 下一步：部署 `a4724e3a...` loader 与 `606c37cb...` preload，旧文件先备份为 `*.pre-16.143`。

### 16.145 syscall 修正版已部署，设备四目标 SHA 全对上（2026-11-28）

- FACT：设备回读：
  ```text
  husk/libld.so             a4724e3a23faf5674784a88710d23902b9241569bc3acb5587390d18bcb14f8a
  husk-stage/libld.so       a4724e3a23faf5674784a88710d23902b9241569bc3acb5587390d18bcb14f8a
  runtime/husk-bridges      4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f
  rootfs/libhusk_v4l2.so    606c37cbce5189cac7ccc77913ff91412b469d048645a1769b0646da3e3df86f
  ```
- 旧文件已备份为 `*.pre-16.143`；media session 源码脚本保持 `93e2b448...`。
- 下一步：重新经宿主公开入口进桌面，启动媒体栈并确认双进程 preload，再重跑正式 Snapshot。

### 16.146 公开入口保留新 loader，但再次重建旧 helper/preload（2026-11-28）

- FACT：点击公开“打开桌面”后，设备 `husk/libld.so` 仍为 `a4724e3a...`；但 `runtime/husk-bridges` 被覆盖为 `c0663333...`，rootfs `libhusk_v4l2.so` 被覆盖为 `ff1b4b55...`。
- FACT：宿主启动 helper PID `23194`、桌面根 PID `23198`；因此桌面 loader 身份有效，helper/camera preload 身份无效。
- 判定：黑盒宿主每次桌面启动都会重铺 helper/preload，但本次没有覆盖 loader；测试采用“保留宿主桌面，只替换并重启 helper + media 服务”的最小外部阻塞绕行，不改宿主 APK。
- 下一步：结束旧 helper，重新部署 `4e039660...`/`606c37cb...`，按源码要求 `HUSK_FILES`、`HUSK_ROOTFS`、`HUSK_PARENT_PID` 启动 helper，再验证 socket。

### 16.147 新 helper 已手动接管并 running（2026-11-28）

- 操作：结束宿主重建的旧 helper PID `23194`，从 `runtime/husk-bridges` 部署 `4e039660...`；用源码声明的 `HUSK_FILES`、`HUSK_ROOTFS`、`HUSK_PARENT_PID=23198` 启动 helper。
- FACT：新 helper PID `23910`，`/proc` 进程 exe 为 `files/linux/runtime/husk-bridges`；启动日志包含正确 files/rootfs；`rootfs/run/husk/camera.sock` 已存在。
- 说明：启动 shell PID `23909` 仍在父级等待，但 helper 已脱离并持续 running；不影响后续媒体栈。
- 下一步：核对 media session 脚本身份，启动 PipeWire/WirePlumber 并确认双进程映射 `606c37cb...`。

### 16.148 公开入口也覆盖了 media session 脚本，PipeWire 再次无 preload（2026-11-28）

- FACT：rootfs `/usr/local/bin/husk-media-session` 被宿主重建回 SHA `bff08695...`；`media-reload-correct.sh start` 后 PipeWire `23979` 的 preload maps=0，WirePlumber `23997` maps=7。
- 判定：与 16.137 同一外部覆盖问题，不是新 preload 失效。必须先重铺源码版 media session，再重启 PipeWire/WirePlumber。
- 下一步：重新部署 `93e2b448...` 脚本，stop/start 媒体栈并核对 PipeWire maps。

### 16.149 media session 已重新部署，新 preload 在双进程生效（2026-11-28）

- FACT：rootfs media session SHA 恢复并核对为 `93e2b448...`；重启后 PipeWire PID `24073`、WirePlumber PID `24083` 的 preload maps 均为 `7`，exe 均为本轮 `a4724e3a...` loader。
- FACT：`wpctl` Video Sources 为 `* 40. BACK (V4L2)`、`42. FRONT (V4L2)`。
- 下一步：清空 logcat，用 `nohup`/日志重定向正式启动 Snapshot，避免 `gio launch` 占用 adb 会话；随后抓首帧、eventfd 日志和 `pw-top`。

### 16.150 eventfd 修复生效：持续 DQBUF 与 helper 真实帧已打通，但 Snapshot 随后 SIGSEGV（2026-11-28）

- FACT：修正 AArch64 syscall 后，正式 Snapshot 首次进入正常数据流，PipeWire 日志出现 `epoll facade -> eventfd`，并在 back `STREAMON` 后连续完成多轮 `DQBUF send FRAME → DQBUF ok`；`QBUF`/`STREAMOFF` 生命周期也完整出现。
- FACT：helper PID `23910` 出现 `camera|frame|ready|seq=1|1920x1080`；后续 front 流还出现 `frame|ready|seq=1`，证明 Camera2 → camera.sock → V4L2/PipeWire 的首帧链路已真实打通，不再是黑屏前的 fallback。
- FAIL：Snapshot 进程随后在自身启动日志写出
  ```text
  segv|sig=11|fa=507217031216|pc=507688734172|lr=-1|sp=506126586864|fp=1080|x0=367231049288|x1=507217031216|x2=505799842968|x8=0|ld=507691544576|lc=0
  ```
  即 `SIGSEGV (139)`；应用窗口消失。相机门禁仍为 **FAIL**，不能把“有帧”冒充“应用可用”。
- 下一步：复跑并抢在崩溃前保存 `/proc/<snapshot>/maps`，把 PC `0x76349b4ddc` 归属到具体模块；同时保留 eventfd 已打通的事实，不回退修复。

### 16.151 Snapshot 崩溃已精确定位到 `libgstvideo` 的 plane copy（2026-11-28）

- 操作：复跑 Snapshot，并在启动后 200ms 起轮询保存首个 `/usr/bin/snapshot` 进程的 maps；成功抢到崩溃前现场 `rootfs/tmp/nr-live-snapshot.maps`。
- FACT：第二次崩溃记录为
  ```text
  segv|sig=11|fa=509648416816|pc=510329834972|lr=-1|sp=508971963376|fp=1080
  ```
  PC `0x76d2074ddc` 落在 `libgstvideo-1.0.so.0.2402.0` 的 `0x84ddc` 偏移；`llvm-addr2line` 解析为 `gst_video_frame_copy_plane`。
- FACT：触发地址 FA `0x76a969b030` 在崩溃时 maps 中没有任何映射；PC 所在库 SHA 为 `6ca03094f4e8a45f63fe427d25b22a760e03566bbee3baf838b80aad71c04ee4`。
- 判定：不是 Camera2/helper 再黑屏；数据确实到达 GStreamer，但 plane copy 使用的源/目标内存指针已失效或错误。下一步记录 V4L2 mmap/DQBUF 的实际指针与长度，对照 FA 判断是否为提前 unmap/错指针。

### 16.152 指针审计补丁首次脚本修改失败，源码未写入（2026-11-28）

- FAIL：尝试用 Python 大段字符串替换给 `v4l2-shim` 增加 `trace_hex`、`MMAP ptr/len`、`DQBUF dst/cap/bytes` 审计；前两处替换构造成功，但在 DQBUF 上下文断言处失败。
- FACT：脚本在最终 `write_text()` 前退出，`runtime/crates/v4l2-shim/src/lib.rs` 本次没有发生修改；不能把这次失败当成已加审计。
- 纠正：停止大段字符串替换，改用 `nl -ba` 定位行号并通过 `apply_patch` 小步修改；每次修改后先记录，再构建与上机验证。

### 16.153 开始写入 mmap/DQBUF 有界十六进制审计（待验证，2026-11-28）

- 现状：已用 `nl -ba` 核对 `trace_dev`、`dqbuf`、`mmap` 的实际行号与上下文，未发现与交接摘要冲突。
- 操作：将新增单行有界 `trace_hex()`；在 `mmap()` 成功后记录映射指针与长度，在采样的 `DQBUF` 路径记录 `dst/cap/bytes`。不改变帧数据写入与返回逻辑，不增加非采样的热路径日志。
- 待验证：格式检查、AArch64 测试目标编译、preload/loader 构建、导出审计及设备上指针/FA 对照。

### 16.154 审计代码已落盘且静态 diff 检查通过（构建待验证，2026-11-28）

- FACT：`git diff --check -- runtime/crates/v4l2-shim/src/lib.rs` PASS；新增 `trace_hex()` 使用固定 96 字节栈缓冲输出单行十六进制，不分配内存。
- FACT：`mmap()` 成功后记录 `MMAP ptr`/`MMAP len`；采样 DQBUF 在读取前记录 `dst/cap/bytes`，未改动帧路径控制流。
- 下一步：执行 `cargo fmt --all -- --check`、shim AArch64 测试目标编译、`make loader preloads` 和导出审计。

### 16.155 格式检查 PASS；首次 AArch64 编译命令路径错误（2026-11-28）

- FACT：在 `/Users/l/Documents/Codex/2026-08-10/husk/runtime` 下执行 `cargo fmt --all -- --check` PASS。
- FAIL：随后沿用 `--manifest-path runtime/Cargo.toml`，相对当前目录变成 `runtime/runtime/Cargo.toml`，报 `manifest path ... does not exist`，退出码 101；这是命令工作目录与路径重复，不是代码编译失败。
- 纠正：在当前 `runtime` 目录改用 `--manifest-path Cargo.toml`，重新执行 AArch64 测试目标编译。

### 16.156 AArch64 编译发现 mmap 审计触发借用冲突（2026-11-28）

- FAIL：`v4l2-shim` 编译报 `E0502` 两处；`mmap()` 先持有 `&mut self.devs[i]`，随后调用 `self.trace_hex()` 需要不可变借用 `self`，编译器拒绝。
- 判定：这是新增审计代码的 Rust 借用生命周期问题，不涉及 syscall 或运行逻辑；不能跳过编译直接部署。
- 纠正：先读取 `memory/nbuffers` 做边界检查，映射成功并完成 trace 后再取得 `&mut self.devs[i]` 写回 `addr/len`。

### 16.157 mmap 借用冲突已修正，重跑构建（待验证，2026-11-28）

- 操作：`mmap()` 的边界检查改为直接读 `self.devs[i]`，`trace_hex()` 之后才创建可变借用写回缓冲地址与长度。
- 下一步：重跑格式检查和 AArch64 测试目标编译；只有 PASS 才继续 `make loader preloads`。

### 16.158 格式检查与 shim AArch64 编译 PASS（2026-11-28）

- FACT：`cargo fmt --all -- --check` PASS。
- FACT：`cargo test --manifest-path Cargo.toml --target aarch64-unknown-linux-gnu -p v4l2-shim --no-run` PASS；仅有既存 `w` 未读取和 `bytes_per_line` 未使用警告。
- 下一步：从仓库根执行 `make loader preloads`，随后审计导出符号并计算新产物 SHA256。

### 16.159 loader/preloads 构建与导出审计 PASS（2026-11-28）

- FACT：`make ... loader preloads` 退出 0；`libhusk_v4l2.so` 的 `epoll_ctl@GLIBC_2.17` 及其他必需导出全部 `OK`，最终输出 `preload exports OK`。
- FACT：构建仍有既存 warning（workspace profile、未使用赋值/method、linker version reassignment 等），没有升级为构建错误；本次不把 warning 冒充 clean build。
- 下一步：定位本轮产物路径、计算 SHA256，并先写部署记录再替换设备文件。

### 16.160 本轮审计产物身份已记录（部署前，2026-11-28）

- FACT：本轮新产物 SHA256：
  - `app/app/src/main/jniLibs/arm64-v8a/libld.so`：`fc40912bb8f9a26c1943f82d68caf28604cfb157093c37315dc86e60127800c8`
  - `runtime/target/aarch64-unknown-linux-gnu/release/libhusk_v4l2.so`：`e875890cb1a991f8ae926804c994b0bc7677977e8bb110bda4ffec5136c50299`
  - `app-rs/target/aarch64-linux-android/release/husk-bridges`：`4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f`
- 说明：helper 本轮未改，保持原 SHA；loader/preload 必须成对部署，不能只换其中一个。
- 下一步：先观察设备当前进程与文件身份，再停止 Snapshot/PipeWire/WirePlumber 和旧 helper，部署上述 loader/preload。

### 16.161 设备现状与执行身份已核对（2026-11-28）

- FACT：设备 `UKHMMB8PYTGECMNV` 在线；桌面根 PID `23198`，helper PID `23910`，PipeWire PID `24073`，WirePlumber PID `24083`；Snapshot 当前未运行。
- FACT：设备 loader `files/linux/husk/libld.so` 仍为旧版 `a4724e3a...`，preload 仍为旧版 `606c37cb...`，helper 为 `4e039660...`；media session 源码版 `93e2b448...` 仍在。
- FACT：`/proc/23198/exe`、`/proc/24073/exe`、`/proc/24083/exe` 均指向 `files/linux/husk/libld.so`，因此 loader 应更新该路径；rootfs 中没有 `/libld.so`。
- 下一步：保留桌面根进程，停止媒体双进程和 helper；备份后部署 loader `fc40912b...` 与 preload `e875890c...`，核对设备端 SHA。

### 16.162 preload 已更新；loader 覆盖遇 ETXTBSY（FAIL，2026-11-28）

- FACT：旧媒体双进程和 helper 已停止；preload 已成功替换并核对为 `e875890cb1a991f8ae926804c994b0bc7677977e8bb110bda4ffec5136c50299`。
- FAIL：直接 `cp` 到正在被桌面进程执行的 `files/linux/husk/libld.so` 报 `Text file busy`，设备 loader SHA 仍为旧 `a4724e3a...`。不能把 preload 更新冒充 loader 已更新。
- 纠正：在 loader 同目录写入临时文件，`chmod 755` 后用原子 `mv` 替换目录项；已运行进程继续持有旧 inode，新启动的媒体进程会使用新 loader，桌面根进程不重启。

### 16.163 loader/preload 设备端身份已达到预期（2026-11-28）

- FACT：原子替换后设备 `files/linux/husk/libld.so` SHA 为 `fc40912bb8f9a26c1943f82d68caf28604cfb157093c37315dc86e60127800c8`。
- FACT：设备 rootfs `libhusk_v4l2.so` SHA 为 `e875890cb1a991f8ae926804c994b0bc7677977e8bb110bda4ffec5136c50299`；helper 保持 `4e039660...`。
- 下一步：启动新 helper，再用 `files/linux/media-reload-correct.sh start` 启动媒体栈；核对 PipeWire/WirePlumber 的 exe 和 preload maps 必须对应本轮 SHA。

### 16.164 新 helper 已启动且 camera.sock ready（2026-11-28）

- FACT：helper 子进程 PID `27104`（父 shell `27102`）持续运行；日志确认 `files=/data/user/0/com.linuxhub.myth.husk/files`、`rootfs=.../ubuntu-24.04/rootfs`。
- FACT：`rootfs/run/husk/camera.sock` 已是 socket 且可访问；helper 文件 SHA 保持 `4e039660...`。
- 下一步：执行设备临时入口 `media-reload-correct.sh start` 拉起 PipeWire/WirePlumber；随后必须用新 PID、exe 和 maps 证明使用 `fc40912b...` + `e875890c...`。

### 16.165 媒体栈已用新 loader/preload 启动，双摄像头重新可见（2026-11-28）

- FACT：PipeWire PID `27171`、WirePlumber PID `27187` 均 ready；两者 `/proc/<pid>/exe` 均为新 loader `files/linux/husk/libld.so`，`libhusk_v4l2.so` maps 均为 7 段。
- FACT：`wpctl status` 的 Video Devices 为 `36. BACK [v4l2]`、`39. FRONT [v4l2]`；Video Sources 为 `40. BACK (V4L2)`、`42. FRONT (V4L2)`，默认 node 为 back。
- 下一步：通过 `guest-correct.sh /usr/bin/snapshot` 启动正式 Snapshot，同时轮询保存 `/proc/<snapshot>/maps`；检查新审计日志能否给出 `MMAP ptr/len` 与 `DQBUF dst/cap/bytes`。

### 16.166 新审计版 Snapshot 仍 SIGSEGV，指针日志已拿到但 MMAP 日志缺失（FAIL，2026-11-28）

- FAIL：正式 Snapshot PID `27308` 约 3.6 秒后退出码 `139`，窗口未成立；相机门禁继续 FAIL。
- FACT：崩溃日志为
  ```text
  segv|sig=11|fa=512587051056|pc=513332956636|lr=-1|sp=512208131056|fp=1080
  ```
  十六进制为 FA `0x775891b030`、PC `0x7785074ddc`、SP `0x7741fbd3f0`；PC 仍落在 `libgstvideo` 的 `0x84ddc`，即 `gst_video_frame_copy_plane`。
- FACT：PipeWire 日志记录 16 次 `DQBUF ok`；NV12 `cap=0x3f4800`、`bytes=0x2f7600`，映射目标在 `0x7fa605b030` 与 `0x7fa644f860` 之间轮换。崩溃 FA 与这些地址高位不同但低 16 位同为 `b030`，尚不能据此断定是同一映射。
- FAIL：本轮 `grep "MMAP ptr"` 为 0，说明 `mmap` 审计没有出现在 PipeWire 日志；但 DQBUF 的目标地址非零、帧可读取，说明映射路径确实发生，审计 hook/输出位置仍有缺口，不能把审计记录当作完整。
- FAIL：轮询保存的 `nr-live-snapshot-pointer-audit.maps` 只停在 `/usr/bin/env` 阶段，未捕获到 Snapshot 的完整映射；该 maps 不能用于判定故障地址，需改为扫描 cmdline 含 `/usr/bin/snapshot` 的进程并连续覆盖保存。
- 下一步：先解释并补齐 `mmap` 审计缺失，再改进 Snapshot 进程识别与 maps 抓取；同时检查 helper 输出是否只是重定向后的缓冲导致无 `frame` 行。

### 16.167 改为按 cmdline 扫描 Snapshot 并抓完整 maps（2026-11-28）

- 假设（INFERENCE）：`DQBUF cap/bytes` 路径可能使用 `V4L2_MEMORY_USERPTR`，因此没有走 `mmap()`；现有 16 次 `DQBUF ok` 不能证明 mmap 未发生，也不能证明发生。
- 操作：复跑正式 Snapshot；监控脚本遍历 `/proc/*/cmdline`，匹配 `/usr/bin/snapshot`，每 20ms 覆盖保存该 PID 的完整 maps，避免继续抓到紧邻的 `/usr/bin/env` 进程。
- 目标：在崩溃前得到包含 Snapshot、GStreamer、PipeWire client 和 buffer 映射的现场；同时保留 stderr 的 139 记录。

### 16.168 首次 cmdline 扫描误抓监控 shell，Snapshot 仍 139（FAIL，2026-11-28）

- FAIL：按 cmdline 扫描时，监控 shell 自身命令行也包含字符串 `/usr/bin/snapshot`，导致保存的是 PID `27659` 的 shell maps，而不是 Snapshot；该 maps 仍不能用于根因判断。
- FACT：被监控的 Snapshot 启动器 PID `27661` 仍在约 3.8 秒后退出 `139`；monitor 仅循环 5 次。
- 纠正：进程匹配必须同时满足 `readlink /proc/<pid>/exe` 为目标 `files/linux/husk/libld.so` 且 cmdline 含 `/usr/bin/snapshot`，排除 `/system/bin/sh` 监控脚本自身。

### 16.169 完整 Snapshot maps 已拿到，发现崩溃日志 ucontext 偏移错误（2026-11-28）

- FACT：改进匹配后成功抓到 PID `28487` 的完整 maps，大小 `173636` 字节；该进程确为目标 loader，cmdline 的 argv0/程序均为 `/usr/bin/snapshot`。
- FACT：本轮崩溃 `fa=541823381552 (0x7e2730b030)`、`pc=542790340060 (0x7e60d34ddc)`。按现场 maps，FA 位于
  ```text
  7e2730b000-7e27700000 rw-s ... /memfd:pipewire-memfd:...,type=2,size=33177984 (deleted)
  ```
  即 FA 当时并非未映射，而是 PipeWire 共享 memfd 的 `rw-s` 映射起点 + `0x30`；PC 值落在 `libgstvideo` 的 `0x84ddc`，该指令是 `memcpy@plt` 调用后的返回地址。
- 判定：先前把 PC 当成实际故障指令、把 FA 当作未映射地址的结论不完整。结合当前日志字段 `lr=-1`、`fp=1080` 明显不合理，怀疑 `on_segv()` 读取的 ucontext 起点错误，导致把 LR 位置读成了 PC。
- FACT：guest AArch64 `ucontext_t` 布局为 `uc_mcontext` 在 16 字节对齐后的偏移 `176`；`mcontext_t` 前 8 字节是 `fault_address`，regs[0] 从 `+8` 开始，sp `+256`、pc `+264`。当前源码 `on_segv()` 使用 `const MC: usize = 160`，与实际布局相差 16 字节。
- 纠正：把崩溃现场读取的 `MC` 改为 `176`，重建 loader 后再抓一次；届时 pc 应指向真正故障的 `memcpy` 内部，寄存器也应自洽。

### 16.170 on_segv mcontext 偏移 176 已落盘，开始重建（待验证，2026-11-28）

- 操作：`runtime/crates/path-shim/src/lib.rs` 的 `on_segv()` 从 `MC=160` 改为 `MC=176`，注释同步为实际 `ucontext_t` 布局；不改信号处理链和默认崩溃语义。
- 下一步：`cargo fmt --all -- --check`、重建 loader/preloads、计算新 loader SHA；部署后复跑 Snapshot，用真实寄存器判断 FA/PC 是否指向 `memcpy` 源或目标越界。

### 16.171 ucontext 修正版构建 PASS（2026-11-28）

- FACT：`cargo fmt --all -- --check` PASS；`make loader preloads` 退出 0，GLIBC 导出审计输出 `preload exports OK`。
- FACT：新 loader SHA256 为 `09f7758c7e8687e87fdda85ff0487d552d6c76b4c77a44683281f18902119f1e`；preload 未变，仍为 `e875890c...`。
- 下一步：停止媒体双进程和 helper，原子替换设备 loader，再拉起 helper/media 并核对新 PID、exe、maps。

### 16.172 ucontext 修正版 loader 已部署设备（2026-11-28）

- FACT：旧媒体进程与 helper 已停止；原子替换后设备 loader SHA 为 `09f7758c7e8687e87fdda85ff0487d552d6c76b4c77a44683281f18902119f1e`，preload 仍为 `e875890c...`。
- 下一步：重启 helper 与媒体栈，确认媒体进程实际 exe/maps 使用此 loader，再运行 Snapshot 获取修正后的崩溃现场。

### 16.173 新 helper/media 已重启并加载修正版身份（2026-11-28）

- FACT：helper PID `29469`，camera.sock ready；PipeWire PID `29498`、WirePlumber PID `29512` 均 ready。
- FACT：两个媒体进程 exe 均为 `files/linux/husk/libld.so`，`libhusk_v4l2.so` maps 均为 7 段；进程启动时间晚于 `09f7758c...` 部署，可确认不是旧 loader 进程复用。
- 下一步：再次以正式 Snapshot + exe/cmdline 双重匹配抓完整 maps，读取修正后的 PC/寄存器。

### 16.174 修正版首次复跑：Snapshot 存活超过 60 秒但画面仍黑（FAIL/NOT VERIFIED，2026-11-28）

- FACT：Snapshot PID `29589` 在修正版 loader 下存活超过 60 秒，进程仍在；本次没有立即出现 139，PipeWire 持续产生 `DQBUF ok` 并反复启动/暂停 stream。
- FAIL：设备截图显示 Snapshot 窗口存在，但预览区域仍为纯黑，没有真实相机画面；不能因进程不崩或 DQBUF 连续就判相机 PASS。
- 说明：高频复制 173KB maps 的监控循环仍在运行，可能改变时序；该次“未立即崩溃”不能证明崩溃已根治，也不能与之前无监控复跑直接等价。
- 下一步：停止监控 shell（不要把仍在运行的 Snapshot 一并当成结果），检查 `pw-top` RATE、helper 帧日志和窗口当前状态；随后至少无监控复跑多轮，记录是否仍 139。

### 16.175 当前存活实例：Camera2 帧与 PipeWire 30Hz 均存在，但 Snapshot 仍黑屏（2026-11-28）

- FACT：停止高频 maps 监控 shell 后 Snapshot PID `29589` 仍存活；5 秒后截图仍为纯黑预览。
- FACT：`adb logcat -s Husk:I` 显示 helper 持续 `camera|capture|started|count=4410`、`camera|image_reader|available`、`camera|frame|ready|seq=4350|1920x1080`；期间存在 `frame|acquire|rc=-30001|image_null=true` 后立即 `recovered` 的反复状态。
- FACT：`pw-top -b -n 2` 中 FRONT V4L2 node `42` 为 `RATE=30`、`YUY2 1920x1080`；Snapshot node `46` 显示 `RATE=0`。因此硬件帧源/PipeWire 生产端已有活动，但消费端没有形成可见预览。
- 判定：当前不是单纯“无帧”或“服务未启动”，而是 GStreamer/Snapshot 消费或呈现链路仍失败；相机门禁仍 **FAIL**。

### 16.176 扩大崩溃自落盘 maps 缓冲，准备无监控复跑（待验证，2026-11-28）

- 现状：外部 20ms 复制完整 maps 会显著影响时序；当前 `path-shim` 自带的 `CRASH_MAPS` 只有 65536 字节，而本次现场 maps 约 173636 字节，最后一个窗口可能截断。
- 操作：将 `CRASH_MAPS` 扩到 262144 字节；崩溃处理器继续直接读取 `/proc/self/maps` 并写到 guest `/tmp/nr-crash-<pid>.maps`。这样无外部监控也能保存完整现场。
- 下一步：重建并部署 loader，结束当前黑屏 Snapshot，无监控重启正式 Snapshot，读取修正后的 `segv` 行和完整自落盘 maps。

### 16.177 崩溃 maps 缓冲已扩到 256KiB，开始重建（待验证，2026-11-28）

- 操作：`runtime/crates/path-shim/src/lib.rs` 的 `CRASH_MAPS` 改为 `262144` 字节；未改崩溃记录格式或恢复默认信号动作。
- 下一步：格式检查、重建 loader、部署并核对 SHA；结束当前黑屏 Snapshot 后无监控复跑。

### 16.178 256KiB maps 诊断版 loader 构建 PASS（2026-11-28）

- FACT：`cargo fmt --all -- --check` PASS；`make loader` 退出 0。
- FACT：新 loader SHA256 为 `0ea8c2b5c732541d2606406ef93e193fecf4824946773759cedccba6e41ef53b`，文件大小 `1263104` 字节。
- 下一步：停止当前黑屏 Snapshot、媒体双进程和 helper；原子部署新 loader 后重启，进行无外部 maps 监控的正式 Snapshot 复跑。

### 16.179 256KiB maps 诊断版已部署并启动（2026-11-28）

- FACT：设备 loader SHA 已核对为 `0ea8c2b5...`；helper PID `18874`，PipeWire PID `18896`、WirePlumber PID `18910` 均 ready。
- FACT：媒体双进程 exe 均为新 loader，`libhusk_v4l2.so` maps 均为 7 段。
- 下一步：无外部高频 maps 复制启动 Snapshot；进程若退出则取返回码、stderr `segv` 行和崩溃处理器自落盘的 `/tmp/nr-crash-<pid>.maps`。

### 16.180 无监控复跑 20 秒：Snapshot 不再立即 139，但仍黑屏（FAIL，2026-11-28）

- FACT：Snapshot PID `18968` 无外部 maps 监控启动，20 秒后仍存活；没有产生 `nr-crash-*.maps`，也没有新的 `segv` 记录。
- FAIL：是否“永不复现”尚不能确认，且截图/窗口预览仍黑；相机门禁继续 FAIL。
- 说明：当前首要阻塞已从“立即崩溃”转为“数据到 PipeWire 后 Snapshot 不显示”。下一步直接绕过 PipeWire，用 guest `v4l2src` 抓一帧并保存，判断 shim 输出的 YUY2 内容是真实画面还是全黑。

### 16.181 改用仓库自带 `v4l2-test` 直接抓 V4L2 帧（2026-11-28）

- FACT：guest 未安装 `gst-launch-1.0`，首次直接用它的命令报 127；不能把缺失工具当成相机失败。
- FACT：仓库已有 `runtime/target/aarch64-unknown-linux-gnu/release/v4l2-test`，支持 `v4l2-test [dev] [frames] [fourcc] [out-prefix] [width] [height]`，会打印帧统计并可保存 raw/PPM。
- 操作：结束当前黑屏 Snapshot 后，将该探针部署到 rootfs `/tmp`，通过 `guest-correct.sh` 运行 `v4l2-test /dev/video1 3 YUYV /tmp/husk-v4l2-yuyv 1920 1080`；据此判断 shim 输出是否有真实非黑像素。

### 16.182 直接 V4L2 抓帧 PASS；但 `/tmp` 落盘文件不可见（2026-11-28）

- FACT：`v4l2-test /dev/video1 3 YUYV ...` 输出 `v4l2test|OK`，连续 DQBUF 3 帧，每帧 `4147200` 字节，fps 约 `12.01`，设备格式为 YUYV 1920x1080。
- FAIL：探针打印 `saved|/tmp/husk-v4l2-yuyv.raw`，但 run-as 在 rootfs `/tmp` 找不到该文件；`std::fs::write` 失败被探针忽略，日志不能作为已落盘证据。
- 纠正：改用 guest `/home/husk/husk-v4l2-yuyv` 输出，避免 `/tmp` 映射路径不确定；重新抓帧后 pull 并统计像素。

### 16.183 FRONT 直接抓帧已落盘，但内容为黑帧（FAIL，2026-11-28）

- FACT：重跑 `v4l2-test /dev/video1 3 YUYV /home/husk/husk-v4l2-yuyv 1920 1080` PASS，并成功生成 4147200 字节 raw；不再依赖不可见 `/tmp`。
- FACT：对本机拉回的 raw 统计 Y 分量为 `min=0`、`max=6`、`mean=0.1287`；转换出的整帧图像接近纯黑。探针“DQBUF 成功”只证明传输成功，不证明相机内容有效。
- 判定：目前 FRONT 的真实 V4L2 输出就是黑帧；Snapshot 黑屏与数据内容一致，不能继续把问题归给 Snapshot 呈现层。
- 下一步：对 BACK `/dev/video0` 做同样 3 帧抓取与像素统计，区分单个摄像头/朝向问题和 Camera2→NV12 公共路径问题。

### 16.184 BACK 也为黑帧；转到 Camera2 selftest 分层定位（FAIL，2026-11-28）

- FACT：BACK `/dev/video0` 同样成功 DQBUF 3 帧、每帧 4147200 字节；本机统计 Y `min=0`、`max=2`、`mean=0.4058`，仍为近纯黑。
- 判定：FRONT 与 BACK 都在 V4L2 输出端为黑，问题位于 Camera2 采集/NV12 转换公共路径，或两个物理摄像头当前都无有效曝光；不能继续归因于 Snapshot/PipeWire 呈现。
- 操作：停媒体栈与 helper，写 `files/camera-selftest` 标记 `frames=3 camera=0`，重启 helper，让仓库现有 selftest 在 Camera2 ImageReader 层直接保存 NV12 并打印 `min/max/mean`；随后以 `camera=1` 重测 front。

### 16.185 Camera2 selftest 在 BACK 1280x720 输出有效非黑帧（2026-11-28）

- FACT：`camera-selftest frames=3 camera=0` 在 `1280x720` 连续保存 3 帧；日志统计分别为 `min=0/max=128/mean=42.91`、`mean=42.97`、`mean=42.97`，`saved=true`，fps=20.00。
- 判定：Camera2 → AImageReader → NV12 路径在 1280x720 下确实有有效画面；此前“Camera2 全黑”判断过宽，应修正为“目前 1920x1080 抓取为黑或无效”。
- 下一步：用同一 V4L2 `v4l2-test` 在 BACK 1280x720 抓 YUYV，验证 shim 转换后仍非黑；再单独复测 1920x1080，定位是分辨率协商/采集还是转换问题。

### 16.186 V4L2 BACK 1280x720 仍为黑帧，继续分层到 camera.sock（FAIL，2026-11-28）

- FACT：`v4l2-test /dev/video0 3 YUYV ... 1280x720` PASS，每帧 1843200 字节；拉回统计 Y `min=0`、`max=2`、`mean=0.3365`，仍为近纯黑。
- 判定：720p 下 Camera2 selftest 非黑，但 V4L2 façade 输出黑，问题已缩小到 camera.sock 发送/接收或 V4L2 shim 写入/映射转换，不是采集分辨率本身。
- 下一步：直接运行仓库 `camera-client` 读 camera.sock 的原始 NV12 并打印 `min/max/mean`；若它也黑，问题在 helper 服务/协议；若非黑，问题在 V4L2 shim 数据拷贝或 YUYV 转换。

### 16.187 camera-client 预构建产物不存在，先构建（FAIL，2026-11-28）

- FAIL：尝试 push `runtime/target/aarch64-unknown-linux-gnu/release/camera-client` 报文件不存在；此前只确认源码存在，未确认该 bin 已构建。
- 纠正：使用与 shim 相同的 AArch64 guest 链接环境 `cargo build -p camera-client --release --target aarch64-unknown-linux-gnu`，成功后再部署运行。

### 16.188 camera-client AArch64 构建 PASS，开始 socket 验帧（2026-11-28）

- FACT：`cargo build -p camera-client --release --target aarch64-unknown-linux-gnu` PASS，产物已生成。
- 下一步：通过 `guest-correct.sh` 连接 `/run/husk/camera.sock`，请求 BACK 1280x720 三帧并读取探针统计。

### 16.189 camera.sock 原始 NV12 为有效帧，问题锁定在 V4L2 shim（2026-11-28）

- FACT：`camera-client /run/husk/camera.sock ... frames=3 w=1280 h=720 id=0` PASS；3 帧 NV12 均为 `min=0/max=128/mean≈42.9`，与 Camera2 selftest 一致。
- 判定：helper 服务、camera.sock 协议和原始 NV12 payload 都正常；同分辨率下 V4L2 输出黑，故障已锁定在 shim 的 MMAP 目标写入或 NV12→YUYV 转换。
- 下一步：用 `v4l2-test ... NV12 1280 720` 抓取。若 NV12 非黑，则 YUYV 转换错误；若 NV12 也黑，则 MMAP/ QBUF 目标地址管理错误。

### 16.190 V4L2 NV12 也为黑，确认为 MMAP/写入目标层问题（FAIL，2026-11-28）

- FACT：`v4l2-test /dev/video0 3 NV12 ... 1280x720` PASS，输出 raw 中 Y `min=0/max=2/mean=0.3543`，仍为近纯黑。
- 判定：不是 NV12→YUYV 转换错误；同一个有效 camera.sock NV12 payload 经 V4L2 MMAP 路径后变黑，故障在 `mmap` 返回地址、`QBUF/DQBUF` 缓冲索引或 shim 写入目标管理。
- 操作：给 loader 内嵌 V4L2 路径也按 `NR_V4L2_AUDIT` 开启 `Ctx::set_trace`，并让 `v4l2-test` 打印每次 mmap 的返回用户指针；对照 `MMAP ptr` 与 `DQBUF dst`。

### 16.191 loader 审计开关与测试指针输出已落盘（待验证，2026-11-28）

- 操作：`v4l2_ready()` 初始化成功后，若 `NR_V4L2_AUDIT` 存在则调用 `V4L2_CTX.set_trace(true)`；`v4l2-test` 的 `buf` 行增加 `ptr=<返回地址>`。
- 下一步：格式检查、重建 loader 与 v4l2-test，部署后重跑 NV12 720p，直接比较用户态 mmap 指针与 shim 的 `MMAP/DQBUF` 地址。

### 16.192 loader/V4L2 审计构建 PASS（2026-11-28）

- FACT：格式检查、`v4l2-test` AArch64 release 构建、`make loader` 全部 PASS。
- FACT：新 loader SHA `e25f0299c5e406d36ddfb1d46b63f53a6dadebf95ec851e713c1ed7001bbf514`；新 `v4l2-test` SHA `28b55b7fa90f769f807aed5a00512162fede2cafe65e4df5d1f54060aca8aa60`。
- 下一步：部署两者，运行 BACK NV12 720p，抓 `v4l2test|buf ... ptr=` 与 stderr 中 `[v4l2] MMAP ptr/DQBUF dst`。

### 16.193 审计版 loader / v4l2-test 已部署，准备做 MMAP 地址对照（待复核，2026-11-28）

- 交接状态：审计版 loader `e25f0299c5e406d36ddfb1d46b63f53a6dadebf95ec851e713c1ed7001bbf514` 已原子部署到设备 `files/linux/husk/libld.so`，新 `v4l2-test` `28b55b7fa90f769f807aed5a00512162fede2cafe65e4df5d1f54060aca8aa60` 已部署到 rootfs `/tmp/v4l2-test`。
- 本轮先复核设备端 SHA、helper/socket 与部署文件是否仍一致，再运行 BACK NV12 1280x720 三帧。
- 必须对照 `v4l2test|buf|...|ptr=`、`[v4l2] MMAP ptr=`、`[v4l2] MMAP len=`、`[v4l2] DQBUF dst=`、`[v4l2] DQBUF cap=` 和 `[v4l2] DQBUF bytes=`；地址/长度不一致则直接修单一路径，不继续叠补丁。
- 状态：NOT VERIFIED；当前 V4L2 黑帧缺陷仍未修复。

### 16.194 设备运行时身份复核 PASS（2026-11-28）

- FACT：设备在线；loader SHA 与构建产物一致：`e25f0299c5e406d36ddfb1d46b63f53a6dadebf95ec851e713c1ed7001bbf514`。
- FACT：`rootfs/tmp/v4l2-test` SHA 与构建产物一致：`28b55b7fa90f769f807aed5a00512162fede2cafe65e4df5d1f54060aca8aa60`。
- FACT：`rootfs/run/husk/camera.sock` 存在且可读写；helper `husk-bridges` PID `20734` 在运行；`files/camera-selftest` 仍为 `frames=3 camera=0`。
- 下一步：通过 `guest-correct.sh` 运行 BACK `/dev/video0` 三帧 NV12 1280x720，并同时收集用户态 mmap 指针与 loader 审计输出。

### 16.195 BACK NV12 审计首次运行未进入取流：`fstat` 身份检查 FAIL（FAIL，2026-11-28）

- FACT：`v4l2-test` 对 `/dev/video0` 的 `stat` 通过（`chr=true rdev=81:0`），camera.sock INFO 握手也成功；打开后 `fstat` 却得到 `chr=false rdev=0:0`，探针以 `v4l2test|FAIL|fd identity is not character device 81:0` 退出。
- 判定：本轮没有进入 MMAP/DQBUF，不能用于相机数据面结论；当前审计版 loader 至少新增/暴露了 V4L2 fd 的 `fstat` 兼容问题。
- 下一步：检查 `v4l2-shim` 的 open/fstat/fcntl 拦截与 AArch64 syscall 分派，确认为何 `stat` 被伪装而 `fstat` 没有；修复后重跑同一命令。
- 状态：FAIL；V4L2 黑帧仍未解决。

### 16.196 下一步：打印 V4L2 fd 的内核链接目标（待验证，2026-11-28）

- 观察：`stat`/camera.sock 握手成功，但 `fstat` 返回 memfd 的普通文件身份 `0:0`，说明 `synth_fd_stat` 没认领该 fd。
- 最小假设：打开后的 fd 链接目标不是预期的 `/memfd:husk-v4l2-0`，或路径分派让 `fstat` 落入另一个 V4L2 Ctx。
- 操作：`v4l2-test` 在 open 后、fstat 前打印 `/proc/self/fd/<fd>` 的 readlink 目标；重建并部署探针，用同一 BACK NV12 720p 命令复现。
- 状态：待验证；不改生产语义。

### 16.197 fd 链接诊断版 v4l2-test 构建 PASS（含两次失败，2026-11-28）

- FACT：已加入 `v4l2test|fd=<n>|target=<read_link>` 只读诊断；`cargo fmt --package v4l2-test` 与 `git diff --check` PASS。
- FAIL：首次从仓库根执行 `cargo fmt` 报错“找不到 Cargo.toml”，未改文件；改到 `runtime/` 后继续。
- FAIL：直接用 `rust-lld` 两次链接均缺少 `-lgcc_s/-lutil/-lrt/-lpthread/-lm/-ldl/-lc`；这是链接环境问题，不是源码错误。
- PASS：使用 NDK clang + `--target=aarch64-linux-gnu --sysroot=/tmp/nr-toolchain/root` 构建成功；新探针 SHA `b9c53dfbad713eb9108d9507606c5cbe46d88fba37c851779c44907982bdbdb8`。
- 下一步：原子部署探针到 rootfs `/tmp/v4l2-test`，重跑 BACK NV12 720p，读取 fd readlink 目标。

### 16.198 fd 链接诊断版探针部署 PASS（2026-11-28）

- FACT：通过 `/data/local/tmp` 中转并以临时文件 + `mv` 原子替换到 `rootfs/tmp/v4l2-test`；设备 SHA 为 `b9c53dfbad713eb9108d9507606c5cbe46d88fba37c851779c44907982bdbdb8`，权限 `755`。
- 下一步：重跑 BACK NV12 1280x720 三帧，只读取 open 后 fd readlink 与 fstat 诊断行。

### 16.199 fd 命名正确，问题转为 fstat 分派/状态不一致（FAIL，2026-11-28）

- FACT：open 返回 fd `4`，内核 readlink 为 `Ok("/memfd:husk-v4l2-0 (deleted)")`；`fd-identity` 的精确规则应识别为 V4L2 index 0。
- FACT：同一次调用仍得到 `fstat|chr=false|rdev=0:0`，说明 `synth_fd_stat` 没有生效，问题不是 memfd 名称拼写。
- 判定：下一步必须确认 `fstat` 落入哪一层、`husk_v4l2_fstat` 是否看到同一份 `Ctx`；不能继续用原始 memfd `fstat` 冒充设备身份。
- 操作：读取设备上的 `guest-correct.sh` 与运行时环境，核对 loader、preload 注入顺序和实际 preload 路径。

### 16.200 根因确认：内嵌 V4L2 Ctx 的 fstat 没有回退（2026-11-28）

- FACT：`guest-correct.sh` 使用 loader 的 `usr/bin/env -i`，只设置 `NR_V4L2_AUDIT` 等变量，不注入 `LD_PRELOAD`；因此无 preload ABI 时相机由 loader 内嵌 `V4L2_CTX` 提供。
- FACT：`v4l2_dispatch_fstat()` 当前只要 `v4l2_api()` 返回 `None` 就直接返回 `false`，没有像 ioctl/mmap/read 那样回退到内嵌 `V4L2_CTX`；随后 `fstat` 看到的是 memfd 原始普通文件身份 `0:0`。
- 判定：这是原生分派缺口，不是 memfd 命名问题，也不是内核权限问题。
- 操作：修改 `v4l2_dispatch_fstat()`，先尝试 preload API，失败后回退到内嵌 `V4L2_CTX.char_device_rdev()` 并复用现有 `v4l2_stat()` 合成设备身份；保留 `v4l2-test` 的 fstat 正向后测试为回归闸门。
- 状态：待验证；V4L2 黑帧仍未解决。

### 16.201 fstat 内嵌 Ctx 回退修复构建 PASS（含命令路径错误，2026-11-28）

- FAIL：第一次执行 `make loader` 未传 `NDK`，Makefile 明确报 `set NDK=<ndk>/bin` 并退出；未产生部署。
- PASS：带 `NDK` 与 `GUEST_SYSROOT` 重新构建 loader PASS；新 loader SHA `11c2e3055862b0cdb89eefc30fdbd7ceba64386e5da45a24607e9cc032a156a9`，大小 `1263104` 字节。
- PASS：诊断版 `v4l2-test` 重新构建 PASS，SHA 仍为 `b9c53dfbad713eb9108d9507606c5cbe46d88fba37c851779c44907982bdbdb8`。
- FAIL：同一条命令最后从仓库根对 `libld.so` 取 SHA 写错路径，`sha256sum` 报文件不存在；随后从真实产物 `app/app/src/main/jniLibs/arm64-v8a/libld.so` 复算成功，源码/构建本身 PASS。
- 下一步：原子部署新 loader，保留当前 helper/camera.sock，重跑 BACK NV12 720p；先确认 fstat 通过，再看 MMAP/DQBUF 地址对照。

### 16.202 fstat 回退修复版 loader 部署 PASS（2026-11-28）

- FACT：新 loader `11c2e3055862b0cdb89eefc30fdbd7ceba64386e5da45a24607e9cc032a156a9` 已通过临时文件 + `mv` 原子替换到 `files/linux/husk/libld.so`，设备复算 SHA 一致。
- 下一步：用 `guest-correct.sh` 重跑 BACK `/dev/video0` 三帧 NV12 1280x720，确认 fstat 通过并收集 MMAP/DQBUF 审计。

### 16.203 fstat 修复 PASS；MMAP/DQBUF 地址与长度完全一致（2026-11-28）

- FACT：fd `4` 现在 `fstat|chr=true|rdev=81:0`，QUERYCAP、S_FMT、REQBUFS、mmap、QBUF、STREAMON、DQBUF 3 帧和 STREAMOFF 全部 PASS；探针输出 `v4l2test|OK`。
- FACT：四个用户 mmap 指针与 shim 审计逐一一致（例如 buffer 0 都是 `0x77f348e000`）；`MMAP len=0x151800` 与 `DQBUF cap=0x151800`、`DQBUF bytes=0x151800` 完全相等，均为 `1382400`。
- 判定：先前怀疑的重复 mmap/错误目标地址/长度不一致已被排除；fstat 回归也通过。
- 下一步：拉回 `husk-v4l2-audit.raw`，统计 Y 分量并检查 PPM；地址正确但内容仍可能为黑，像素结论尚未验证。
- 状态：传输路径 PASS；相机画面仍 NOT VERIFIED。

### 16.204 地址修复后仍为黑帧，故障转到 camera.sock→recv_exact（FAIL，2026-11-28）

- FACT：拉回的 BACK NV12 1280x720 raw 大小精确为 `1382400`；Y 分量 `min=0/max=3/mean=0.373487`，非零像素比例 `0.37347`，只有 4 个不同值，仍为近纯黑。
- FACT：证据文件 SHA：raw `3656ec61eb004ebfb60ae94966bf9b12df9eeb1181ba4fcf8623b81f87adadbc`；PPM `847b7f79e03d5c6627ebab623434be40bbabe552c8a3566755c717d621598486`。
- 判定：mmap 目标、cap、bytes 和 fd 身份都正确，黑帧来自 DQBUF 从 socket 收到的 payload 或其写入值，不再是 V4L2 buffer 地址管理。
- 下一步：检查 `recv_exact()` 与 frame reply 解析；在审计模式下对收到的 payload 做 bounded `min/max/mean` 统计，和 camera-client 的非黑结果直接对照。
- 状态：FAIL；Snapshot 仍不能判定可用。

### 16.205 下一步：同一 helper 下重跑 camera-client 对照（待验证，2026-11-28）

- 观察：`v4l2-test` 与 `camera-client` 使用相同 `OPEN w=1280 h=720 id=0` / `FRAME` 文本协议和 payload 长度，但前者得到黑帧。
- 最小假设：当前 helper 的相机会话状态已变化，或 V4L2 Ctx 的 OPEN 参数/解析与 camera-client 有差异。
- 操作：不重启 helper，先查找设备上的 `camera-client` 并立即请求 BACK 1280x720 三帧；若非黑，则继续审计 V4L2 OPEN 与 recv 内容；若也黑，则先修 helper/Camera2 当前会话状态。

### 16.206 同 helper 对照：camera-client 非黑，V4L2 路径仍黑（FAIL，2026-11-28）

- FACT：不重启 helper，直接 `camera-client` 请求 BACK 1280x720 三帧，得到 `min=0/max=128/mean=42.87~42.90`，`PASS`。
- 判定：Camera2、helper 和当前硬件会话仍能输出有效画面；黑帧只出现在 V4L2 Ctx 路径。
- FACT：V4L2 Ctx 与 camera-client 的主要协议差异是：V4L2 先在同一个 camera.sock 连接上发送 `INFO` 枚举，再复用该连接发送 `OPEN`/`FRAME`；camera-client 直接 `OPEN`。
- 下一步：核对 helper 的连接状态机和 `INFO` 结束后的协议状态，确认是否允许同连接继续 `OPEN`，并在 shim 侧修成正确的会话重连/状态切换。
- 状态：FAIL；不能把普通 camera-client 通过算作 Snapshot 可用。

### 16.207 下一步：审计 V4L2 OPEN 与实际 payload（待验证，2026-11-28）

- 操作：在 `open_session()` 的审计模式记录实际发送的 `OPEN ... id=...`，并在 `recv_exact()` 完成后对写入目标的前若干字节计算 `min/max/sum`。
- 判据：若 payload 统计非黑而测试读回黑，继续查 buffer 生命周期；若 payload 统计也黑，问题在 helper 对同连接 INFO→OPEN 会话的帧内容或 shim 请求参数。
- 状态：待验证；生产路径只在 `NR_V4L2_AUDIT` 下增加日志。

### 16.208 OPEN/payload 审计版 loader 构建 PASS（2026-11-28）

- FACT：`v4l2-shim` 格式检查 PASS；带 NDK + guest sysroot 重建 loader PASS。
- FACT：新 loader SHA `cce193be44c9f3af5ea855698af94b94c02868577fe8d69387475d601de744dd`，大小仍为 `1263104` 字节。
- 下一步：部署后重跑 BACK NV12 720p，抓实际 `OPEN w=... id=...` 与 `DQBUF payload min/max/sum`。

### 16.209 OPEN/payload 审计版 loader 部署 PASS（2026-11-28）

- FACT：loader `cce193be44c9f3af5ea855698af94b94c02868577fe8d69387475d601de744dd` 已原子替换，设备 SHA 一致。
- 下一步：运行 BACK NV12 720p 三帧，抓 OPEN 与 payload 统计。

### 16.210 V4L2 OPEN 参数正确，但 shim 收到的 payload 本身就是黑（FAIL，2026-11-28）

- FACT：实际发送行为 `OPEN w=1280 h=720 id=0`，与 camera-client 对照一致。
- FACT：三次 `DQBUF` 在 `recv_exact()` 后读取目标前 4096 字节：`min=0/max=1/sum=0x694,0x8eb,0x8e5`，不是测试读回或转换造成。
- 判定：helper 在这个 V4L2 会话下发的是黑帧。剩余差异只剩“同连接先 INFO 再 OPEN”或 V4L2 会话生命周期。
- 操作：给 camera-client 增加 `info=1` 测试模式，在同一连接先读完整 INFO 再 OPEN/FRAME；用它判断 INFO→OPEN 是否触发黑帧。
- 状态：FAIL。

### 16.211 camera-client 增加 `info=1` 对照模式并构建 PASS（2026-11-28）

- FACT：`info=1` 时同一 socket 先发送 `INFO`、读取所有 `CAM` 行直到 `OK`，之后按原协议 `OPEN/FRAME/STOP`。
- FACT：格式/差异检查与 AArch64 release 构建 PASS；新 camera-client SHA `bb16d1ebdb4cfaef29133a00cfffe79ef3f0256a01180b2d01307fd01f1fa767`。
- 下一步：部署后分别跑 `info=0` 与 `info=1` 的同分辨率三帧，直接比较黑/非黑。

### 16.212 camera-client `info=1` 版本部署 PASS（2026-11-28）

- FACT：设备 `/tmp/camera-client` 已原子替换，SHA `bb16d1ebdb4cfaef29133a00cfffe79ef3f0256a01180b2d01307fd01f1fa767` 与构建一致，权限 `755`。
- 下一步：同一 helper 下连续运行 `info=0` 和 `info=1` 对照，不重启、不换参数。

### 16.213 INFO→OPEN 对照：两种普通 client 路径均非黑（PASS 对照，2026-11-28）

- FACT：同一 helper 下 `info=0` 与 `info=1` 都输出 3 帧有效画面，`max=128/mean=42.88~42.94`。
- 判定：helper 的 INFO→OPEN 同连接状态机正常；“先 INFO”不是 V4L2 黑帧原因。
- 剩余差异：V4L2 Ctx 自己设置的 socket 选项、请求节奏、内存映射/直接写入流程，或 V4L2 OPEN 前的 fd/stat 路径触发了 helper 的不同会话。
- 下一步：抓 helper 对 V4L2 会话的 `camera|client|OPEN/INFO/FRAME` 日志，并与 camera-client 会话日志逐项比较。
- 状态：对照 PASS；V4L2 黑帧仍 FAIL。

### 16.214 下一步：helper 记录自身 frame 像素统计（待验证，2026-11-28）

- 操作：在 `CameraSession::try_frame()` 的首帧/每 30 帧日志中加入 `min/max/mean`，不改变请求或帧数据。
- 判据：V4L2 会话若 helper 自身统计也黑，问题在 helper/相机状态；若 helper 非黑而 V4L2 recv 黑，问题在进程间传输或 shim 读缓冲。
- 状态：待验证；当前黑帧仍 FAIL。

### 16.215 helper 帧统计诊断构建 PASS（含一次链接失败，2026-11-28）

- FAIL：未设置 Android linker 直接 `cargo build --bin husk-bridges` 时，`libhuskapp.so` 落到 host `cc`，报 unknown GNU linker options，构建失败；旧 helper 产物未被当成新构建。
- FACT：`cargo check --target aarch64-linux-android --bin husk-bridges` PASS。
- PASS：设置 `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`、`CC_aarch64_linux_android`、`AR_aarch64_linux_android` 为 NDK r28 clang/llvm-ar 后构建 PASS。
- FACT：新 helper SHA `7c52946a32dcb733528e12cfa8fbd53318687382a1a63be5d30fdc0c06910375`；日志增加 `camera|frame|ready|...|min/max/mean`。
- 下一步：核对当前 helper exe/启动参数，备份并重启到新 helper，再分别跑 V4L2 与 camera-client 对比 helper 自身帧统计。

### 16.216 当前 helper 运行身份核对（2026-11-28）

- FACT：helper PID `20734`，exe 指向 `files/linux/runtime/husk-bridges`；当前文件 SHA 仍是 `4e0396608ce6669c2e3fb91bb704e71b9bf707750f97991fff29c4506607aa2f`。
- FACT：启动环境为 `HUSK_FILES=/data/user/0/com.linuxhub.myth.husk/files`、`HUSK_ROOTFS=.../distributions/ubuntu-24.04/rootfs`、`HUSK_PARENT_PID=23198`、`NR_V4L2_AUDIT=1`；父桌面根进程 `23198` 仍存活。
- 下一步：备份当前 helper，停止 PID `20734`，原子替换为 `7c52946a...` 帧统计版，并用同环境参数经 `setsid -f` 重启。

### 16.217 helper 替换成功但首次重启失败：Android `setsid` 不支持 `-f`（FAIL，2026-11-28）

- FACT：旧 helper 已备份为 `husk-bridges.pre-16.215`；新 helper 原子替换成功，设备 SHA `7c52946a32dcb733528e12cfa8fbd53318687382a1a63be5d30fdc0c06910375`。
- FAIL：启动命令执行 `setsid -f ./husk-bridges ...`，Android `setsid` 报 `Unknown option 'f'`；旧 helper 已停止，新 helper 未启动。
- FACT：此时看到的 `rootfs/run/husk/camera.sock` mtime 仍是旧的 `05:18`，是残留 socket，不能判 ready。
- 纠正：改用 Android 支持的 `setsid ./husk-bridges ... > log 2>&1 < /dev/null &`，随后核对进程 PID、PPID、exe、socket mtime 和启动日志。

### 16.218 帧统计版 helper 启动 PASS（2026-11-28）

- FACT：使用 `setsid ./husk-bridges ... &` 启动成功；helper PID `22610`，exe 为 `files/linux/runtime/husk-bridges`，设备 SHA `7c52946a32dcb733528e12cfa8fbd53318687382a1a63be5d30fdc0c06910375`。
- FACT：`camera.sock` 已重建，mtime `05:29`；启动日志为正确的 `HUSK_FILES` 与 `HUSK_ROOTFS`。
- 下一步：清 logcat，分别跑 V4L2 一帧与 camera-client 一帧，比较 helper `camera|frame|ready` 的 min/max/mean。

### 16.219 helper 自身帧非黑，故障缩到 V4L2 socket→mmap 读取（FAIL，2026-11-28）

- FACT：V4L2 会话中 helper 自身记录 `camera|frame|ready|seq=1|...|min=0|max=128|mean=42.88`；camera-client 对照为 `mean=42.89`。两者 Camera2 输出都有效。
- 判定：helper 生成帧正常，黑帧发生在 `write_all(frame)` 之后的 V4L2 客户端接收/写入路径，已排除 Camera2、helper、INFO/OPEN 状态机。
- 操作：审计模式暂时改为先 `recv_exact()` 到 shim scratch，统计后再拷贝到 mmap；生产模式仍直接写 mmap。该实验用于区分“socket 字节本身黑”与“直接读入 mmap 后变黑”。
- 状态：FAIL；黑帧仍未修复。

### 16.220 scratch→mmap 审计实验版 loader 构建 PASS（2026-11-28）

- FACT：审计模式改为 socket→scratch，先输出 payload 统计，再 `copy_nonoverlapping` 到 mmap；生产模式仍 socket→mmap 直写。
- FACT：格式检查与 loader 构建 PASS；新 loader SHA `6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506`。
- 下一步：部署并运行 BACK NV12 720p 三帧，对照审计 payload 统计与测试保存的完整 raw。

### 16.221 scratch→mmap 审计实验版 loader 部署 PASS（2026-11-28）

- FACT：loader `6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506` 已原子替换，设备 SHA 一致。
- 下一步：运行 BACK NV12 720p 三帧；若 scratch 统计非黑但保存 raw 黑，则确认直读 mmap 问题；若 scratch 仍黑，则 socket 字节问题。

### 16.222 scratch 仍黑，下一步对账 helper 与 shim 同一帧前缀（FAIL，2026-11-28）

- FACT：审计模式 socket→scratch 后统计仍为 `min=0/max=3/sum≈0x501`，保存 raw 与之前一致为黑；排除“直接读入 mmap 才坏”。
- 判定：V4L2 客户端从 socket 收到的字节在进入 scratch 前就已不是 helper 统计的有效 frame，或收到了另一路数据。
- 操作：helper 的 `camera|frame|ready` 日志增加前 4096 字节 `head_min/head_max/head_sum`，与 shim 同一次会话的 `DQBUF payload min/max/sum` 精确对账。
- 状态：FAIL。

### 16.223 helper 同帧前缀对账版构建 PASS（2026-11-28）

- FACT：格式检查 PASS；NDK Android 目标构建 PASS；新 helper SHA `58a1911e5bcc907aee835f57b20203a1ef6c7b4d7261c1d0180cf647cfe5a5da`。
- 下一步：备份当前 `7c52946a...` helper，替换并重启到 `58a1911e...`，运行 V4L2 一帧后对账 helper `head_min/head_max/head_sum` 与 shim payload 统计。

### 16.224 helper 同帧前缀对账版部署并启动 PASS（2026-11-28）

- FACT：旧 helper 已备份为 `husk-bridges.pre-16.223`；新 helper SHA `58a1911e5bcc907aee835f57b20203a1ef6c7b4d7261c1d0180cf647cfe5a5da` 已原子替换。
- FACT：新 helper PID `23225`，camera.sock 已重建，mtime `05:31`。
- 下一步：清 logcat 后跑 V4L2 一帧，提取 helper `head_min/head_max/head_sum` 与 shim `DQBUF payload` 对账。

### 16.225 前 4096 字节 helper/shim 完全一致，需改看整帧分块（观察修正，2026-11-28）

- FACT：同一次 V4L2 会话 helper 记录 `head_min=0/head_max=2/head_sum=1538`；shim 记录 `min=0/max=2/sum=0x602(=1538)`，完全一致。
- FACT：helper 同一帧完整 buffer 为 `min=0/max=128/mean=42.88`，说明帧开头暗部本来就可全为 0..2，不能用前 4096 字节判黑。
- 判定：传输至少前 4096 字节正确；必须比较整帧或分块，定位后段是否丢失/错位。
- 下一步：拉回 `husk-v4l2-one.raw`，按 4 KiB/64 KiB 分块统计 Y，并与 helper 整帧 `mean=42.88` 对照。
- 状态：黑帧仍 FAIL，但“helper 发黑”判断已撤回。

### 16.226 重大修正：整帧 mean 被 UV=128 抬高，Y 平面实际上一直是黑的（FAIL，2026-11-28）

- FACT：V4L2 保存的同一帧完整 NV12 统计为 `all: min=0/max=128/mean=42.876`，但拆开后 `Y: min=0/max=3/mean=0.3146`，`UV: min=127/max=128/mean≈128`。
- FACT：helper 同一帧完整统计 `max=128/mean=42.88` 与客户端 full-buffer 数字一致；前 4096 字节也完全一致。说明 V4L2 socket/scratch/mmap 传输没有出现已证明的损坏。
- 判定：此前 `min=0/max=128/mean≈42.9` 被误当成“有效非黑”，实际是 UV 平面固定 128 抬高了整帧均值；Y 平面从 Camera2 selftest 到 camera-client 到 V4L2 都接近纯黑。Snapshot 黑屏至少与源帧 Y=黑一致。
- 影响：camera-client、v4l2-test、camera selftest 和 helper 的帧 PASS 日志都必须改成 Y/UV 分开统计；旧的“Camera2 selftest PASS / camera.sock PASS”若只引用整帧 max/mean，状态改为“传输 PASS，画面内容 FAIL/UNKNOWN”。
- 下一步：拉取 camera-client 和 Camera2 selftest 的 NV12，分别计算 Y/UV，确认是否为同一黑画面；再决定是相机物理遮挡/曝光问题，还是 Camera2 输出异常。
- 状态：FAIL；不能发布“相机可用”。

### 16.227 Camera2 selftest、camera-client、V4L2 的 Y 平面全部一致为黑（FAIL，2026-11-28）

- FACT：同一 1280x720 BACK NV12 的三层样本：
  - Camera2 selftest：Y `min=0/max=3/mean=0.3365`，UV `128`。
  - camera-client：Y `min=0/max=3/mean=0.3277`，UV `128`。
  - V4L2：Y `min=0/max=3/mean=0.3146`，UV `127..128`。
- FACT：三者完整 buffer 的 `max=128/mean≈42.88` 都来自 UV 平面，不是有效图像。
- 判定：Camera2→helper→socket→V4L2 buffer 的数据内容一致；没有证据表明 shim 丢失或篡改画面。真正阻塞上移到 Camera2 输出或物理相机条件。
- 下一步：核对屏幕状态、相机 AppOps/权限、Android 相机隐私/占用状态与闪光灯能力；在受控条件下复测 BACK Y 平面。若打开灯光后 Y 仍恒黑，再查 Camera2 request/输出格式或厂商 HAL。
- 状态：FAIL；相机门禁不能通过。

### 16.228 相机权限/占用/硬件状态正常，准备受控灯光复测（2026-11-28）

- FACT：设备 `mWakefulness=Awake`；Husk AppOp `CAMERA: allow`，当前 uid state `active`。
- FACT：`dumpsys media.camera` 当前没有其他 Active Camera Client；BACK device 0 在测试时能 CONNECT/DISCONNECT，静态信息显示 `Facing: Back`、`Has a flash unit: true`。
- FACT：系统未暴露可用的 camera-privacy 关闭项；未发现隐私开关阻断。
- 判定：权限、App 占用和屏幕状态均正常，但不能据此排除桌面/遮挡或 HAL 输出黑。
- 操作：给 helper 增加仅由 `HUSK_CAMERA_TORCH=1` 启用的 BACK 闪光灯 TORCH 诊断；同分辨率采一帧并比较 Y 平面。测试后关闭灯光/恢复默认启动。
- 状态：待验证。

### 16.229 opt-in TORCH 诊断 helper 构建 PASS（2026-11-28）

- FACT：只有 `HUSK_CAMERA_TORCH` 存在时才给 CaptureRequest 设置 `FLASH_MODE_TORCH(2)`；默认路径不设置。
- FACT：格式检查与 NDK 构建 PASS；新 helper SHA `d21199f67329c5fc3a9a90f652e65a6bc59f18fe2bfdf4b6dd1b9d6d9a92e976`。
- 下一步：备份当前 helper，替换并以 `HUSK_CAMERA_TORCH=1` 重启，运行 BACK camera-client 一帧并看 `torch_rc`、Y 平面和灯光是否变化；测试后恢复默认 helper 环境。

### 16.230 TORCH 诊断 helper 已启动（2026-11-28）

- FACT：helper 原子替换为 SHA `d21199f67329c5fc3a9a90f652e65a6bc59f18fe2bfdf4b6dd1b9d6d9a92e976`；PID `23924`，camera.sock mtime `05:33`；环境含 `HUSK_CAMERA_TORCH=1`。
- 下一步：清 logcat，用 camera-client 取 BACK 1280x720 三帧，确认 `torch_rc=0` 并比较 Y 平面；测试完成后恢复不带 TORCH 的默认启动。

### 16.231 CameraManager TORCH 版 helper 构建 PASS（2026-11-28）

- FACT：撤销请求级 `TAG_FLASH_MODE` 实验；改用 NDK 官方 `ACameraManager_setTorchMode`，符号缺失时只记 `unavailable`，不影响默认路径。
- FACT：格式检查与 NDK 构建 PASS；新 helper SHA `e0abb131c43661d9de876b868fb8d4b9ea9b3e3fbfc4eeb92161879f90fea9f9`。
- 操作：备份当前 helper、暂停并备份 `files/camera-selftest` marker，替换 helper，以 `HUSK_CAMERA_TORCH=1` 启动，确认 `camera|torch|...|rc=0` 后再采帧。
- 状态：待验证。

### 16.232 CameraManager TORCH helper 已落盘但首次启动路径错误（FAIL，2026-11-28）

- FACT：新 helper 已原子替换到 `files/linux/runtime/husk-bridges`；`files/camera-selftest` 已备份为 `camera-selftest.pre-16.231`，启动前无 selftest marker。
- FAIL：启动命令先将 cwd 切到 `files`，又使用 `./files/linux/runtime/husk-bridges`，实际变成 `files/files/...`；日志文件和进程均不存在，camera.sock mtime 仍为旧的 `05:33`。
- 纠正：从 `files` 直接执行 `./linux/runtime/husk-bridges`，日志写到 `linux/runtime/husk-bridges-manager-torch.log`，启动后再核对 SHA、PID、socket 和 `camera|torch` 日志。

### 16.233 该 ROM 不提供 NDK setTorchMode，转测 FRONT 对照（TORCH NOT VERIFIED，2026-11-28）

- FACT：CameraManager 初始化成功，枚举到 BACK id=0 / FRONT id=1；但 `ACameraManager_setTorchMode` 符号不存在，日志 `camera|torch|setTorchMode unavailable`。
- 判定：TORCH 复测不可用，未点亮闪光灯；不能把灯光变化当证据。
- 下一步：用当前 helper 请求 FRONT id=1、1280x720 三帧，拆分 Y/UV 统计。前摄正常则后摄问题缩到物理遮挡或后摄 HAL；两路 Y 都黑则查全局 Camera2 request/输出。
- 状态：TORCH NOT VERIFIED；相机 Y 黑仍 FAIL。

### 16.234 FRONT 1280x720 也为黑，两枚相机一致（FAIL，2026-11-28）

- FACT：FRONT id=1 三帧 full-buffer `max=128/mean≈42.73~42.85`，但 Y 平面为 `min=0/max=4/mean=0.2806`，UV 固定 `128`。
- FACT：证据文件 SHA `c2a02f0b69e3764880937eacfcf02c2bb94bdb0747cbcf85c5b61f98142c7da0`。
- 判定：BACK 与 FRONT 的 Camera2 Y 都黑，不能归因于单个后摄遮挡；下一步用系统相机 App 做独立黑盒对照，区分设备/HAL 条件与 Husk 的 CaptureRequest/ImageReader 路径。
- 状态：FAIL。


### 16.235 厂商相机黑盒对照已启动并截图（NOT VERIFIED，2026-11-28）

- FACT：已启动系统厂商相机 `com.oplus.camera/.component.CameraImageActivity`，当前前台 Activity 为 `com.oplus.camera/.Camera`，PID `17166`；Husk 未修改、未读取该 APK。
- FACT：已保存厂商相机截图 `docs/design/evidence/367-camera-audit-20261128/vendor-camera-preview-20261128.png`，截图中心相机预览区为黑。
- 限制：此截图只能证明当前 screencap 画面；硬件相机预览可能由 SurfaceView/独立图层呈现，screencap 不保证捕获，不能单独据此判定设备相机或 HAL 黑屏。
- 下一步：在厂商相机内触发拍照并检查 MediaStore/DCIM 中生成的最新 JPEG；JPEG 正常则把故障收窄到 Husk 的 Camera2/AImage 路径，JPEG 同样黑才考虑设备相机条件或 HAL 外部阻塞。
- 状态：厂商相机预览黑屏 NOT VERIFIED；Husk 相机 Y 平面黑 FAIL；Husk 0.0.37 仍不可发布。


### 16.236 执行厂商相机 JPEG 黑盒对照（进行中，2026-11-28）

- 目的：让厂商相机自行完成 Camera2/HAL 拍摄与 JPEG 编码，绕开 Husk 的 `AImageReader → to_nv12() → camera.sock → V4L2` 路径。
- 方法：记录拍照前 MediaStore 最新图片；读取当前 UI 快门边界并截图；点击厂商相机快门后等待落盘；按 `date_added/datetaken` 找新 JPEG，复制到证据目录并检查尺寸、哈希和图像内容。
- 边界：不修改厂商 APK、不修 Husk 源码、不据此直接宣称 Husk 相机可用。
- 状态：进行中。


### 16.237 厂商相机 JPEG 同样接近全黑：Husk 不是唯一故障点（FAIL，2026-11-28）

- FACT：点击厂商相机快门后生成新文件 `DCIM/Camera/IMG20261128053653.jpg`，MediaStore `_id=1000000467`、`_size=2825397`、`datetaken=1795815413962`。
- FACT：JPEG 已复制为 `docs/design/evidence/367-camera-audit-20261128/vendor-camera-IMG20261128053653.jpg`，SHA256 `14f8ba7b1fd8ae73f8f0f80b4e0c9a47956fccf9bea40bc12c68c87ab1413e2b`。
- FACT：图像为 4624x3468；RGB extrema 约 `R 0..15 / G 0..14 / B 0..20`，灰度 `min=0/max=15/mean=3.463`；中心区域灰度 `min=0/max=11/mean=2.836`，肉眼为近全黑。
- 判定：厂商相机自己的完整拍摄/编码链路也输出近黑 JPEG。当前不能把 Husk 的 Y 平面黑单独归因于 `to_nv12()`、socket 或 V4L2；设备相机物理条件、镜头遮挡、当前拍摄模式或厂商 HAL 是更强的共同上游嫌疑。
- 边界：这只是当前一次拍摄。还需切换另一枚相机/普通拍照模式复测，不能直接断言硬件损坏。
- 状态：厂商 JPEG 黑 FAIL；Husk 相机 FAIL；外部条件 NOT VERIFIED；0.0.37 不可发布。


### 16.238 切换厂商相机到另一枚摄像头复测（进行中，2026-11-28）

- FACT：当前厂商相机 UI 的 `switch_camera_button` content-desc 为“后置摄像头”，说明首次黑 JPEG 来自当前前摄方向。
- 操作：点击 `com.oplus.camera:id/switch_camera_button` 的中心 `(2061,180)`，截图并 dump UI 确认切换；随后按同一快门流程拍第二张 JPEG，检查其灰度统计。
- 目的：排除只发生在前摄的偶发/模式问题。
- 状态：进行中。


### 16.239 后摄厂商相机正常出图：Husk 后摄黑帧可判定为 Husk 路径缺陷（PASS，2026-11-28）

- FACT：切换后 UI `switch_camera_button` 变为“前置摄像头”，确认当前为后摄；点击快门生成 `IMG20261128053753.jpg`，MediaStore `_id=1000000469`。
- FACT：JPEG 已复制为 `docs/design/evidence/367-camera-audit-20261128/vendor-camera-rear-IMG20261128053753.jpg`，SHA256 `a9f6e36320b3c9747a241d8a29cc95ddd270e470425d77630a41e9d8f638a5a3`，尺寸 4608x3456。
- FACT：后摄 JPEG 灰度 `min=24/max=191/mean=110.989`，中心区域 `min=68/max=190/mean=127.092`；肉眼为正常室内画面。
- 判定：设备后摄与厂商 Camera HAL/JPEG 链路正常；首次前摄黑 JPEG 不能代表后摄。Husk 的 BACK(id=0) Camera2 输出 Y 黑是 Husk 路径或请求配置缺陷，不能再记作设备硬件阻塞。
- 下一步：关闭厂商相机释放设备，用默认 helper（不带 TORCH）重跑 BACK 一帧并抓 `AImage` 格式、plane 数、row/pixel stride、Y/UV 分平面统计；重点审计 `to_nv12()` 与 CaptureRequest 输出配置。
- 状态：厂商后摄 PASS；Husk BACK 黑 FAIL；0.0.37 不可发布。


### 16.240 释放厂商相机并审计 Husk Camera2/AImage 输出配置（进行中，2026-11-28）

- 操作：`am force-stop com.oplus.camera` 释放相机；确认 Husk helper 与 marker 状态；只读审计 `app-rs/src/camera.rs` 中 ImageReader 格式、CaptureRequest 配置、`AImage_getPlane*` 与 `to_nv12()` 的数据路径。
- 目的：用厂商后摄正常这一对照，定位 Husk BACK 为何仍输出 Y≈0。
- 边界：先审计再改，避免用补丁掩盖 AImage 格式/stride 根因。
- 状态：进行中。


### 16.241 审计 Husk ImageReader/CaptureRequest，准备 plane 级取证（进行中，2026-11-28）

- FACT：源码当前创建 `AImageReader(width,height,YUV_420_888,3)`，窗口格式实测为 `35`；使用 `ACAMERA_TEMPLATE_RECORD`，设置 `CONTROL_MODE_AUTO` 与 `CONTROL_AE_MODE_ON`，提交 repeating request。
- FACT：既有日志证明 `session_active`、`request_rc=0`、`repeat_rc=0`、`AImage` 能取得且尺寸为 1280x720；不是相机打不开或不产帧。
- FACT：`to_nv12()` 对 plane 0 按 row stride 直接按行复制，但当前没有记录 AImage format、plane 数、plane data len、row stride、pixel stride，尚不能排除 plane 指针/stride 取错导致读到黑区。
- FAIL：首次抓 logcat 时 zsh 将未加引号的 `Husk:*` 当 glob，命令报 `no matches found`；已改为引号形式重跑，未影响设备。
- 操作：增加仅由 `HUSK_CAMERA_PLANE_AUDIT=1` 启用的 AImage plane 取证日志，记录 format、plane 数、各 plane len/row stride/pixel stride，以及按有效像素边界计算的 Y/U/V min/max/mean；不改变默认帧路径和请求参数。
- 判据：若 plane 0 Y 本身黑，继续比较 PREVIEW/RECORD template 或 HAL 请求；若 plane 0 Y 非黑但 NV12 Y 黑，修 `to_nv12()` 的读取/布局。
- 状态：进行中；Husk BACK 仍 FAIL。


### 16.242 构建 AImage plane 取证 helper（进行中，2026-11-28）

- FACT：已加入 `HUSK_CAMERA_PLANE_AUDIT=1` 门控日志；默认路径仅多一次环境变量判断，不改变 ImageReader、CaptureRequest 或 NV12 数据。
- 操作：`cargo fmt --check` 后以 NDK 28.2.13676358 构建 `husk-bridges` Android arm64 release。
- 判据：构建 PASS 才部署；失败则保留当前 helper，不覆盖设备。
- 状态：进行中。


### 16.243 AImage plane 取证 helper 构建 PASS（2026-11-28）

- FACT：`cargo fmt --check` PASS；NDK 28.2.13676358 Android arm64 release 构建 PASS。
- FACT：产物 `app-rs/target/aarch64-linux-android/release/husk-bridges` 为 AArch64 PIE，SHA256 `4ea5a078743deb8cffd6427c9648150adbc26a4917640f48fa5914f45205de26`。
- 备注：构建有既存警告（未使用项、C extern 签名警告等），无编译错误；本步只证明构建成功，不代表设备行为通过。
- 下一步：备份 PID `24179` 当前 helper，原子部署新 helper，以 `HUSK_CAMERA_PLANE_AUDIT=1` 重启，验证 SHA/PID/socket 后跑 BACK 一帧。
- 状态：构建 PASS；设备 plane 取证进行中。


### 16.244 部署 plane 取证 helper 并以审计环境重启（进行中，2026-11-28）

- 操作：停止当前 PID `24179`；备份 `files/linux/runtime/husk-bridges` 为 `husk-bridges.pre-16.244`；将新 helper 原子替换到正式路径。
- 启动环境：保留 `HUSK_FILES`、`HUSK_ROOTFS`、`HUSK_PARENT_PID=23198`、`NR_V4L2_AUDIT=1`，新增 `HUSK_CAMERA_PLANE_AUDIT=1`，不设置 `HUSK_CAMERA_TORCH`。
- 验证：设备 SHA 必须等于 `4ea5a078...`，helper PID/socket 存在，环境包含 plane audit、不含 torch；否则回滚到备份。
- 状态：进行中。


### 16.245 plane 取证 helper 部署并启动 PASS（2026-11-28）

- FACT：设备正式路径 helper SHA256 `4ea5a078743deb8cffd6427c9648150adbc26a4917640f48fa5914f45205de26`，与构建一致；旧 helper 备份 SHA `e0abb131...`。
- FACT：旧 helper PID `24179` 已停止；实跑 PID `27690`（setsid 包装 PID `27687`），`camera.sock` 于 `05:40` 重建。
- FACT：进程环境确认 `HUSK_CAMERA_PLANE_AUDIT=1`、`NR_V4L2_AUDIT=1`、`HUSK_PARENT_PID=23198`，且无 `HUSK_CAMERA_TORCH`。
- 状态：部署/启动 PASS；下一步跑 BACK 一帧读取 plane audit。


### 16.246 采集 BACK 1280x720 AImage plane 审计帧（进行中，2026-11-28）

- 操作：清空 logcat；通过 `files/linux/guest-correct.sh /tmp/camera-client /home/husk/camera-plane-audit frames=1 w=1280 h=720 id=0` 请求 BACK 一帧，同时读取 helper 的 `camera|image|audit` 与 `camera|frame|ready` 日志，并保存 raw NV12。
- 判据：plane audit 的 Y 平面有效像素若黑，说明源 Camera2 输出或请求模式问题；若 Y 非黑，说明 `to_nv12()` 或后续传输问题。
- 状态：进行中。


### 16.247 camera-client 参数少写 socket，连接失败（FAIL，2026-11-28）

- FAIL：实际命令把输出目录放在 argv[1]，漏了 socket 参数；camera-client 将 `/home/husk/camera-plane-audit` 当作 socket，报 `connect ... No such file or directory`。
- 纠正：正确参数顺序为 `<socket> <out-dir> [frames=...] [w=...] [h=...] [id=...]`，重新执行 `/tmp/camera-client /run/husk/camera.sock /home/husk/camera-plane-audit ...`。
- 状态：本次未取到帧；Husk 相机结论不变，仍 FAIL。


### 16.248 AImage 源 plane 本身就是黑：排除 to_nv12/socket/V4L2（FAIL，2026-11-28）

- FACT：BACK id=0、1280x720、AImage format `35`、plane 数 `3` 均正确；Y plane `len=921600/row=1280/pixel=1`，U/V plane `len=460799/row=1280/pixel=2`。
- FACT：源 Y plane 有效像素统计 `min=0/max=3/mean=0.29`，U=`127..128/mean=128.00`，V=`128/mean=128.00`。客户端 NV12 同样为 `Y min=0/max=3/mean=0.292`。
- FACT：源 plane 与输出 NV12 完全同型，说明 `to_nv12()` 的 stride/plane 索引、socket 传输、V4L2 shim 均不是当前黑帧的首因。
- 判定：阻塞在 Husk 的 Camera2 CaptureRequest/HAL 输出配置或 HAL 对当前 template 的响应；继续测试 `ACAMERA_TEMPLATE_PREVIEW` 与当前 `RECORD` 的单变量差异。
- 证据：`docs/design/evidence/367-camera-audit-20261128/back-plane-audit.log`、`back-plane-audit-frame-00.nv12`（SHA256 `3546e63a49bf18c7fc896079315936d176f929e3662ac14963050be141c36b6f`）。
- 状态：源 Y 黑 FAIL；下一步 PREVIEW/RECORD A/B。


### 16.249 增加 PREVIEW/RECORD 单变量 A/B 开关（进行中，2026-11-28）

- 操作：`HUSK_CAMERA_TEMPLATE_PREVIEW=1` 时创建 `ACAMERA_TEMPLATE_PREVIEW`，否则维持现有 `ACAMERA_TEMPLATE_RECORD`；请求日志增加实际 template。
- 边界：只改变 CaptureRequest 模板，不改 ImageReader、分辨率、AE、NV12 或传输路径。
- 判据：同一 BACK、同一分辨率、同一 helper 版本下，PREVIEW Y 若恢复正常则确认 template 根因；若仍黑则继续查 AE/曝光/镜头/权限。
- 状态：进行中。


### 16.250 构建 PREVIEW/RECORD A/B helper（进行中，2026-11-28）

- FAIL：代码替换后首次 `cargo fmt --check` 因两处换行不符合 rustfmt 返回失败；源码逻辑未运行。
- 纠正：执行 `cargo fmt` 后重新 `cargo fmt --check`，再以 NDK 构建 Android arm64 release。
- 判据：构建通过才部署；默认仍为 RECORD，测试时仅额外设置 `HUSK_CAMERA_TEMPLATE_PREVIEW=1`。
- 状态：进行中。


### 16.251 PREVIEW/RECORD A/B helper 构建 PASS（2026-11-28）

- FACT：`cargo fmt` 后 `cargo fmt --check` PASS；NDK arm64 release 构建 PASS。
- FACT：新 helper SHA256 `6b3f83b28ca88c9e44aae90e1ecbdbd697f296e34f5614e446ce3e7ea76c5c40`。
- 备注：仍只有既存警告；默认 template 仍为 RECORD，只有设置 `HUSK_CAMERA_TEMPLATE_PREVIEW=1` 才走 PREVIEW。
- 下一步：部署并只带 PREVIEW 开关重跑同一 BACK/1280x720/1 帧，比较 Y plane。
- 状态：构建 PASS；A/B 待运行。


### 16.252 部署 PREVIEW A/B helper 并重启（进行中，2026-11-28）

- 操作：停止 PID `27690`；备份当前 helper；原子部署 SHA `6b3f83b2...`；以 `HUSK_CAMERA_TEMPLATE_PREVIEW=1`、`HUSK_CAMERA_PLANE_AUDIT=1`、`NR_V4L2_AUDIT=1` 启动，不设置 TORCH。
- 验证：设备 SHA、进程环境、socket 和 `request_setup|template=1` 日志必须一致；失败则回滚 `husk-bridges.pre-16.252`。
- 状态：进行中。


### 16.253 PREVIEW A/B helper 部署并启动 PASS（2026-11-28）

- FACT：设备正式路径 SHA256 `6b3f83b28ca88c9e44aae90e1ecbdbd697f296e34f5614e446ce3e7ea76c5c40`；旧 `4ea5a078...` 备份为 `husk-bridges.pre-16.252`。
- FACT：实跑 PID `27966`，`camera.sock` 于 `05:43` 重建；环境含 `HUSK_CAMERA_TEMPLATE_PREVIEW=1`、`HUSK_CAMERA_PLANE_AUDIT=1`、`NR_V4L2_AUDIT=1`，无 TORCH。
- 下一步：同 BACK/1280x720/1 帧运行 camera-client，确认日志 `template=1`，比较源 Y 与 NV12 输出 Y。
- 状态：部署/启动 PASS；A/B 运行进行中。


### 16.254 运行 BACK PREVIEW 单变量帧（进行中，2026-11-28）

- 操作：清 logcat；通过 `guest-correct.sh /tmp/camera-client /run/husk/camera.sock /home/husk/camera-preview-audit frames=1 w=1280 h=720 id=0` 请求一帧；保存 `camera|session|request_setup template`、`camera|image|audit`、`camera|frame|ready`。
- 判据：`template=1` 且源 Y 从 `max=3/mean=0.29` 变为正常纹理，才是真正 PREVIEW 修复；若仍黑则 FAIL 并继续查请求/AE。
- 状态：进行中。


### 16.255 PREVIEW 仍然黑，排除 RECORD template 单独根因（FAIL，2026-11-28）

- FACT：日志确认 `camera|session|request_setup|template=1`；源 Y `min=0/max=3/mean=0.38`，U/V 仍约 128；输出 NV12 Y `mean=0.384`。
- FACT：证据 `back-preview-audit.log`，raw SHA256 `2672b315f53ab91a5978f7f2048ec51583d91778eeaa640beefe54bf6566143d`。
- 判定：把请求模板从 RECORD 改成 PREVIEW 不能修复黑帧；继续查 auto-exposure 收敛/首帧时机、session ready/active 等待，或 HAL 对 NDK 请求的处理。
- 下一步：同一 PREVIEW helper 连续请求 30 帧，按 Y 平面逐帧统计，判断是否存在“首帧黑、后续收敛”。
- 状态：PREVIEW FAIL；A/B 结论为 template 非单独根因。


### 16.256 连续 30 帧仍未收敛，排除单纯首帧/AE 延迟（FAIL，2026-11-28）

- FACT：PREVIEW、BACK、1280x720 连续 30 帧耗时 2156 ms；Y plane mean 从 `1.749` 仅升到 `2.035`，max 从 `11` 到 `7` 左右，始终是近黑噪声。
- FACT：客户端 elapsed 2.156 s，seq 1..30 完整，无 DQBUF/协议失败；helper capture started/completed 计数到 30。
- 判定：不是只取首帧太早，也不是简单 2 秒内 AE 收敛延迟；整条回放持续接近零曝光或 HAL 输出无效。
- 下一步：单变量改变输出分辨率（1920x1080 与 640x480），排除 1280x720 stream 配置本身的问题。
- 证据：`back-warmup-30-client.log`、`back-warmup-30-helper.log`、`back-warmup-30/`。
- 状态：30 帧持续黑 FAIL。


### 16.257 分辨率单变量对照 1920x1080 / 640x480（进行中，2026-11-28）

- 操作：保持同一 PREVIEW helper、同一 BACK id=0、同一 AE 设置，分别请求 1920x1080 与 640x480 各 3 帧；清 logcat 前后分别保存 helper/client 日志并比较源 Y。
- 判据：若某分辨率恢复有效纹理，定位到窗口/AImageReader stream config；若两档仍黑，继续查 request/3A/HAL。
- 状态：进行中。


### 16.258 多分辨率均黑，排除单一 1280x720 stream 配置（FAIL，2026-11-28）

- FACT：请求 `1920x1080` 时 helper 实际协商为 `1600x720`；3 帧源 Y `max=12/7/8`、`mean=1.90/2.02/2.03`，仍黑。证据 `back-1600x720-*`。
- FACT：请求 `640x480`；3 帧源 Y `max=14/9/7`、`mean=1.82/1.96/1.96`，仍黑。证据 `back-640x480-*`。
- 判定：不是 1280x720 单档 stream 配置；Camera2 NDK 在多种输出尺寸上持续给出近零 Y，继续查 3A/exposure/capture result 与 HAL request 参数。
- 状态：分辨率单变量 FAIL，相机仍不可发布。


### 16.259 增加 CaptureResult 3A/exposure 取证（进行中，2026-11-28）

- 现状：源 AImage 在 640x480、1280x720、实际 1600x720 均为近黑；仅靠模板/分辨率/30 帧时机已排除常见 stream 问题，但还不知道 HAL 实际采用的曝光时间、ISO、AE 状态。
- 操作：在 capture progressed/completed 回调中读取 `SENSOR_EXPOSURE_TIME`、`SENSOR_FRAME_DURATION`、`SENSOR_SENSITIVITY`、`SENSOR_TIMESTAMP`、`CONTROL_AE_STATE`、`CONTROL_AE_MODE`、`CONTROL_MODE`，仅在 `HUSK_CAMERA_PLANE_AUDIT=1` 记录前几帧；不改请求和默认路径。
- 判据：若曝光时间/ISO 为 0 或 AE 状态长期 SEARCHING/INACTIVE，查 3A/request；若曝光正常但 Y 仍黑，查 HAL/ISP 或 stream 消费端。
- 状态：进行中。


### 16.260 CaptureResult 3A/exposure 取证 helper 构建 PASS（2026-11-28）

- FACT：`cargo fmt` + `cargo fmt --check` PASS；NDK arm64 release 构建 PASS。
- FACT：产物 SHA256 `8a1a1d2851a320418f0d08a046f6eec76ee7869fdcc8e3f33618f5de7a4b3faa`；既有证书警告不变，无错误。
- 下一步：部署后以 PREVIEW + plane audit 启动，跑 BACK 3 帧并读取 capture result metadata。
- 状态：构建 PASS。


### 16.261 部署 3A 取证 helper（进行中，2026-11-28）

- 操作：停止 PID `27966`，备份 `6b3f83b2...`，原子部署 `8a1a1d28...`；以 PREVIEW、plane audit、V4L2 audit 启动，无 TORCH。
- 验证：设备 SHA、PID、socket、环境变量；然后同 BACK/1280x720 跑 3 帧。
- 状态：进行中。


### 16.262 3A 取证 helper 部署并启动 PASS（2026-11-28）

- FACT：设备 SHA `8a1a1d2851a320418f0d08a046f6eec76ee7869fdcc8e3f33618f5de7a4b3faa`，旧 `6b3f83b2...` 备份；PID `28607`，socket 于 `05:46` 重建。
- FACT：环境含 PREVIEW、plane audit、V4L2 audit、parent PID，且无 TORCH。
- 下一步：同 BACK/1280x720 跑 3 帧，提取 `camera|capture|result` 的 exposure/ISO/AE 与源 Y。
- 状态：部署 PASS。


### 16.263 采集 BACK 3A/capture result（进行中，2026-11-28）

- 操作：清 logcat，BACK/1280x720/3 帧；保存 capture progressed/completed、image audit、frame ready 完整日志；不保存多余 raw。
- 判据：exposure/ISO/AE 数值决定下一步是 request/3A 还是 HAL/ISP。
- 状态：进行中。


### 16.264 CaptureResult 显示 3A 已拉满但场景仍被判为极暗（FAIL，2026-11-28）

- FACT：BACK 1280x720 PREVIEW capture result：`sensor_exposure=59,998,000 ns`（约 60 ms）、`sensor_frame_duration=60,271,084 ns`、`sensor_sensitivity=15996`、`ae_state=4`（FLASH_REQUIRED）、`ae_mode=1`、`control_mode=1`。
- FACT：同一帧源 Y `max=9/mean=1.67`，下一帧 `mean=1.81`、再下一帧 `1.84`。
- 判定：3A 并非没有运行；HAL 实际选择了很长曝光和很高 ISO，仍判定场景需要闪光，说明 Camera2 看到的 sensor/ISP 数据极暗。下一步必须核实厂商相机前后摄与 Android camera device 的真实映射，不能继续假设 UI 文案的方向。
- 新现场：`dumpsys media.camera` 显示共有 5 个 HAL camera devices，其中 normal 4、public API1 2；Husk manager 只枚举到 id 0/1。必须查 hidden device 与厂商 app 实际连接的 device id。
- 状态：3A 结果 FAIL；相机不可发布。


### 16.265 厂商相机前后摄与 CameraService device id 黑盒映射（进行中，2026-11-28）

- 操作：复制当前完整 `dumpsys media.camera` 到证据目录；重新启动厂商相机，记录启动后的 `CONNECT device N`；在不清除 UI 文案歧义的情况下分别拍摄两张图，并从 CameraService event log 读取每次切摄后的 device id。
- 判据：以 CameraService 的 `CONNECT/DISCONNECT device N` 和图片亮度为硬证据，建立“厂商 UI 方向 → API camera id → 图像是否有效”的映射；不再根据“后置摄像头/前置摄像头”按钮文案猜测当前方向。
- 状态：进行中。


### 16.266 厂商相机 device 0/1 黑盒映射确认（FACT，2026-11-28）

- FACT：厂商启动时 CameraService `CONNECT device 0`；UI `switch_camera_button` 显示“后置摄像头”；拍得 `IMG20261128054755.jpg`，灰度 `min=0/max=12/mean=2.777`，近黑。
- FACT：点击切换后事件为 `DISCONNECT device 0` → `CONNECT device 1`；UI 显示“前置摄像头”；拍得 `IMG20261128054807.jpg`，灰度 `min=22/max=196/mean=106.773`，正常室内图。
- 结论：CameraService/API device 0 是当前“后置”方向且现场近黑；device 1 是“前置”方向且正常出图。按钮文案表示当前方向，不是点击目标。
- 新问题：此前 Husk id=1/FRONT 也得到黑帧，与厂商 device 1 正常矛盾；必须在同一物理现场重新用当前 3A 取证 helper 跑 id=1，确认 Husk 的 FRONT 是否仍黑。
- 证据：`dumpsys-media-camera-vendor-launch.txt`、`dumpsys-media-camera-vendor-switched.txt`、两张厂商 JPEG 与 SHA。
- 状态：厂商映射 FACT；Husk 前后摄仍 FAIL。


### 16.267 同现场复测 Husk FRONT id=1（进行中，2026-11-28）

- 操作：`am force-stop com.oplus.camera` 并等待释放；用当前 3A 取证 helper 请求 FRONT id=1、1280x720、3 帧；保存 capture result、源 Y 和输出 NV12。
- 判据：若 Husk FRONT 有正常纹理，则当前阻塞主要是后置方向物理条件；若 Husk FRONT 仍黑但厂商 FRONT 正常，则确定 Husk Camera2 NDK/request 路径还有独立缺陷。
- 状态：进行中。


### 16.268 同现场 Husk FRONT 仍黑，确认存在独立 Husk 回归（FAIL，2026-11-28）

- FACT：厂商 device1/FRONT 在 `05:48:07` 拍摄 JPEG 灰度 mean `106.773`；`05:48:32` 同一现场 Husk id=1/1280x720 三帧源 Y `mean=1.83/1.90/1.89`，仍近黑。CameraService 事件确认 Husk 连接 device 1。
- 判定：黑帧不能归因于当前物理遮挡；当前 Husk Camera2 session 与历史 Evidence 93 的正常前摄结果之间存在源码/控制面回归。
- 差异假设：当前调试版额外执行 `ANativeWindow_setBuffersGeometry`、`AImageReader_setImageListener`，并给 `setRepeatingRequest` 传入 capture callbacks；历史成功路径 `88800a6` 没有这些控制面调用。
- 操作：增加 `HUSK_CAMERA_LEGACY_SESSION=1` 单变量开关，跳过上述三项、以 null capture callbacks 提交；ImageReader 格式/尺寸、request 参数、to_nv12 和 socket 不变。
- 判据：同现场由门控版跑 FRONT 3 帧；若恢复，再二分三项控制面；若仍黑，继续查 HAL/camera id/request。
- 状态：独立 Husk 回归 FAIL；legacy 控制面 A/B 进行中。


### 16.269 构建 legacy session 控制面 A/B helper（进行中，2026-11-28）

- 修正实验范围：`HUSK_CAMERA_LEGACY_SESSION=1` 不仅跳过 window geometry、ImageReader listener、capture callbacks，也跳过 session_active 的 200ms 等待，以更接近历史成功路径。
- FACT：rustfmt 已完成；下一步 NDK arm64 release 构建并核对 SHA。
- 状态：进行中。


### 16.270 legacy session A/B helper 构建 PASS（2026-11-28）

- FACT：`cargo fmt --check` 与 NDK arm64 release 构建 PASS；SHA256 `be80d7c2cb61256986ced639875f9149a2d6dcfd57e487ab3b4bd8165a5a517e`。
- 下一步：部署后以 `HUSK_CAMERA_LEGACY_SESSION=1`、plane audit、RECORD template 启动，同现场跑 FRONT 3 帧。
- 状态：构建 PASS。


### 16.271 部署 legacy session A/B helper（进行中，2026-11-28）

- 操作：停止当前 PID `28607`；备份当前 `8a1a1d28...`；原子部署 `be80d7c2...`；以 `HUSK_CAMERA_LEGACY_SESSION=1`、`HUSK_CAMERA_PLANE_AUDIT=1`、RECORD template 启动，无 TORCH。
- 验证：设备 SHA、环境、socket；运行 FRONT id=1/1280x720/3 帧并保存 source Y 统计。
- 状态：进行中。


### 16.272 legacy 控制面仍黑，转进程/包身份 A/B（FAIL，2026-11-28）

- FACT：legacy、RECORD、FRONT id=1、1280x720：源 Y `max=5/4/3`、`mean=0.26/0.03/0.03`，仍黑；`legacy_session=true`、geometry/listener rc=-1、null capture callbacks。
- 判定：window geometry、ImageReader listener、capture callbacks、active 等待均不是黑帧根因。
- 新假设：厂商 CameraService 对黑盒宿主 `com.linuxhub.myth.husk`（Myth Prometheus）的相机 UID 可能返回隐私黑帧/无权数据；历史 Evidence 93 成功来自 Husk 自有包 `com.linuxhub.husk`。
- 操作：用同一设备、同一现场，启动 Husk 自有 `com.linuxhub.husk` v0.0.37，写 `files/camera-selftest=frames=3 camera=1`，拉回 selftest raw 并计算 Y。只作黑盒 A/B，不改另一个仓库源码。
- 判据：自有包前摄非黑而 `.myth.husk` 同现场黑，则定位为宿主身份边界；两包都黑则继续查公共 runtime/HAL。
- 状态：legacy FAIL；包身份 A/B 进行中。


### 16.273 自有包 selftest 首次启动进入权限页（FAIL，2026-11-28）

- FACT：`com.linuxhub.husk/android.app.NativeActivity` 启动结果被 `com.android.permissioncontroller/.permission.ui.GrantPermissionsActivity` 接管；应用进程未执行 selftest，`files/camera-frames` 为空。
- FACT：CAMERA 已 granted，但 RECORD_AUDIO 未 granted；不能把权限页当成相机结果。
- 纠正：授予 Husk 自有包 CAMERA/RECORD_AUDIO 及 CAMERA AppOp，确认前台回到 NativeActivity 后再重跑 selftest；若权限页仍出现，记录并停止该 A/B。
- 状态：自有包 A/B 未运行；Husk 相机仍 FAIL。

### 16.274 恢复现场并固定逐步留证规则（进行中，2026-11-28）

- FACT：当前 Husk 工作区未提交改动仍在，`app-rs/src/camera.rs` SHA256 `37d54381f459f5c2537a963655f9b30db32859b274d9d7c2b199539bfd392c79`。
- FACT：设备 `UKHMMB8PYTGECMNV` 当前顶层为 `com.android.permissioncontroller/.permission.ui.GrantPermissionsActivity`；`com.linuxhub.husk` PID `24596`、`com.linuxhub.myth.husk` PID `22124`、`husk-bridges` PID `30734` 均存活。
- 操作规则：从本条开始，每个有状态变化的操作先写计划，执行后补 FACT/FAIL/UNVERIFIED，再进入下一步；禁止 commit/push/tag。
- 下一步：先 dump 当前权限页并确认被请求的权限，不盲点；完成自有包前台验证后再重跑前摄 selftest。
- 状态：现场恢复 PASS；相机门禁仍 FAIL。

### 16.275 dump 当前权限页与前台窗口（进行中，2026-11-28）

- 操作：读取 `uiautomator` XML、当前 focus 和 top resumed activity；确认具体权限名、可点击区域和是否确有 Camera 权限项。
- 状态：进行中。

### 16.276 权限页确认请求的是大致位置（FACT，2026-11-28）

- FACT：UI XML 标题为“要允许‘Husk’获取此设备的大致位置信息吗？”，按钮为“使用时允许/仅本次使用时允许/拒绝”，没有 Camera 权限项。
- 判定：CAMERA/RECORD_AUDIO 已单独授予；当前阻塞自有包进入前台的是定位权限对话框。
- 下一步：点击“使用时允许”中心 `(1260,615)`，随后重新验证 top resumed activity 必须是 `com.linuxhub.husk/android.app.NativeActivity`。
- 状态：权限页识别 PASS；相机仍 FAIL。

### 16.277 自有包已真正进入前台（PASS，2026-11-28）

- FACT：点击“使用时允许”后，`topResumedActivity=com.linuxhub.husk/android.app.NativeActivity`，`mCurrentFocus`/`mFocusedApp` 均为同一 Activity；PID `24596` 保持存活。
- FACT：`dumpsys package` 显示 CAMERA/RECORD_AUDIO 在当前用户为 granted；粗略位置对话框的授权动作已完成。
- 判定：自有包前台状态已可验证，之前 selftest 的“前台 NOT VERIFIED”阻塞解除。
- 下一步：确认 selftest 触发接口与帧输出目录，清空旧帧后在前台重跑 3 帧前摄。
- 状态：前台身份验证 PASS；相机画质仍 FAIL。

### 16.278 核对自有包 selftest 触发协议（进行中，2026-11-28）

- 操作：只在 Husk 源码内搜索 `camera-selftest`、`camera-frames` 的读取/写入实现，确认 run-as 路径、触发时机和帧命名。
- 状态：进行中。

### 16.279 selftest 触发协议核对（FACT，2026-11-28）

- FACT：`app-rs/src/camera.rs` 启动时读取 `files/camera-selftest`，解析 `frames=` 与 `camera=`，默认 10 帧；输出到 `files/camera-frames/`。
- FACT：该 selftest 在 camera bridge 启动前执行，因此必须先写 marker，再 force-stop/重启 `com.linuxhub.husk`，普通热切前台不会重跑。
- 下一步：清空 `files/camera-frames`，写入 `frames=3 camera=1`，重启自有包，确认顶层仍为 NativeActivity，再收集 3 帧和日志。
- 状态：触发协议 FACT；待执行。

### 16.280 自有包前台 selftest 生成 3 帧（PASS for execution，画质未定）

- FACT：写入 `frames=3 camera=1` 后冷启动总耗时 `250 ms`；15 秒后顶层仍为 `com.linuxhub.husk/android.app.NativeActivity`。
- FACT：`files/camera-frames/frame-00.nv12`、`frame-01.nv12`、`frame-02.nv12` 均为 `1,382,400` 字节，正好对应 1280x720 NV12；selftest 不再被权限页拦截。
- FACT：日志确认 Camera2 session/output/add 均返回 0，并继续建立 V4L2 两设备树。
- 下一步：把三帧和 selftest 日志写入证据目录，计算 Y/U/V 统计与 SHA256；只有 Y 非黑才算相机 PASS。
- 状态：selftest 执行 PASS；画质待判。

### 16.281 自有包前台仍为黑帧（FAIL，2026-11-28）

- FACT：前台 selftest 三帧 Y 平面统计分别为 `0/6/0.2815`、`0/5/0.0971`、`0/4/0.0969`；U/V 均接近 128，典型黑帧。
- FACT：Camera2 `repeat_rc=0`、capture callback completed，30.01 fps；但每次首轮 acquire 返回 `-30001`，随后 recovered，不能解释三帧全黑。
- FACT：证据已落盘：`docs/design/evidence/367-camera-audit-20261128/own-app-front-foreground/`，三帧 SHA256 分别为 `961d43aa...`、`7d06dd26...`、`6c8fef32...`。
- 判定：自有包已在真正前台，仍黑；排除“权限页遮住导致黑帧”的解释。当前缺陷位于 Husk Camera2 输出或 HAL/隐私策略消费路径。
- 下一步：对比历史成功提交 `88800a6` 与当前 camera.rs，检查是否有未被 legacy 开关覆盖的行为差异；再抓前台 session 期间的 CameraService/HAL error。
- 状态：相机门禁 FAIL；发布禁止。

### 16.282 对比历史成功路径与当前 camera.rs（进行中，2026-11-28）

- 操作：只读查看 camera.rs 的提交历史、`88800a6` 对应版本与当前 diff；重点找 legacy 开关未覆盖的 request/stream/permission/metadata 差异，不修改文件。
- 状态：进行中。

### 16.283 历史成功路径对比结果（FACT，2026-11-28）

- FACT：`88800a6` 与当前主路径都使用 `AImageReader_new(..., YUV_420_888, 3)`、同一 output container、`ACAMERA_TEMPLATE_RECORD`、`CONTROL_MODE_AUTO + AE_MODE_ON` 和 repeating request；`legacy_session=1` 已跳过当前新增的 geometry/listener/capture callbacks/active 等待。
- FACT：当前新增的 `start_helper_binder_thread_pool()` 只在 standalone helper `start_once_at()` 调用，自有 NativeActivity 路径不调用；不能解释自有包黑帧。
- FACT：当前相对历史版本的主要新增是取证日志、回调统计和 helper session ownership，不改变 selftest 的 AImageReader 格式/尺寸/缓冲数。
- 判定：没有发现 legacy 开关未覆盖的源码控制面差异；继续改 to_nv12/socket 没有证据支持。下一步转向 session 活跃期间的 CameraService/HAL error。
- 状态：历史对比完成；根因未定。

### 16.284 抓取活跃 session 的 CameraService/HAL 错误（进行中，2026-11-28）

- 操作：写 `frames=60 camera=1` 冷启动自有包；并行轮询 `dumpsys media.camera`，并采集 CameraService/Camera3/HAL/camera 相关 logcat，覆盖请求提交、帧到达和 session close。
- 判据：必须看到 `IN-FLIGHT->ERROR`、buffer error、request timeout 或明确 privacy/denial 之一；只有会话结束后的空 dump 不算证据。
- 状态：进行中。

### 16.285 活跃前台 session 的 HAL 帧计数与错误边界（FACT，2026-11-28）

- FACT：`media-camera-00..06` 捕获到活跃 client：Camera ID 1、PID 31295、包名 `com.linuxhub.husk`；ImageReader consumer 为 `ImageReader-1280x720f23u3m3-31295-0`，格式 `0x23`，HAL 帧数从 0 增至 53，证明 HAL 在持续产帧。
- FACT：`requestNo 62..65` 在 05:55:24.835–.850 出现 `YCrCb_420_SP:0|SW_READ_OFTEN IN-FLIGHT->ERROR`、`Request Error`；时间对应 60 帧完成后的 session 停止阶段，不是首帧失败。不能拿这批停止期错误解释此前 60 帧全黑。
- FACT：当前前台 session 的 3A 结果仍为高曝光/高 ISO；Y 平面从首帧即黑，HAL 的 buffer 计数正常。
- 判定：问题已收敛为“HAL 产出 YUV buffer 但 CPU 可见内容为空/黑”，下一步做 AImageReader usage 单变量：加入 GPU texture usage，验证 MTK HAL 是否走了另一条 buffer 路径。
- 证据：`docs/design/evidence/367-camera-audit-20261128/own-app-front-active-errors/`。
- 状态：HAL 产帧 FACT；画质 FAIL；根因待 A/B。

### 16.286 AImageReader GPU usage 单变量实验（进行中，2026-11-28）

- 假设：MTK HAL 对纯 CPU-read `YUV_420_888` ImageReader 可能分配/写回错误 buffer；厂商流同时带 `HW_TEXTURE`，加入 GPU sampled usage 可能切到可用写回路径。
- 操作：保留 CPU_READ_OFTEN，新增可选 `AImageReader_newWithUsage(..., CPU_READ_OFTEN|GPU_SAMPLED_IMAGE, ...)`；只让 selftest marker 的 `usage=gpu` 启用，生产默认仍走原 `AImageReader_new`。
- 判据：同一前台、同一 id=1、同一 1280x720、同一 request 下，GPU usage 三帧 Y 若恢复非黑则假设成立；若仍黑则回退该实验并继续查 vendor/private stream 差异。
- 状态：代码修改前先记录，随后构建/安装/实测。

### 16.287 AImageReader GPU usage 开关实现与静态检查（PASS，2026-11-28）

- FACT：`app-rs/src/camera.rs` 现有 `AImageReader_newWithUsage` 可选绑定；仅 marker 含 `usage=gpu` 时使用 `CPU_READ_OFTEN|GPU_SAMPLED_IMAGE = 0x103`，默认路径仍为 `AImageReader_new`。
- FACT：设备 `/system/lib64/libmediandk.so` 导出 `AImageReader_newWithUsage`。
- FACT：`cargo fmt --check` PASS；Android arm64 `cargo check --lib` PASS（19 条既有 warning，无新增错误）。修改前备份 `/tmp/camera.rs.pre-gpu-usage-16.286`，新文件 SHA256 `be2d5c38bde2928b52796e065031c0259c32d6e4c175b8a5ecd2fc06c3ea0ae4`。
- 下一步：构建 release `libhuskapp.so`、打包 APK、安装后以 `frames=3 camera=1 usage=gpu` 前台重跑。
- 状态：静态验证 PASS；设备画质待测。

### 16.288 GPU usage A/B APK 构建 PASS（2026-11-28）

- FACT：Android arm64 release `libhuskapp.so` 构建 PASS，SHA256 `e2a74c798ee8ad76d60fc0fd7ff5fa75bd8434ee7b12d847f495226ad0790177`；APK `assembleDebug` PASS，SHA256 `d5536b4a5ca848f21ee5b17e2a820c6071e370566a02cb4c4b46888f5a8465b6`。
- 说明：构建仍有既有 warning，但无错误；没有用构建成功冒充运行成功。
- 下一步：`adb install -r` 保留数据安装，核对包 version，写入 `frames=3 camera=1 usage=gpu`，前台冷启动并统计 Y。
- 状态：构建 PASS；设备 A/B 待跑。

### 16.289 GPU sampled usage 仍黑（FAIL，2026-11-28）

- FACT：marker `frames=3 camera=1 usage=gpu` 实际走了 `AImageReader_newWithUsage`，日志 `usage=0x103 rc=0`；repeating `rc=0`，30.01 fps。
- FACT：三帧 Y 仍为 `0/4/0.2841`、`0/3/0.1056`、`0/3/0.1053`，U/V=128；与默认 CPU reader 无实质差异。证据目录 `own-app-front-gpu-usage/`。
- 判定：单纯增加 GPU sampled usage 不是修复；回退这条实验代码，不留在生产路径。
- 下一步：回退前先做同一分钟厂商前摄黑盒对照，确认此刻物理场景仍能产生正常图；然后恢复 `camera.rs` 原 SHA `37d54381...`。
- 状态：GPU usage FAIL；相机 FAIL。

### 16.290 同一分钟厂商前置仍正常，物理场景/硬件排除（FACT，2026-11-28）

- FACT：05:59:30 厂商相机 UI 明确显示 `前置摄像头`，快门后生成 4608x3456 JPEG；灰度 `min=21/max=197/mean=111.2436`，肉眼可见墙面、空调与挂架等纹理。
- FACT：该照片 SHA256 `ead4c7d003e89859fe56a319168f71a1cc4b4681a1219e38d190f7704c030257`；证据 `vendor-device1-same-minute-IMG20261128055930.jpg`。
- 判定：同一分钟内物理前摄、CameraService device1 和场景都正常；Husk 自有包 Camera2 NDK 的 YUV buffer 黑不是遮镜头、熄屏、锁屏或物理硬件损坏。
- 状态：物理场景 PASS；Husk 相机 FAIL。

### 16.291 GPU usage 实验代码已回退（PASS，2026-11-28）

- FACT：厂商相机已 force-stop；`camera.rs` 从实验前备份恢复，SHA256 回到 `37d54381f459f5c2537a963655f9b30db32859b274d9d7c2b199539bfd392c79`；`cargo fmt --check` PASS。
- 说明：设备上当前安装 APK 仍是实验构建；后续正式复测前必须重建并安装恢复后的源码，避免用实验包冒充最终包。
- 下一步：转向 Camera2 request/stream 单变量与厂商私有流的差异，优先验证 `TEMPLATE_STILL_CAPTURE`/JPEG 是否能拿到正常图像，以区分 YUV stream 与 sensor 输入。
- 状态：实验回退 PASS；相机仍 FAIL。

### 16.292 CPU_READ_RARELY 缓存路径单变量实验（进行中，2026-11-28）

- 假设：MTK HAL 的 dmabuf 写入后，`CPU_READ_OFTEN` 的缓存映射可能让 CPU 读到未失效的零页；改用 `CPU_READ_RARELY` 可能触发不同的 uncached/一致性映射。
- 操作：临时重新加入 marker `usage=rare`，仅 selftest 使用 `AImageReader_newWithUsage(..., CPU_READ_RARELY=2, ...)`；默认路径不变。
- 判据：同前台、同 id=1、同分辨率三帧，若 Y 恢复则保留“reader cache/usage 配置修复”；仍黑则回退并排除 usage 方向。
- 状态：修改前记录，随后构建安装实测。

### 16.293 CPU_READ_RARELY 仍黑（FAIL，2026-11-28）

- FACT：`usage=rare` 实际走 `AImageReader_newWithUsage`，日志 `usage=0x2 rc=0`；三帧 Y 为 `0/3/0.2584`、`0/3/0.0252`、`0/2/0.0252`，U/V=128。
- 判定：CPU cache mapping/usage 方向不成立；回退实验代码。持续全黑且 U/V 为中性值，更像 HAL 输出合成空帧，而不是普通曝光不足。
- 下一步：构建历史成功提交 `88800a6` 的 exact APK 做设备 A/B；若历史同样黑，则问题在设备/系统状态；若历史正常，则当前 APK 仍有未定位回归。
- 状态：usage A/B FAIL；相机 FAIL。

### 16.294 历史成功提交 exact APK A/B 准备（PASS，2026-11-28）

- FACT：`usage=rare` 实验已回退，当前 `camera.rs` SHA 回到 `37d54381...`；新建 detached worktree `/tmp/husk-historical-88800a6`，HEAD `88800a6fe7482b9697b374f317f079ca841b042e`，未提交、未改动主工作区。
- 操作：在该 worktree 用同一 NDK `28.2.13676358` 构建 exact APK，设备安装后写 `frames=3 camera=1`，与当前版做同现场 A/B；测试后必须重装当前源码恢复后的包。
- 判据：历史版非黑则当前版本仍有回归；历史版也黑则设备/系统状态或 ROM 策略变化，继续查 HAL 空帧来源。
- 状态：准备 PASS；构建待跑。

### 16.295 历史 exact APK 构建 PASS（含一次 SDK 环境失败，2026-11-28）

- FAIL：历史 worktree 首次 Gradle 构建报 `SDK location not found`，未产出可作为证据的新 APK。
- PASS：显式设置 `ANDROID_HOME/ANDROID_SDK_ROOT=/Users/l/Library/Android/sdk` 后 `:app:assembleDebug` PASS；使用提交内记录的原生件 `libhuskapp.so` SHA `63967cd77dfcfd09a42ef223623827d50a58c333b4ee0f956fd1f96a3f18cb34`、`libld.so` SHA `0f3eca76af7cac8258258ea1e6d8932f070034dbd8e913cc19d29797bd13c229`。
- 下一步：安装历史 APK，核对 versionName，写 `frames=3 camera=1` 前台冷启动，统计 Y。
- 状态：构建 PASS；设备 A/B 待跑。

### 16.296 历史成功提交 exact APK 仍为黑帧（FACT，2026-11-28）

- FACT：安装 exact `88800a6` APK（versionName `0.1`，APK SHA `8eb2eccb61f27def9aa11efad457ec181a1a74f4ae9f53e6b1f6a93782ef6842`），前台冷启动 `300 ms`；Camera2 重复请求 `rc=0`、30 fps。
- FACT：三帧 Y 仍为 `0/6/0.2673`、`0/5/0.0520`、`0/4/0.0522`，U/V=128。证据目录 `historical-88800a6-front/`。
- 判定：当前未提交 camera.rs 改动不是根因；同一 APK 在 11-23 成功、11-28 同机同镜头变黑，差异在设备/ROM 运行状态或 HAL 策略，而不是当前源码分支。
- FACT：日志出现 `mtkcam-SensorSettingPolicy4cell ... ThirdPkgName com.oplus.camera, camId 1, maxYuvSize 1280x720`，与当前调用包 `com.linuxhub.husk` 不一致；需要核查 ROM 的第三方相机包名/场景策略。
- 下一步：以厂商相机黑盒启动为对照，观察 `oplus.camera.packname` 和 `oplus_camera_3rd_activity` 是否变化；不修改系统设置，先取证。
- 状态：历史 A/B FAIL；ROM/HAL 策略成为主假设。

### 16.297 厂商相机启动会设置 `oplus.camera.packname`（FACT，2026-11-28）

- FACT：厂商相机前台时 `getprop oplus.camera.packname` 返回 `com.oplus.camera`；force-stop 后恢复为空。`oplus_camera_3rd_activity` 在厂商相机运行期间仍为 `0`，不是有效区分项。
- FACT：厂商进程多次调用 `OplusCameraManagerGlobal.setClientInfo(packageName=com.oplus.camera, uid=10117, pid=6169)`；MTK tuning 日志同步使用该包名。
- 假设：第三方 Camera2 启动路径没有建立对应的 OPlus client info，HAL 的 `ThirdPkgName` 被错误归类，可能触发错误的相机场景/空帧输出。
- 操作：仅在本次实验里用 shell 设置 `oplus.camera.packname=com.linuxhub.husk`，再启动 exact 历史 APK 跑三帧；不修改任何系统持久设置，测完清空属性。
- 判据：属性设置后历史 APK 若恢复非黑，则必须用不受 shell 特权依赖的原生集成方案替代；若仍黑，排除该属性作为根因。
- 状态：继续 A/B。

### 16.298 shell 无权设置厂商相机属性（FAIL，2026-11-28）

- FAIL：`adb shell setprop oplus.camera.packname com.linuxhub.husk` 被系统拒绝：`Failed to set property ... See dmesg for error reason`；没有启动 APK，也没有把设置失败误当 A/B 结果。
- 说明：属性属于厂商受限域；需要 root 是否能临时设置先验证，且该实验只用于确认根因，不能成为 Husk 依赖 root 的产品方案。
- 下一步：检查 `adb root` 是否可用；可用则同样测历史 APK，完成后恢复空属性并停止 root；不可用则记为外部阻塞，转其它原生 Camera2 路径。
- 状态：属性实验阻塞；相机 FAIL。

### 16.299 adb root 不可用，属性实验终止（BLOCKED，2026-11-28）

- FAIL：`adb root` 返回 `adbd cannot run as root in production builds`；`adb shell id` 仍为 `uid=2000(shell)`。
- 判定：无法用受限厂商属性做 A/B；此路不能作为产品修复。属性假设保留为 UNKNOWN，不再继续尝试提权。
- 新证据：厂商同一张前置 JPEG EXIF 为 `ExposureTime=0.1s`、`ISO=9599`、`Flash=9`（闪光已触发），说明现场很暗且厂商相机很可能用屏幕/闪光补光；Husk NativeActivity 黑屏没有补光，曝光同样被拉到 60 ms/ISO≈16000 后仍近黑。
- 新假设：此前把“厂商前置正常”当成纯环境光是错误的；必须做有光场景对照，才能判断 Husk Camera2 YUV 是否真的失效。
- 下一步：先截取 Husk 前台画面、检查屏幕是否全黑；再用可控白光/手电或厂商屏幕补光路径做一次有光 A/B。
- 状态：属性路径 BLOCKED；黑帧根因重新打开。

### 16.300 Husk 前台屏幕几乎全黑，暗场假设成立（FACT，2026-11-28）

- FACT：`frames=0` 启动历史 APK 后截屏 `husk-foreground-screen-before-light.png`，灰度 `mean=11.86`，主体全黑，仅状态栏/手势条发亮。
- FACT：厂商前置 JPEG EXIF 为 0.1 s、ISO 9599、Flash=9；Husk 3A 为 60 ms、ISO 15996、`FLASH_REQUIRED`。两者都指向极暗场景，厂商相机很可能用屏幕补光，而 Husk 黑屏没有补光。
- 判定：此前“同分钟厂商正常、Husk 黑”不能直接证明 Camera2 软件故障；必须先加光。当前不应再改 to_nv12/reader/请求路径。
- 下一步：通过系统手电筒/快速设置或外部光做有光 A/B；若 Camera2 Y 恢复，则把 0.0.37 相机缺陷改判为测试现场暗场，不伪造修复。
- 状态：现场条件纠正为 FACT；相机软件路径 UNKNOWN。

### 16.301 用系统浅色主题制造受控屏幕补光（进行中，2026-11-28）

- 操作：临时把 APK manifest 从 `Theme.Material.NoActionBar` 改为 `Theme.Material.Light.NoActionBar`，只作为测试光源；构建当前恢复后的 camera.rs（不带 GPU/rare 实验），安装后前台跑三帧。测试后恢复 manifest 并重新构建正式包。
- 判据：若浅色窗口把 Y 从近零拉到明显非黑，则“暗场 + Husk 黑屏无补光”成立；若浅色窗口仍黑，暗场假设也不足以解释，继续查 HAL。
- 状态：修改前记录，随后构建安装实测。

### 16.302 Light theme 未改变渲染面，转 GL 白屏补光（FAIL，2026-11-28）

- FACT：manifest 使用 `Theme.Material.Light.NoActionBar` 后，前台 Husk 截屏仍 `mean=11.85`，三帧 Y 仍为 `0/5/0.2658`、`0/5/0.0454`、`0/4/0.0451`。浅色窗口被 GL SurfaceView 的深色 clear 覆盖，不能作为补光。
- 判定：这不是新增软件问题；测试需要让实际渲染面发白光。
- 操作：临时把 `app-rs/src/gl.rs` 的 `glClearColor(0.04,0.05,0.06,1.0)` 改为白色 `(1,1,1,1)`，构建当前相机源码并前台跑三帧；随后恢复 manifest 和 gl.rs，重装正式包。
- 判据：屏幕截屏应接近全白；若 Y 仍黑，暗场补光假设失败。
- 状态：Light theme FAIL；GL 白屏 A/B 待跑。

### 16.303 白屏但亮度仅 479/4095，需最大亮度复测（进行中，2026-11-28）

- FACT：GL 白屏截屏 `mean=242.03`，但系统 `screen_brightness=614`、自动亮度开，实际 `Display Brightness=479`，白屏只以低亮度发光；三帧仍黑。
- 修正：这次白屏不足以推翻暗场假设，因为物理发光强度仍低；厂商前置照片的 `Flash=9` 暗示其可能把屏幕拉到最大亮度作补光。
- 操作：记录原亮度 `614` 和自动亮度 `1`，临时设为 `4095`、自动亮度 `0`，保持同一 GL 白屏 APK 和同 camera id/size，重跑三帧；测完恢复原亮度与自动亮度。
- 判据：最大白屏下若 Y 明显恢复，则 Camera2 正常但现场需要补光；若仍黑，补光假设失败。
- 状态：继续 A/B。

### 16.304 shell 无权直接改亮度，转系统 QS 控件（FAIL，2026-11-28）

- FAIL：`settings put system screen_brightness_mode 0` 被 `SecurityException` 拒绝（shell 无 `WRITE_SETTINGS`）；没有执行后续三帧测试。
- 纠正：用已确认的 Quick Settings 亮度滑块和自动亮度开关，通过真实 SystemUI 控件调最大亮度，不改系统数据库。
- 操作：展开 QS，关闭自动亮度，将滑块拖到最大，确认实际 `Display Brightness` 接近 4095 后重跑；测完重新开启自动亮度。
- 状态：shell 路径 FAIL；QS 路径待跑。

### 16.305 QS 高亮白屏只把 Y 提到 1.6，需排除采集早于白屏（FAIL/待验证，2026-11-28）

- FACT：QS 实测 `screen_brightness=2047`、自动亮度 `0`、`Display Brightness=1597`；白屏截屏 `mean=242.03`。
- FACT：三帧 Y 上升为 `0/12/1.5806`、`0/9/1.6762`、`0/7/1.6965`（此前约 0.1），说明相机确实受到屏幕光影响，但三帧仍在启动后约 200 ms 内完成，不能证明白屏已经稳定呈现。
- FACT：测试后已恢复自动亮度模式 1（系统亮度值由系统重算为 1391）。
- 操作：给 selftest marker 增加仅诊断用 `delay_ms=3000`，等 GL 白屏真正稳定后再采集；仍用最大 QS 亮度，采集后回退 delay 代码和 gl 白屏代码。
- 判据：延迟后 Y 明显升高则 Camera2 正常，之前的黑帧是“暗场+采集早于补光”；仍黑则继续查 HAL。
- 状态：高亮白屏 FAIL/时序待验证。

### 16.306 白屏稳定 3 秒后 Y 仍只有 3.0（FAIL，需外部光，2026-11-28）

- FACT：QS 实际 `screen_brightness=2047`、`Display Brightness=2047`、白屏均值 `242.02`；selftest 明确等待 `delay_ms=3000` 后才打开相机。
- FACT：三帧 Y 升至 `0/17/2.9072`、`0/14/3.0161`、`0/12/3.0219`（先前约 0.1），证明 sensor/HAL 对屏幕光有响应，但仍不是可用画面；U/V 基本 128。
- 判定：相机不是完全死路，长曝光+白屏能让 Y 上升，但仍远低于正常场景。必须用真正的外部强光做最终判据；不能把 3.0 当相机 PASS。
- 操作：恢复 `camera.rs`（去掉 delay）、`gl.rs`（恢复深色 clear）、manifest（恢复深色主题），重建并重装当前正式源码包，避免测试包留在设备。
- 下一步：外部光下复测 Camera2；同时推进 PipeWire/WirePlumber 桌面入口，因为媒体服务是否拉起与相机像素黑是两条独立链路。
- 状态：暗场/补光实验 FAIL；相机门禁继续 FAIL。

### 16.307 用系统 QS 手电筒做外部强光 30 帧 A/B（进行中，2026-11-28）

- 补记：LinuxHub `11.270` 已补写，对应本条之前的 Husk `16.306`；两处记录已重新对齐。
- 现场：设备 `UKHMMB8PYTGECMNV`，当前前台 `com.linuxhub.husk/android.app.NativeActivity`，`oplus.camera.torch=0`，自动亮度模式 `1`。
- 操作：展开系统 Quick Settings，确认“手电筒”tile 后真实点击；确认 `oplus.camera.torch=1`，再冷启动 Husk selftest 写 `frames=30 camera=1 delay_ms=3000`，收集 30 帧、Y 统计、截图与 logcat；完成后关闭手电筒并确认 `oplus.camera.torch=0`。
- 判据：手电筒强光下若 30 帧 Y 明显正常，则现有 Camera2→YUV 链路可用，暗场是此前黑帧主因；若仍近黑，才继续查 MTK HAL/私有 stream/CameraExtension。无论结果，都不能用 probe 或单帧冒充 Snapshot PASS。
- 状态：执行前记录，随后跑 A/B。

### 16.308 手电筒状态下前摄 30 帧实际 Y 仍黑；转后摄直接光照 A/B（FAIL/继续，2026-11-28）

- FACT：QS 手电筒 tile 文本从“关闭”变“开启”、`checked=true`，但厂商属性 `oplus.camera.torch` 仍为 `0`；记录“UI 已开、属性不跟随”，不把属性误当唯一状态。
- FACT：Husk 前台冷启动 `303 ms`，selftest `frames=30 camera=1 delay_ms=3000` 完成 `30` 帧、`29.92 fps`、`elapsed_ms=1151`，输出均为 `1280x720` NV12。
- 纠正统计口径：日志里的 `mean≈42.78` 是整块 NV12（UV 固定 128）的均值，不是 Y。逐帧拆 Y 平面后，`Y mean=0.1669..0.3037`、`Y max` 中位数 `3`、最大仅 `6`；实际画面仍全黑。证据在 `torch-30f-16.307/`，含 30 帧、`y-stats.tsv`、`y-contact-sheet.png`、logcat 和前后截图。
- 判定：前摄没有被后方手电筒有效照亮，因此这轮不能验证前摄正常；但整条 selftest 请求/回调/转换仍在 30 fps 稳定产帧。下一步改测后摄 `camera=0`，让手电筒直接照到后摄视场；这才是同设备、同光源的有效直照 A/B。
- 状态：前摄手电筒 A/B FAIL（照明未覆盖）；后摄 A/B 待跑。

### 16.309 QS 手电筒在相机打开时被系统强制关闭；改为会话内 FLASH_MODE_TORCH 诊断（FACT/进行中，2026-11-28）

- FACT：后摄 30 帧同样 `29.92 fps`、`elapsed_ms=1204`，但逐帧 Y 仅 `0.2896..0.6155`、`Y max` 中位数 `2`；画面仍黑。证据在 `torch-30f-back-16.308/`。
- 根因证据：该轮 logcat 明确记录 `Torch for camera id 0 turned on`，随后 Husk 打开 camera 0 时出现 `Torch status changed ... newStatus=0`、`setTorchOnOff enable:0` 和 `setTorchMode ... CAMERA_IN_USE`。系统 QS 手电筒会在相机占用时被关闭，因此前/后两轮都不是有效的外部光 A/B，不能据此判断 HAL 输出。
- 纠正：不再用“先开 QS 手电筒、再开相机”的顺序测。给 selftest marker 增加仅诊断用的 `torch=1`，在已建立的 Camera2 capture request 内设置标准 `ANDROID_FLASH_MODE=TORCH`（同时保持 `CONTROL_AE_MODE=ON`），默认生产路径保持关闭。
- 操作：实现 `torch=1` 会话内诊断开关，构建/安装 Husk 自有 APK，先跑后摄 `frames=30 camera=0 delay_ms=3000 torch=1`，再按结果决定前摄；测后回退临时白屏/delay/主题并重装正式源码包。
- 状态：QS 手电筒 A/B 判定 INVALID；会话内 torch 诊断修改前记录。

### 16.310 selftest 会话内 torch 诊断已实现，Android 类型检查 PASS（2026-11-28）

- 改动：`app-rs/src/camera.rs` 增加 `TAG_FLASH_MODE=(4<<16)+2`、`FLASH_MODE_TORCH=2`；`camera-selftest` 支持 `torch=1`，只在 selftest 的 active Camera2 request 内设置 `FLASH_MODE`。默认 camera bridge 调用显式传 `false`，生产路径不启用 torch。
- 验证：`cargo fmt --manifest-path app-rs/Cargo.toml` PASS；`cargo check --manifest-path app-rs/Cargo.toml --target aarch64-linux-android --lib --bin husk-session-guard` PASS（19 条既有 warning，无 error）。
- 操作：构建 release `libhuskapp.so` + Gradle APK，安装后写 `frames=30 camera=0 delay_ms=3000 torch=1`，跑后摄 30 帧，拆 Y 统计并抓 logcat 的 `flash_mode_rc`/`torch`。
- 状态：类型检查 PASS；APK 构建与设备 A/B 待跑。

### 16.311 会话内 torch 诊断 APK 构建 PASS（2026-11-28）

- FACT：`make apk` PASS；`libhuskapp.so`、`libhuskguard.so` 和 Gradle `:app:assembleDebug` 均完成，APK 生成于 `app/app/build/outputs/apk/debug/app-debug.apk`。
- SHA256：`66e6219f5ef1454e948292c080ee683bf0cb5bbc8c5a5f2ba13fcd0ef95bb720` (`libhuskapp.so`)；`fe1842bd9ec49185b4cd014ed21903e74387bb270d79f853c46ef19861ded0a0` (APK)。
- 操作：安装此 APK；先 force-stop，备份旧帧目录，写 `frames=30 camera=0 delay_ms=3000 torch=1`，冷启动并收集 30 帧/logcat；通过 `camera|session|request_entries|...flash_mode_rc...|torch=true` 证明请求生效，再拆 Y 判图。
- 状态：构建 PASS；设备 torch 请求 A/B 待跑。

### 16.312 会话内 torch 后摄 30 帧真实非黑，Camera2→NV12 主链路验证 PASS（2026-11-28）

- PASS：安装新 APK 后 Husk 前台冷启动 `274 ms`；`frames=30 camera=0 delay_ms=3000 torch=1` 完成 `30` 帧、`28.92 fps`、`elapsed_ms=1200`。
- PASS：日志明确 `camera|session|request_entries|control_mode_rc=0|ae_mode_rc=0|flash_mode_rc=0|torch=true`，说明会话内标准 `FLASH_MODE_TORCH` 请求已被接受；轮次结束日志恢复 `Torch ... AVAILABLE_OFF`、`setFlashOff`，资源正常释放。
- FACT：逐帧 Y `mean=49.2584..55.3924`、中位数 `52.4266`，`Y max=119..133`，`Y std` 中位数 `25.8723`，U/V 也随画面变化；`y-contact-sheet.png` 可见非黑亮度梯度。证据在 `torch-request-30f-back-16.311/`。
- 判定：Camera2 session/output/request/ImageReader/`to_nv12()` 在真实光照下可输出非黑 30 帧，主链路 PASS。此前零光黑帧与 `mean≈42.8` 的整块 NV12 误判已分清；前摄是否只是缺光仍待外部光验证，不能直接判前摄 PASS。
- 操作：保留 selftest-only `torch=1` 诊断钩子（默认生产 false），恢复临时 Light 主题、GL 白屏和 `delay_ms` 诊断改动；重建并重装恢复后的正式源码包，核对 camera.rs SHA 与设备顶层。
- 状态：后摄 Camera2→NV12 PASS；临时实验回退进行中。

### 16.313 临时主题/白屏/延迟改动已恢复，torch 诊断保留（PASS，2026-11-28）

- PASS：`AndroidManifest.xml` 已从 `Theme.Material.Light.NoActionBar` 恢复为 `Theme.Material.NoActionBar`，与 `/tmp/AndroidManifest.xml.pre-light-16.301` 逐字一致。
- PASS：`app-rs/src/gl.rs` 已恢复深色 `glClearColor(0.04, 0.05, 0.06, 1.0)`，与 `/tmp/gl.rs.pre-white-16.302` 逐字一致。
- PASS：`camera.rs` 已移除 `delay_ms` 解析和 3 秒 sleep，仅保留 selftest-only `torch=` 诊断；`cargo fmt --check` PASS。
- 操作：再次执行 Android 类型检查与 `make apk`，生成正式源码包；安装前移除设备 selftest marker，清空测试帧目录，重装后核对 APK/lib 版本、SHA256 与 NativeActivity 前台。
- 状态：临时实验代码恢复 PASS；正式包重建待跑。

### 16.314 恢复后的正式源码包重建 PASS（2026-11-28）

- PASS：Android arm64 `cargo check` PASS；`make apk` PASS，Gradle `assembleDebug` PASS。
- SHA256：`libhuskapp.so=928d1677fce81d9b5cb29d639fc0ee087ea927345dec34d76e7d4364b8b583a9`；`app-debug.apk=ccfe125d454974a01e93c7d6f49b3c5b7cb32db7387bb45fcdf16627347d4da9`；`libld.so=6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506`。
- 操作：安装该 APK；删除设备 `files/camera-selftest` marker，保留不再触发 selftest；冷启动后确认顶层为 `com.linuxhub.husk/android.app.NativeActivity`，并从 APK/设备文件核对 libhuskapp SHA 与版本。
- 状态：正式包构建 PASS；设备安装与身份核对待跑。

### 16.315 恢复后的正式源码包安装通过，运行身份可自证（PASS，2026-11-28）

- PASS：`adb install -r` Success；`versionCode=37`、`versionName=0.0.37`；冷启动 `276 ms`，顶层仍为 `com.linuxhub.husk/android.app.NativeActivity`。
- PASS：设备无 `files/camera-selftest` marker，本次启动无 selftest 日志；`restored-screen.png` 灰度均值 `11.8446`，确认 GL 白屏实验已回退。
- PASS：拉回设备 APK SHA256 `ccfe125d454974a01e93c7d6f49b3c5b7cb32db7387bb45fcdf16627347d4da9`，与构建产物一致；运行中 `/proc/<pid>/maps` 明确映射 `/data/app/.../lib/arm64/libhuskapp.so`，该已安装私有库 SHA256 为 `32ce41518218c5e70342ed5835e476fb30ea802e46cfc6432a507c20371fbeb3`（AGP strip 后版本，不能与源码构建 SHA 混写）。
- 证据：`formal-restored-16.314/`，含 install/launch/top/version、设备实际 APK、runtime lib hash、maps 和回退后截图。
- 状态：正式源码包恢复 PASS；相机 selftest 临时白屏/主题/延迟残留已清；下一步对接媒体服务。

### 16.316 审计 Husk 唯一桌面入口与 PipeWire/WirePlumber 配置集成（进行中，2026-11-28）

- 操作：只读检查 Husk `tools/guest/husk-desktop.sh`、`tools/guest/husk-media-session.sh`、`tools/guest-config/wireplumber.conf` 和 LinuxHub 对应 assets，确认启动顺序、去重、退出收尾是否已经进入唯一入口。
- 判据：桌面入口必须在 camera.sock ready 后启动媒体服务；重复启动不得产生第二套 PipeWire/WirePlumber；退出必须回收；不能靠手工临时脚本冒充。
- 状态：审计前记录。

### 16.317 媒体入口审计发现两套资产漂移：LinuxHub 的 PipeWire 漏了 V4L2 preload（FAIL，2026-11-28）

- PASS：Husk `husk-desktop.sh` 已在会话总线就绪后、GNOME Shell 之前调用 `/usr/local/bin/husk-media-session start`，顺序符合 `camera.sock ready -> PipeWire/WirePlumber -> Shell`；退出路径也调用同一个 helper `stop`。
- PASS：`husk-media-session.sh` 有 pid 文件、`kill -0`/`pw-cli`/`wpctl` 探测、重复启动 reuse、失败清理和陈旧 socket 清理，已具备幂等入口骨架。
- FAIL：Husk canonical `tools/guest/husk-media-session.sh` 给 PipeWire 和 WirePlumber 都设置 `LD_PRELOAD=$V4L2_PRELOAD`；LinuxHub asset `app/src/main/assets/desktop/gnome/husk-media-session.sh` 只给 WirePlumber 设置，PipeWire 启动行漏掉 preload。两份脚本不字节一致，正是“两套实现没有完全融合”的实证。
- PASS：`pipewire.conf` 与 `wireplumber.conf` 在两仓库当前副本逐字一致；`husk-desktop.sh`、`install-husk.sh` 和 `make dist` 都已包含 helper/config。
- 操作：以 Husk canonical helper 为准同步 LinuxHub asset；增加 LinuxHub 单测，断言 helper 对 PipeWire 和 WirePlumber 都注入 V4L2 preload，防止再次漂移。随后运行 LinuxHub 单测并构建 `com.linuxhub.myth.husk` debug APK。
- 状态：媒体入口未融合完成，FAIL；同步修复前记录。

### 16.318 LinuxHub 媒体 helper 已同步，首个单测编译失败（FAIL，2026-11-28）

- PASS：LinuxHub asset `husk-media-session.sh` 已由 Husk canonical 覆盖，`cmp` 字节一致；PipeWire 与 WirePlumber 现都设置 `LD_PRELOAD=$V4L2_PRELOAD`。
- FAIL：首次直接执行 `./gradlew` 被拒（文件无执行位）；改用 `sh gradlew` 正常进入构建。
- FAIL：新增 `HuskMediaSessionAssetTest.kt` 编译失败：Kotlin 字符串中的 `$V4L2_PRELOAD` 被当成变量，且把 `File` 直接传给字符串断言；尚未运行测试，不记为测试失败。
- 纠正：测试改为先 `readText()`，用普通字符串转义 `$`，再按行统计双 preload；修正后重跑单测。
- 状态：同步 PASS；测试代码修正中。

### 16.319 LinuxHub 媒体 helper 同步与防漂移单测 PASS（2026-11-28）

- PASS：LinuxHub `app/src/main/assets/desktop/gnome/husk-media-session.sh` 已与 Husk canonical 字节一致；PipeWire/WirePlumber 双 `LD_PRELOAD=$V4L2_PRELOAD`。
- PASS：新增 `HuskMediaSessionAssetTest`，按行断言两个 preload，并用 `/bin/sh -n` 校验脚本语法；`sh gradlew :app:testDebugUnitTest --tests ...HuskMediaSessionAssetTest` BUILD SUCCESSFUL。
- FAIL（新发现）：LinuxHub `HuskBridgeController` 运行 `assets/husk/arm64-v8a/husk-bridges`，但该 asset 在仓库中不存在；`libld.so` 与 `libhusk_v4l2.so` 的 SHA 也落后于 Husk 当前构建。媒体栈即使启动，也没有 Camera2/V4L2 桥可供它消费。
- 操作：用 Husk 当前源码重建 Android `husk-bridges`，再同步 `husk-bridges`、`libhusk_v4l2.so`、`libld.so` 到 LinuxHub assets；逐个记录 SHA256，并加资产存在性/身份校验。
- 状态：媒体 helper PASS；硬件桥资产缺失为 BLOCKER，开始同步。

### 16.320 硬件桥、V4L2 preload、loader 当前产物重建 PASS（2026-11-28）

- PASS：`make preloads` PASS，重建 `libhusk_v4l2.so`；`cargo build --release --target aarch64-linux-android --bin husk-bridges` PASS。
- SHA256：`libhusk_v4l2.so=2cde00f8fd81216299edebe4e7435124d8f64f3b90f86ac4031aff26586c9d23`；`husk-bridges=7e41ab7e1e3f428071ae5784bac160afb58b18e8ae0532b0c0133bad18c36bcb`；`libld.so=6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506`。
- 操作：将这三个当前产物同步到 LinuxHub `app/src/main/assets/husk/arm64-v8a/`，设置 bridge/preload 可执行位；用 `cmp` 和逐项 SHA 校验，不保留旧副本冒充当前 runtime。
- 状态：产物构建 PASS；同步前记录。

### 16.321 LinuxHub 硬件桥三件套已同步到当前 SHA（PASS，2026-11-28）

- PASS：LinuxHub `app/src/main/assets/husk/arm64-v8a/` 现包含 `husk-bridges`、`libhusk_v4l2.so`、`libld.so`；三者与 Husk 当前构建逐字一致（`cmp` PASS）。
- SHA256：`husk-bridges=7e41ab7e1e3f428071ae5784bac160afb58b18e8ae0532b0c0133bad18c36bcb`；`libhusk_v4l2.so=2cde00f8fd81216299edebe4e7435124d8f64f3b90f86ac4031aff26586c9d23`；`libld.so=6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506`。
- PASS：`file` 确认 bridge 为 aarch64 PIE executable，两个 `.so` 为 aarch64 shared object；bridge/preload 可执行位置位。
- 操作：新增 LinuxHub 资产单测，断言三件套存在、ELF magic 正确、体积不为占位；随后跑完整相关单测和 `:app:assembleDebug`。
- 状态：同步 PASS；防回归测试与 APK 构建待跑。

### 16.322 LinuxHub 硬件资产防回归单测 PASS（2026-11-28）

- PASS：新增 `HuskHardwareAssetsTest`，断言 `husk-bridges`、`libhusk_v4l2.so`、`libld.so` 均存在、体积大于占位阈值、前四字节为 ELF。
- PASS：与 `HuskMediaSessionAssetTest` 同跑，`sh gradlew :app:testDebugUnitTest --tests ...HuskMediaSessionAssetTest --tests ...HuskHardwareAssetsTest` BUILD SUCCESSFUL。
- 操作：构建 LinuxHub `:app:assembleDebug`，产出 `com.linuxhub.myth.husk` debug APK；记录 APK SHA、大小、版本和 assets 内三件套 SHA。
- 状态：单测 PASS；完整 APK 构建待跑。

### 16.323 LinuxHub APK 构建 PASS，但 debug 资产覆盖旧 bridge（FAIL，2026-11-28）

- PASS：`sh gradlew :app:assembleDebug` BUILD SUCCESSFUL；APK `app/build/outputs/apk/debug/app-debug.apk` 大小 `1,049,344,044` 字节，SHA256 `4be9a661a562947bf9ca503f76beeee867e96e02c351348d3f33e0c1f8841a7d`。
- PASS：APK 内 `libhusk_v4l2.so`/`libld.so` SHA 与 canonical 一致，媒体 helper/config 均已打包。
- FAIL：APK 内 `assets/husk/arm64-v8a/husk-bridges` SHA 为 `c0663333...`、大小 `652904`，不是 main asset 的 `7e41ab7e...`/`662472`。根因是 `app/src/debug/assets/husk/arm64-v8a/husk-bridges` 旧副本覆盖 main asset；再次证明双份资产会静默装旧桥。
- 操作：把 debug 覆盖副本移到 `/tmp` 备份，main assets 保留唯一当前 bridge；在 `HuskHardwareAssetsTest` 增加“debug 不得覆盖 bridge”断言，重建 APK 并重新核对 APK 内 SHA。
- 状态：APK 构建 PASS；runtime identity FAIL；修复前记录。

### 16.324 去除 debug 覆盖后 APK runtime identity 全部对齐（PASS，2026-11-28）

- PASS：debug 旧 `husk-bridges` 已移到 `/tmp/husk-bridges.pre-11.279-debug`；新增单测禁止 debug source set 再出现第二份 bridge。
- PASS：资产单测与媒体 helper 单测重跑 BUILD SUCCESSFUL；`:app:assembleDebug` BUILD SUCCESSFUL。
- PASS：新 APK SHA256 `3b2ca3aa841baaa2b5c080e6da8a8d010849633f5555ec76af98a1e7bbee0476`，大小 `1,049,344,044` 字节；APK 内 `husk-bridges=7e41ab7e...`、`libhusk_v4l2.so=2cde00f8...`、`libld.so=6ef6e5bf...`，均与当前构建一致。
- 操作：`adb install -r -g` 安装 `com.linuxhub.myth.husk`，启动 `com.java.myapplication.MainActivity`；确认版本、权限、Native/首页前台，进入 Husk 桌面后检查 `husk-bridges`、`camera.sock`、PipeWire/WirePlumber。
- 状态：APK identity PASS；设备安装与运行验证待跑。

### 16.325 LinuxHub `.husk` APK 安装、权限与硬件桥启动 PASS（2026-11-28）

- PASS：`adb install -r -g` Success；版本 `versionCode=201`、`versionName=Myth-Prometheus`；冷启动 `858 ms`，顶层 `com.linuxhub.myth.husk/com.java.myapplication.MainActivity`。
- PASS：CAMERA、RECORD_AUDIO、ACCESS_FINE_LOCATION、ACCESS_COARSE_LOCATION 均 granted；首页显示 `Runtime 就绪`、`Ubuntu 24.04`、`Husk`、`4 个硬件端点`。
- PASS：`ps -A` 显示 `husk-bridges` 进程；logcat 记录 `HuskBridge: bridge helper started`，并以 `.husk` UID 执行 app 私有 `husk-bridges`；非仅声明/仅打包。
- 证据：`linuxhub-husk-install-16.324/`，含 launch/top/package/startup-logcat/UI XML/启动截图。
- 操作：点击首页“主入口/进入桌面”进入 Husk GNOME 会话；等待并检查桌面 readiness、`camera.sock`、PipeWire/WirePlumber 进程与启动阶段日志。
- 状态：安装与桥启动 PASS；桌面/媒体实测待跑。

### 16.326 LinuxHub 从唯一入口进入 GNOME 桌面（PASS，2026-11-28）

- PASS：通过 LinuxHub `.husk` 首页“主入口/进入桌面”进入 GNOME；桌面截图已落盘 `docs/design/evidence/367-camera-audit-20261128/linuxhub-desktop-16.326/desktop-screen.png`。
- PASS：会话内启动记录出现 `stage=media-ready pipewire=15677 wireplumber=15690`，证明媒体 helper 已由桌面唯一入口拉起，时序在 Shell 会话内完成。
- FACT：`dumpsys top` 中 MainActivity 仍为顶层不等于未进入桌面；GNOME X11 surface 运行在 LinuxHub 应用进程内。
- 操作：保持当前桌面现场，检查 `camera.sock`、`/dev/video0/1`、PipeWire/WirePlumber 以及 `wpctl` 设备枚举；随后启动 GNOME Snapshot 做真实消费端验证。
- 状态：桌面进入 PASS；Snapshot 相机最终门禁仍 FAIL/待验证。

### 16.327 PipeWire/WirePlumber 已枚举后摄 V4L2，仅 BACK 可见（PASS/部分，2026-11-28）

- PASS：`wpctl status` 输出 `Video -> Devices: 37. BACK [v4l2]` 与 `Sources: 38. BACK (V4L2)`。
- PASS：`pw-cli ls Device` 显示 `device.api = "v4l2"`、`device.description = "BACK"`、`device.name = "v4l2_device._sys_class_video4linux_video0"`、`media.class = "Video/Device"`。
- 证据：`docs/design/evidence/367-camera-audit-20261128/linuxhub-media-16.327/wpctl-status.txt`、`pw-cli-info.txt`、`pw-cli-devices.txt`。
- 限制：当前只看到 BACK，未看到 FRONT；不能声称前后摄均可被 Snapshot 使用。
- 操作：启动 GNOME Snapshot，检查是否仍显示 “No Camera Found”；若枚举成功则实际拍摄并抓 `pw-top`/node 状态，证明取流 RATE 非零；同时检查 `/dev/video1` 和 FRONT 未进 WirePlumber 的原因。
- 状态：媒体服务与后摄 V4L2 枚举 PASS；Snapshot 真实出图 NOT VERIFIED，最终相机门禁 FAIL。

### 16.328 检查当前 GNOME 会话、Snapshot 入口与前后摄节点（进行中，2026-11-28）

- 操作：不改代码，先只读核对设备在线状态、LinuxHub/Husk 进程、`rootfs/run/husk/camera.sock`、`/dev/video0`/`/dev/video1`、udev 属性和 `/usr/share/applications/` 中 Snapshot desktop 文件名。
- 判据：会话仍存活；Snapshot 有可执行 GUI 入口；BACK/FRONT 两个节点至少能被 V4L2 层识别，才能解释 WirePlumber 只发现 BACK 的差异。
- 状态：执行前记录。

#### 16.328 检查结果（FAIL/待归因，2026-11-28）

- PASS：设备 `UKHMMB8PYTGECMNV` 在线；LinuxHub app `13740`、`husk-bridges` `15413`、GNOME Shell `15569` 仍在运行；顶层为 LinuxHub MainActivity（X11 surface 承载桌面）。
- PASS：`rootfs/run/husk/camera.sock` 存在，类型为 socket，权限 `srwx------`。
- FAIL：rootfs 内 `dev/video0`、`dev/video1` 当前实际不存在；`ls` 报 `No such file or directory`，`udevadm info --name=/dev/video0/1` 报 `Unknown device`。
- PASS：同一会话 `wpctl status` 仍枚举到 Video Device `37. BACK [v4l2]` 和 Source `38. BACK (V4L2)`。
- PASS：rootfs 内有 `/usr/share/applications/org.gnome.Snapshot.desktop` 与 `org.gnome.Cheese.desktop`；未找到 PATH 中的 `gnome-snapshot`/`snapshot`/`cheese` 短名可执行文件。
- INFERENCE：PipeWire 的 BACK 来自 V4L2 preload/shim 的枚举路径，不是 rootfs 里静态存在的 `/dev/video0` 节点；“节点不存在”不能直接否定媒体设备枚举，两者需继续按源码链路核清。
- 操作：使用 desktop 文件经 `gio launch` 启动 Snapshot，抓进程、日志和屏幕截图；同时核对 FRONT 是否因 `/dev/video1` 缺失而未被 WirePlumber 发布。
- 状态：会话存在；V4L2 静态节点 FAIL；PipeWire 后摄枚举 PASS；Snapshot 待测。

### 16.329 启动 GNOME Snapshot 并验证真实消费端（进行中，2026-11-28）

- 操作：在 Husk 会话环境执行 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop`，等待窗口出现；截图判断是否仍为 “No Camera Found”。
- 判据：Snapshot 能枚举 BACK；拍照/预览时 PipeWire node 出现非零 RATE；日志无 Camera2 timeout；画面肉眼可见。
- 状态：执行前记录。

#### 16.329 启动结果（FAIL，2026-11-28）

- 操作：通过 Husk 环境执行 `gio launch /usr/share/applications/org.gnome.Snapshot.desktop` 成功进入应用启动函数，日志出现 `Snapshot (org.gnome.Snapshot) Version: 46.2` 与 `Datadir: /usr/share/snapshot`。
- FAIL：等待 5 秒后屏幕仍为 GNOME 桌面，没有 Snapshot 窗口；截图 `/tmp/husk-snapshot-launch.png` 为 2400x1080，仅显示桌面。
- WARNING：启动日志有 `libpxbackend-1.0.so: cannot open shared object file`、`Failed to load module ... libgiolibproxy.so`、`MESA-EGL DRI3 error`、`KBASE_IOCTL_MEM_EXEC_INIT failed` 和 PanVK non-conformant 警告。
- FACT：进程扫描命令误匹配到了 pulseaudio，不能据此声称 Snapshot 保持运行；必须按完整 cmdline/窗口树重新核对。
- 操作：抓完整 guest 进程、logcat 尾部、X11 窗口树和 D-Bus application 状态，判断 Snapshot 是退出、崩溃还是窗口未映射。
- 状态：启动入口触发 PASS；窗口可用 FAIL；相机未进入验证。

### 16.330 定位 Snapshot 窗口未出现原因（进行中，2026-11-28）

- 操作：检查含 `org.gnome.Snapshot` 的 cmdline、应用进程退出码、logcat、`wmctrl -l`/`xwininfo -root -tree` 与 session log。
- 判据：找到明确失败点后做最小修复；不得用重试碰运气或截图桌面冒充 Snapshot。
- 状态：执行前记录。

#### 16.330 检查结果（FAIL，2026-11-28）

- FAIL：完整 cmdline 扫描无 `org.gnome.Snapshot` 进程，Snapshot 已退出。
- FAIL：`xwininfo -root -tree` 只有 Desktop Icons、GNOME Shell、portal、gsd 等窗口，没有 Snapshot 窗口；不是窗口跑到别的工作区。
- FACT：`wmctrl` 未安装，不能用该工具；`xwininfo` 可用并作为本次窗口树证据。
- FACT：Android logcat 尾部没有 Snapshot 的有效崩溃记录，只有系统 DRS 日志含 snapshot 字样；Snapshot stderr 在 `gio launch` 时直接输出，需前台直跑才能拿完整退出码。
- 操作：读取 `org.gnome.Snapshot.desktop` 的 `Exec`，直接前台启动真实二进制，重定向 stdout/stderr 到文件并记录退出码。
- 状态：确认 Snapshot 退出；根因未定位。

### 16.331 前台直跑 Snapshot 获取退出原因（进行中，2026-11-28）

- 操作：读取 desktop 文件，解析 Exec；在同一 Husk 环境前台运行，保存退出码和完整 stderr。
- 判据：必须拿到具体失败点；如果只是缺动态库，先分清是启动依赖还是相机/GStreamer 链路。
- 状态：执行前记录。

#### 16.331 前台直跑结果（FAIL，2026-11-28）

- PASS：`org.gnome.Snapshot.desktop` 的 `Exec=/usr/local/bin/doro-gpu-run snapshot`；真实二进制为 `/usr/bin/snapshot`。
- FAIL：前台执行 `/usr/local/bin/doro-gpu-run snapshot` 后进程以 **EXIT=139（SIGSEGV）** 退出，未映射 Snapshot 窗口。
- FACT：退出前仍输出 Snapshot 46.2 启动信息、DRI3 不可用、kbase exec-init EPERM、PanVK 非一致性、`libpxbackend-1.0.so` 缺失与 `libgiolibproxy.so` 加载失败。
- FACT：退出时 PipeWire 只有 BACK device/source，`pw-top` 中 v4l2 source `RATE=0`，证明还没有真实取流。
- 判定：当前阻塞首先是 Snapshot 的 139 崩溃，不是 “No Camera Found” 或 Camera2 黑帧；必须先把崩溃定位到原生栈。
- 操作：检索既有 139 证据/补丁、tombstone/debuggerd/logcat，并对 PanVK 与 Cairo/software 渲染做单变量 A/B。
- 状态：Snapshot 启动 FAIL(139)；相机 NOT VERIFIED。

### 16.332 定位 Snapshot SIGSEGV 与既有 139 修复关系（进行中，2026-11-28）

- 操作：只读检查 evidence 364/365、defect-ledger、当前 139 相关补丁与 Android tombstone/logcat；随后用环境变量单变量复跑。
- 判据：拿到崩溃模块/栈或稳定 A/B 差异；不得把 139 当相机问题，也不得以打补丁代替原生修复。
- 状态：执行前记录。

#### 16.332 检查结果（FAIL/根因仍未命中，2026-11-28）

- PASS：loader 的崩溃处理器生效，`rootfs/tmp/nr-sigsys.log` 中有两条 `segv|sig=11`，生成 `nr-crash-16516.maps` 与 `nr-crash-16802.maps`，时间均为 06:25。
- FACT：两次崩溃寄存器几乎完全同形态：第一次 `fa=0x765e68b030 pc=0x76a36f1a18 lr=0x76a4d24ddc x1=fa x2=3840`；第二次仅 ASLR 基址变化，`fa=0x7b362eb030 pc=0x7b7bc01a18 lr=0x7b7d234ddc x1=fa x2=3840`。
- FACT：两份 `nr-crash-*.maps` 都是约 4 KB，只留下 heap 与大段 `/dev/mali0` 映射，没有库映射，当前无法把 pc 精确归到某个 .so；这是诊断信息不完整，不是“没有崩溃”。
- FACT：Crash 前 Android Camera HAL 已启动（06:25:52 有 mtkcam session），说明 Snapshot 启动阶段确实触发了相机初始化和 139，但尚未证明 139 根因在 Camera2。
- 判定：139 稳定发生在 Snapshot 启动/首帧前后，寄存器显示长度为 3840 的内存操作；需抓完整 live maps 并做 renderer A/B。
- 状态：139 根因未定；需要完整映射与单变量对照。

### 16.333 抓 Snapshot 139 完整映射并做 PanVK/software 单变量 A/B（进行中，2026-11-28）

- 操作：同一启动方式先跑 PanVK，外部循环持续复制 `/proc/<pid>/maps`；记录 exit code 和 stderr。随后仅改 `DORO_GPU_RENDERER=software` 复跑。
- 判据：PanVK 稳定 139、software 不崩则锁定渲染路径；两者都崩则优先检查相机/GStreamer；完整 maps 必须能把 pc 落到库与偏移。
- 状态：执行前记录。

#### 16.333 PanVK 复跑与完整 maps 结果（FAIL/已定位到拷贝路径，2026-11-28）

- PASS（诊断）：PanVK 复跑稳定复现 `exit=139`，live maps 1010 行；第三次 `segv` 为 `pc=0x7eb8951a18 lr=0x7eb9f84ddc x0=0x7e50044ee8 x1=fa=0x7e6b8bb030 x2=3840`。
- PASS（定位）：`pc` 命中 `libc.so.6` 文件偏移 `0xa1a18`；`lr` 命中 `libgstvideo-1.0.so.0.2402.0` 文件偏移 `0x84ddc`。
- FACT：`x1/fa=0x7e6b8bb030` 在该进程 maps 中没有任何映射；`x0` 位于可写匿名映射，`x2=3840`。形态是 libc 拷贝函数从已失效/未映射的 GStreamer 视频 buffer 读取。
- FACT：`nr-crash-17144.maps` 仍只有 48 行且缺库映射；完整的 live maps `snapshot-panvk.maps` 才具备定位能力。
- 判定：139 已从“未知 Snapshot 崩溃”收窄到 `libgstvideo` 调用 libc 拷贝未映射视频 buffer 的原生路径；下一步符号化 `libgstvideo+0x84ddc` 并查这块 buffer 的生命周期/映射来源。
- 状态：139 根因路径已定位；不是已修复。

### 16.334 符号化 libgstvideo 偏移并查 buffer 生命周期（进行中，2026-11-28）

- 操作：拉取设备实际 `libgstvideo` 与 `libc`，用 NDK llvm-symbolizer/addr2line 解析 `0x84ddc` 与 `0xa1a18`；对照 GStreamer 源码定位具体帧拷贝函数。
- 判据：找到调用者函数、buffer 类型与映射/解除映射时机；修复必须放在 Husk V4L2/camera bridge 或通用 GStreamer/V4L2 生命周期，不放“忽略 SIGSEGV”补丁。
- 状态：执行前记录。

#### 16.334 符号化结果（FACT，2026-11-28）

- PASS：NDK `llvm-addr2line` 将 `libgstvideo+0x84ddc` 解析为 `gst_video_frame_copy_plane`；相邻偏移同属该函数。
- FACT：`libc+0xa1a18` 无本地符号，但寄存器形态与 `memcpy/memmove` 一致；`x0` 是已映射目标、`x1` 是未映射源、`x2=3840`。
- 判定：根因在 GstVideo 帧拷贝的源 buffer 生命周期，不在 Camera2 输出黑帧、不是 PanVK 指令崩、也不是单纯窗口系统。
- 操作：用 guest `strace` 跟踪 Snapshot 的 `mmap/munmap/ioctl/openat/close`，定位该源地址的映射来源和提前 unmap 的调用者。
- 状态：函数级根因已确定；具体生命周期缺陷待抓 syscall。

### 16.335 strace 跟踪 Snapshot 的 mmap/munmap 生命周期（进行中，2026-11-28）

- 操作：`strace -f -tt -yy -e trace=mmap,munmap,ioctl,openat,close,memfd_create,ftruncate` 前台跑 Snapshot，日志落 rootfs `/tmp/snapshot.strace`。
- 判据：找到崩溃源地址对应的 mmap 文件与后续 munmap；若 unmap 在引用仍存活时发生，修对应生命周期。
- 状态：执行前记录。

#### 16.335 strace 结果（FACT/根因收窄，2026-11-28）

- PASS：strace 完整记录 44,108 行，Snapshot 仍以 139 退出；崩溃地址 `si_addr=0x704903b030`。
- FACT：同一地址 `0x704903b000` 先由 PID 17570 执行 `mmap(NULL, 4149248, PROT_READ|PROT_WRITE, MAP_SHARED, fd=41, offset=0)`，fd 的类型是 `memfd:pipewire-memfd...size=33177984`。
- FACT：约 0.39 秒后，**另一线程 17579** 执行 `munmap(0x704903b000, 4149248)=0`；再约 0.39 秒后 Snapshot 在 `gst_video_frame_copy_plane` 中用 `0x704903b030` 作为源地址读取并触发 `SEGV_MAPERR`。
- FACT：该地址并非 V4L2 shim 的私有 fake `mmap`；来源是 PipeWire memfd 共享缓冲，因此问题在 PipeWire/GStreamer buffer 生命周期，不是 `/dev/videoN` 的节点存在性。
- 判定：跨线程提前 munmap 是直接原因；下一步要找出线程名、哪个 GstBuffer/stream 触发 unmap，以及为什么旧 frame 仍被持有。
- 状态：139 直接触发链已定位；未修复。

### 16.336 抓线程名与 GStreamer/PipeWire 生命周期日志（进行中，2026-11-28）

- 操作：在 Snapshot 运行窗口内记录 PID 17570/17579 对应的 `/proc/<pid>/task/*/comm`；以 `GST_DEBUG=4/5`、`PIPEWIRE_DEBUG` 复跑，抓 stream/buffer pool 的 stop/negotiate/unmap 顺序。
- 判据：确认提前 unmap 是 stream 重配、buffer pool 释放还是 pipewiresrc 的错误生命周期；修复必须保留共享 buffer 到最后一个 frame 引用释放。
- 状态：执行前记录。

#### 16.336 线程名与日志结果（FACT，2026-11-28）

- PASS：复跑仍为 `exit=139`；线程列表在崩溃前包含 `snapshot`、`pipewiresrc0:sr`、`viewfinderbin-q`、`preview-appsrc:`、`queue0:src`…`queue6:src`、`pw-data-loop`、`gldisplay-event`、`pool-snapshot`。
- FACT：GStreamer 日志确认 viewfinder 分配的是 `YUY2 1920x1080@30`，`size=4147200`；PipeWire 日志显示 `snapshot-43`/`org.gnome.Snapshot-43` stream 启动后在约 0.6 秒内被 suspend/destroy。
- FACT：`PIPEWIRE_DEBUG=3` 显示 `Mod.access` 多次 flatpak check ENOENT、RTKit 不可用、WirePlumber portal camera lookup NotFound，但这些是现有警告，不能直接解释未映射地址。
- 判定：buffer 生命周期仍最可疑；下一步直接打开 `fdmemory`/`pipewiresrc`/`videoflip` 的 map/unmap 计数日志，避免再从高层猜测。
- 状态：线程/管线已确认；139 未修复。

### 16.337 精确抓 fdmemory 映射计数与 videoflip 拷贝（进行中，2026-11-28）

- 操作：用 `GST_DEBUG=fdmemory:7,pipewiresrc:7,videoflip:7,gstpipewirepool:7,parentbuffermeta:7` 复跑 Snapshot，抓 map/unmap 和 buffer 引用日志。
- 判据：确认源 GstFdMemory 是否在 `gst_video_frame_copy_plane` 前被 unmap，以及是否有额外 unref/remove buffer 调用者。
- 状态：执行前记录。

#### 16.337 mapdebug 结果（FAIL/证据不完整，2026-11-28）

- PASS（诊断）：`/tmp/run-snapshot-mapdebug.sh` 完成，Snapshot 仍以 `exit=139` 退出；GStreamer mapdebug 日志成功落盘，共约 243 行。
- FACT：设备日志位于 rootfs `/tmp/snapshot-mapdebug-gst.log`；首轮日志显示 `pipewirepool` 包装 6 个 `MemPtr` buffer，随后出现 `on_param_changed: clear format` 并 remove buffer。
- FACT：实际崩溃的 strace 证据来自 PipeWire `memfd` 共享映射，而本轮第一段日志记录的是 `MemPtr`；两者至少属于不同协商/重配阶段，不能混为同一 buffer。
- FAIL：当前日志尚不足以证明是哪一次 map/unmap 让仍被 `gst_video_frame_copy_plane` 引用的 MemFd frame 失效。
- 判定：139 仍未修复；需要拉完整日志并做 strace TID 对齐。
- 状态：FAIL；下一步拉完整 mapdebug 日志并过滤 `fdmemory/mapped/unmapped/share/remove/got new buffer/parent buffer/videoflip`。

### 16.338 拉取完整 mapdebug 日志并建立时间线（进行中，2026-11-28）

- 操作：从测试宿主 rootfs 拉取完整 `/tmp/snapshot-mapdebug-gst.log`，过滤 map/unmap、pool remove、buffer share/parent、videoflip 和 stream reconfigure 事件。
- 判据：必须区分 `MemPtr` 与 `MemFd` 阶段，并把失效 `memfd` 映射对应到具体释放路径；没有这个证据不做补丁。
- 状态：执行前记录。

#### 16.338 完整 mapdebug 时间线（FACT/根因确认，2026-11-28）

- PASS（诊断）：完整日志共 243 行，成功把失效窗口收敛到 `pipewiresrc0` 的 stream 重配。
- FACT：`0:00:02.735867385` 线程 `0x7164003f00` 执行 `dequeue_buffer:<pipewiresrc0> got new buffer 0x70f4021600`，随后 `0:00:02.736305846` 同一线程把它包装成输出 buffer `0x70c000d580`。
- FACT：仅约 4.6 ms 后，`0:00:02.740483308` 在 `on_remove_buffer:<pipewiresrc0>` 移除同一个 `0x70f4021600`；紧接着该 pool 剩余 7 个 buffer 全部 remove、format clear，stream 从 paused 回到 unconnected 并重新 connect。
- FACT：新协商完成后，`0:00:02.892251615` `videoflip0` 才以 identity 路径开始 `gst_video_flip_transform_frame`；约在此处 `gst_video_frame_copy_plane` 读取先前已被 remove 的 pipewire frame 并触发 139。
- 判定：根因是 PipeWire stream 重配/移除 buffer 时，`gstpipewiresrc` 把下游仍持有的 buffer 立即交回/释放；这不是 Camera2 黑帧、不是 V4L2 节点、也不是 PanVK 崩溃。上游 PipeWire 已有对应修复方向 `bb1bb07f6cdb2a03019b10e9acd3dfb2a611696b`（buffer removal 时先向下游发 flush-start，阻止旧 frame 在 copy 中被移除），需按 1.0.5 代码结构原生回移并上机验证。
- 状态：139 根因已确认；未修复。

### 16.339 回移 PipeWire gstpipewiresrc 的 remove-buffer 生命周期修复（进行中，2026-11-28）

- 操作：核对 PipeWire 1.0.5 的 `on_remove_buffer` 与上游 `bb1bb07f` 差异，按当前结构最小回移；编译插件并在设备上替换后复跑 Snapshot。
- 判据：不得用忽略 SIGSEGV、重复启动、`use-bufferpool=false` 或强制整帧拷贝冒充修复；必须让下游在旧 buffer 被回收前结束使用。
- 状态：执行前记录。

#### 16.339 修复选型校正（FACT，2026-11-28）

- FACT：完整时间线进一步显示 `0:00:02.737685` `pipewiresrc0` 再次进入 `gst_pipewire_src_negotiate`；`02.738` 的 peer caps 仍包含当前 `YUY2 1920x1080@30`，但 `02.739004` 仍主动 `disconnect capture`，随后 `02.740483` remove 刚 dequeue 的 buffer。
- 判定：这不是硬件掉线，也不是必须换 buffer；是 `gst_pipewire_src_negotiate()` 把“固定 current caps”与“仍含范围的 peer caps 第一项”比较失败，导致相同格式也重启 stream。上游 `97699d707df374773fe5d6e32ea7362ef8dcf51b` 正是该根因的原生修复：用 `gst_caps_is_subset(current_caps, possible_caps)` 判断现有 caps 是否仍可接受。
- 决策：采用更窄的“跳过冗余重协商”修复；`bb1bb07f` 作为后续断开保护，不拿它替代本根因修复。不得用 SIGSEGV 忽略、重复启动或强制拷贝。
- 状态：修复方案确定；待编译上机。

### 16.340 编译并部署带 redundant-negotiate 原生修复的 gstpipewire 插件（进行中，2026-11-28）

- 操作：在 PipeWire 1.0.5 源码上加入 `current_caps` 是 peer caps 子集时的快速返回；在 guest 中准备最小开发头文件并编译 `libgstpipewire.so`，替换 rootfs 内插件后复跑 Snapshot。
- 判据：插件必须能正常被 GStreamer 扫描；Snapshot 不再出现同 caps 的 `disconnect capture` 与 remove-dequeued-buffer；窗口真实出现且预览取流。
- 状态：执行前记录。

#### 16.340 插件编译与首轮上机结果（PARTIAL/139 已消失，预览仍 FAIL，2026-11-28）

- PASS（编译）：在 guest 内用 Ubuntu 24.04 arm64 头文件编译 PipeWire 1.0.5 的 `libgstpipewire.so`。首次因 include 路径和 `PACKAGE` 宏失败，修正后成功；strip 后大小 200,920 B，SHA256 `5baacf01e9a98f74a0d3fef88f8da16e9d6e54245b82e871a2b6844665ba7f01`。
- PASS（加载）：用临时 GStreamer registry 加载 `pipewiresrc`，输出 `PIPEWIRESRC_LOADED probe`，插件注册成功；原插件备份 SHA256 `15ac8b33ce86b7099a8eda2b0a539698e680911e01572c162978aef84761eae3`。
- PASS（崩溃修复）：重新拉起桌面并通过 `gio launch` 启动 Snapshot，窗口真实出现，进程保持存活，不再以 139 退出。
- PASS（修复命中）：日志在 `0:00:03.141` 再次进入 `gst_pipewire_src_negotiate`，`0:00:03.141742` 明确输出 `stream is running and new caps are still acceptable; skipping renegotiation`，不再执行先前那条 `disconnect capture`；随后持续 `got new buffer` 与 identity videoflip，流约 30fps。
- FAIL（呈现）：窗口长时间仍为全黑，未见真实相机画面。
- FAIL（buffer 生命周期）：日志有 `buffer_recycle ... can't queue recycled buffer ..., Invalid argument`，并在约 27.57s 出现一次 `buffer ... was not recycled`；后续仍继续出 buffer。当前不能把“进程不崩”当成相机可用。
- 判定：redundant negotiate 导致的 139 已消除；黑屏是新的独立问题，下一步必须抓 pipewiresrc 原始 YUY2 帧做亮度/像素统计，排除相机源黑帧。
- 状态：139 修复通过；Snapshot 预览 FAIL（黑屏）；相机门禁 FAIL。

### 16.341 抓 Snapshot 同一 pipewiresrc 的原始帧并测亮度（进行中，2026-11-28）

- 操作：停止 Snapshot 主进程，编译一个最小 GStreamer appsink 测试器，从 `target-object=42` 拉取 10 帧 YUY2 1920x1080，统计亮度 min/max/mean、非零比例并保存首帧。
- 判据：源帧有正常亮度且非全黑则问题在 Snapshot 呈现/GL；源帧全黑则回到 bridge/Camera2 数据链。不得用窗口出现冒充相机通过。
- 状态：执行前记录。

#### 16.341 appsink 原始帧结果（FAIL/源帧近黑，2026-11-28）

- PASS（工具）：最小 GStreamer appsink 测试器成功从 `target-object=42` 拉取 10 帧，格式/尺寸为 YUY2 1920x1080，`map.size=4,147,200`，并保存首帧 `/tmp/husk-pipewire-frame.yuy2`。
- FAIL（数据）：10 帧亮度统计：`min=0`，`max=3~4`，`mean=0.493~0.617`，`dark(y<=16)=100%`，`bright=0%`。这是近黑源帧，不是 Snapshot 自己的黑背景。
- FACT：PipeWire/V4L2 消费端确实在出帧并回收（约 30fps），但帧内容近黑；139 修复与“画面可用”是两件事。
- 判定：问题已从 Snapshot 崩溃/呈现下移到 `Camera2 -> bridge/V4L2 shim -> v4l2src -> PipeWire` 数据路径。下一步用同一相机节点做 raw Camera2 与 `/dev/video0` V4L2 读取 A/B，确认桥是否把非黑 Camera2 帧交给了 V4L2 consumer。
- 状态：Snapshot 源数据 FAIL；相机门禁 FAIL。

### 16.342 Camera2 与 V4L2 façade 帧内容 A/B（进行中，2026-11-28）

- 操作：复用已通过的 camera fixture/client，先取 Camera2 原始 NV12 帧统计，再从 `/dev/video0` 走 V4L2 mmap/Buffer 读取统计；记录亮度、尺寸、帧数、fps、socket/预加载/进程状态。
- 判据：Camera2 非黑而 V4L2 黑则定位 bridge/V4L2 shim；两边都黑才回 Camera2 HAL/权限/镜头。不得只检查节点存在。
- 状态：执行前记录。

### 16.343 修正 Camera2/V4L2 A/B 的 NV12 亮度统计口径（进行中，2026-11-28）

- 操作：把 `camera-client` 与 `v4l2-test` 的帧输出扩为整帧全字节统计 + NV12 的 Y 平面统计（min/max/mean、暗像素/亮像素比例）；保存原始帧，A/B 只以 Y 平面判黑。
- 判据：禁止再把 NV12 UV 固定值 128 混入 mean；必须分别打印 `all:` 与 `Y:` 字段。
- 状态：执行前记录。

#### 16.343 统计口径修正结果（PASS，2026-11-28）

- PASS（实现）：`camera-client` 现在输出 `all_*` 与 `Y_min/max/mean/dark/bright`；`v4l2-test` 的 mmap/read/readv 三条路径都拆 Y 平面统计。
- PASS（构建）：`cargo check -p camera-client -p v4l2-test` 退出 0；`git diff --check` 无输出。
- FACT：仍未部署、仍未跑设备 A/B；只证明工具代码可编译，不证明相机数据可用。
- 状态：工具 PASS；相机门禁继续按 FAIL 记录，进入实机对账。

### 16.344 编译部署 Camera2/V4L2 A/B 测试器（进行中，2026-11-28）

- 操作：用 `aarch64-unknown-linux-gnu` + `/tmp/nr-toolchain/root` 编译 `camera-client`、`v4l2-test`，推送进测试宿主 rootfs，核对 SHA256 和可执行文件。
- 判据：设备侧 SHA 必须等于本机构建产物；否则不得用设备输出下结论。
- 状态：执行前记录。

#### 16.344 部署核验结果（PASS，2026-11-28）

- PASS（SHA）：设备 `/root/camera-client` = `e585c084bb118e46912e4221b1c61d7055c6b49615edc416e4b89a838d0825b4`；`/root/v4l2-test` = `ebc8cb624fc8dca6ef31e4a1b7e8b232f8f34295cef9bd8943993e51370e90b0`，均与本地 release 产物一致。
- PASS（运行身份现场）：生产 socket `/run/husk/camera.sock` 存在；Android `husk-bridges` PID 19598 存活；测试宿主主进程 PID 19487 存活。
- 状态：工具部署 PASS；相机数据仍待测。

### 16.345 Camera2 路径 Y 平面采样（进行中，2026-11-28）

- 操作：在 guest 内直连生产 `/run/husk/camera.sock`，请求 1920×1080 NV12 10 帧，保存到 guest `/tmp/camera2-ab`；检查 OPEN 格式、每帧 Y 统计、帧率、错误。
- 判据：只按 `Y_*` 判断源亮度；同时保留 `all_*` 作为错误口径对照。黑屏时不得宣称可用。
- 状态：执行前记录。

#### 16.345 首跑入口错误（FAIL，非相机结果，2026-11-28）

- FAIL（原样）：`husk-session: loader not executable: .../ubuntu-24.04/rootfs/libld.so`，退出 69。
- FACT：loader 不在 rootfs 内；本次未创建测试进程、未请求相机帧，不能把该失败归因于 Camera2。
- 纠正：改用 bundle 同级 `.../ubuntu-24.04/libld.so`（以现场 `ls` 为准）后重跑。
- 状态：入口路径 FAIL；相机数据未测。

### 16.346 纠正 loader 路径并重跑 Camera2 采样（进行中，2026-11-28）

- 操作：先查 bundle 内 loader 身份，再以正确 `NR_LD` 重跑同一 10 帧请求。
- 判据：必须进入 camera-client 并收到 OPEN/FRAME；入口错误不得混入相机判定。
- 状态：执行前记录。

#### 16.346 Camera2 socket 原始采样结果（FAIL/源近黑，2026-11-28）

- PASS（传输）：`camera-client` 经生产 `/run/husk/camera.sock` 打开 `id=1`、1920×1080 NV12，10/10 帧成功，收到 `BYE`；seq=1..10。
- FACT（时间戳）：帧 ts 间隔约 33.32–33.79 ms，对应源侧约 29.6–30.0 fps；进程 elapsed_ms=670 是客户端流水请求耗时，不能当作相机采集 fps。
- FAIL（亮度）：`Y_min=0`，`Y_max=21,14,10,8,7,5,5,5,6,6`，`Y_mean=0.53,0.74,0.73,0.74,0.73,0.73,0.73,0.73,0.73,0.73`，`Y_dark=1.000`，`Y_bright=0.000`。
- FACT（错误口径对照）：同批整帧 `all_mean=43.02~43.16`，由 NV12 UV=128 拉高；不能据此说画面不是黑。
- 判定：Camera2 bridge 出口已是近黑 Y；继续测 V4L2 façade 与 PipeWire，区分桥后转换/消费是否再造成变化。
- 状态：Camera2 socket 数据 FAIL；相机门禁 FAIL。

### 16.347 V4L2 façade 同一现场采样（进行中，2026-11-28）

- 操作：不替换生产 socket，在 guest 内跑 `/root/v4l2-test /dev/video0 10 NV12 /tmp/v4l2-ab 1920 1080`，记录 DQBUF、seq/ts、Y 统计、保存 raw/ppm。
- 判据：V4L2 Y 若非黑，问题在 Camera2→V4L2 的转换/映射；若仍近黑，继续看 Camera2 HAL 输出和物理曝光。
- 状态：执行前记录。

#### 16.347 V4L2 首跑 FAIL（preload 缺失，2026-11-28）

- FAIL（原样）：`v4l2test|fd=4|target=Ok("/memfd:husk-v4l2-0 (deleted)")`，随后 `fstat|chr=false`、`FAIL|fd identity is not character device 81:0`。
- FACT：`/dev/video0` 的 stat 已识别为字符设备 81:0，open 也确实落到 Husk 的 `memfd:husk-v4l2-0`；失败点是 fstat ABI 假 fd 需要 preload 接管，而本轮没有加载。
- 纠正：显式 `LD_PRELOAD=/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so` 后重跑，不能把缺少 preload 的失败归因于相机源。
- 状态：命令环境 FAIL；数据待测。

### 16.348 带 V4L2 preload 重跑 façade（进行中，2026-11-28）

- 操作：核对 preload SHA/路径，在 guest 环境显式加载后跑同一 10 帧 mmap/DQBUF。
- 判据：必须通过 fd identity、拿到 Y 统计；同时验证 `fdinfo`/memfd 与相机 socket 连接。
- 状态：执行前记录。

#### 16.348 第二次 V4L2 入口失败与身份纠正（FAIL，非相机结果，2026-11-28）

- FAIL（原样）：`ERROR: ld.so: object '/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so' from LD_PRELOAD cannot be preloaded (cannot open shared object file): ignored.`，随后仍是 fstat 81:0 FAIL。
- FACT：canonical helper 注释已写明：给子进程的 `LD_PRELOAD` 必须用宿主 rooted 路径，例如 `<rootfs>/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so`；guest `/usr/...` 会被 Android linker 在 loader 接管前查找并失败。
- FACT：本轮 manual session 还使用了 rootfs 旁 `distributions/ubuntu-24.04/libld.so`（869888 B, SHA `67f543b9...`），但当前 App 实际运行身份应核对 `files/linux/husk/libld.so`。
- 判定：当前 V4L2 失败仍是入口/运行身份问题，不能算相机源结论；Camera2 socket 采样也需用 canonical loader 复核。
- 状态：入口 FAIL；先核对 canonical runtime，再重跑两路。

### 16.349 核对 canonical loader 与 media 运行身份（进行中，2026-11-28）

- 操作：列出设备 `files/linux/husk/libld.so`、rootfs 旁旧 loader、当前 `husk-bridges`/媒体进程 `/proc/<pid>/exe` 与 SHA；选定 canonical loader 并记录。
- 判据：A/B 必须用 App 当前部署的 loader，不得用 rootfs 旁旧 bundle loader。
- 状态：执行前记录。

#### 16.349 canonical loader 身份结果（PASS，2026-11-28）

- FACT：设备 canonical `files/linux/husk/libld.so` = 1,263,104 B，SHA `6ef6e5bff2b512a11cbbdc3fde1fc86f927731639160d3efe5cc5f55feda3506`。
- FACT：rootfs 旁 `distributions/ubuntu-24.04/libld.so` = 869,888 B，SHA `67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`，确认是旧 bundle。
- FACT：Android `husk-bridges` 可执行文件为 `files/linux/runtime/husk-bridges`，当前 PID 19598；它不是 guest loader，但 Camera2 socket 由它提供。
- 判定：后续 guest 测试显式使用 canonical loader；之前 Camera2 socket 数据在旧 loader 下取得，必须复核后才能作为最终证据。
- 状态：身份 PASS；开始 canonical Camera2 复核。

### 16.350 canonical loader 下 Camera2 复核（进行中，2026-11-28）

- 操作：`NR_LD=<app>/files/linux/husk/libld.so` 重跑同一生产 socket、1920×1080 NV12 10 帧。
- 判据：输出 Y 统计与旧 loader 对比；若仍近黑，再排除 loader 变量。
- 状态：执行前记录。

#### 16.350 canonical Camera2 复核结果（FAIL/源近黑，2026-11-28）

- PASS（运行）：canonical loader `6ef6e5bf...` 下生产 socket 正常 OPEN，10/10 帧，BYE；排除了旧 loader 变量。
- FAIL（亮度）：`Y_mean=0.56~0.79`，`Y_max=20,15,11,9,7,7,10,8,6,14`，`Y_dark=1.000`；与旧 loader 结果同量级。
- 判定：Camera2 bridge 源近黑与 loader 新旧无关。
- 状态：Camera2 数据 FAIL；继续 V4L2/canonical 对账。

#### 16.351 canonical loader + V4L2 preload 对账结果（FAIL/同源近黑，2026-11-28）

- PASS（ABI）：`stat chr=true rdev=81:0`、`fstat chr=true rdev=81:0`、`QUERYCAP driver=husk card=BACK`、NV12 1920×1080、REQBUFS=4、mmap、STREAMON/DQBUF/STREAMOFF 和 `v4l2test|OK` 全部通过。
- PASS（传输）：DQBUF 10 帧，每帧 3,110,400 B，seq=1..10；用户 mmap 指针和 buffer 地址正常。`fps=18.10` 是本次探针 DQBUF 循环吞吐，不能替代相机源 30fps。
- FAIL（亮度）：首帧 `Y_min=0|Y_max=4|Y_mean=0.36|Y_dark=1.000`；末帧 `Y_min=0|Y_max=3|Y_mean=0.50|Y_dark=1.000`。V4L2 与 Camera2 同近黑，且没有变亮。
- 判定：`Camera2 socket -> V4L2 façade -> DQBUF` 没有新增可检测画面；问题不在 V4L2 mmap 地址/长度/转换，而在 Camera2 源或真实曝光/物理暗场。
- 状态：V4L2 数据 FAIL；相机门禁 FAIL。

### 16.352 区分物理暗场与 Camera2 空帧（进行中，2026-11-28）

- 操作：检查 Camera2 bridge 是否有 torch/flash 控制接口与 HAL 曝光日志；在可控补光条件下重取同一 `id`，同一口径统计 Y，并记录 Camera2 capture request/result 的曝光、ISO、timestamp、timeout。
- 判据：补光后 Y 显著上升且纹理出现，才可把当前黑帧归为物理暗场；补光后仍只有 0~4，则继续查 Camera2 request/result 或 bridge 空帧。
- 状态：执行前记录。

#### 16.352 相机 ID 与现有 HAL 日志核对（FACT，2026-11-28）

- FACT：bridge INFO 明确 `id=0 facing=back name=BACK`、`id=1 facing=front name=FRONT`；此前 10 帧黑采样用的是 `id=1` 前摄，V4L2 `/dev/video0` 对应 `card=BACK` 后摄，两枚均近黑。
- FACT：现有 logcat 显示后摄 Camera2 session 正常 `open/repeat/session_active`，HAL 输出 `1920x1080 NV21`，没有 capture timeout；但当前未开启 `HUSK_CAMERA_PLANE_AUDIT`，没有 exposure/ISO/result metadata。
- FACT：logcat 历史中出现 `onTorchStatusChanged`，说明 torch 通路可被 CameraService 接受；这不等于补光后画面可用。
- 判定：需要可控补光 A/B，不能只靠环境猜测。
- 状态：ID/基础 HAL 事实确认；继续 torch 复测。

### 16.353 后摄 torch 补光 Camera2 selftest（进行中，2026-11-28）

- 操作：写入测试宿主 `files/camera-selftest`：`frames=10 camera=0 torch=1`；清空旧帧与 logcat，force-stop 后重启 `MainActivity`，等待 selftest 完成，读取 `files/camera-frames/*.nv12` 与 `camera|` 日志。
- 判据：必须确认 torch rc、session active、10 帧、Y 明显上升/出现纹理；若补光后仍只有 0~4，则排除物理暗场并继续查 Camera2 request/result。
- 状态：执行前记录；测试后会移除 marker，避免污染正常启动。

#### 16.354 torch 证据拉取命令封装 FAIL（未执行，2026-11-28）

- FAIL（原样）：命令在执行前被策略拒绝，原因是包含 `rm -rf`；没有产生设备读取、截图或统计。
- 纠正：使用新的只增目录 `torch-selftest-canonical-20261128-v2`，不删除任何已有证据。
- 状态：命令封装 FAIL，非相机结果。

### 16.355 拉回 torch NV12 并生成 Y/图像证据（重试，进行中，2026-11-28）

- 操作：只增写入新 evidence 目录，拉 10 帧、算 Y、转 PNG/contact sheet。
- 判据：Y 有真实纹理且肉眼可见；仍不代替 Snapshot。
- 状态：执行前记录。

#### 16.355 torch NV12 Y/图像证据结果（PARTIAL，2026-11-28）

- PASS（Y）：10 帧 Y 为 `min=14~15`、`max=122~126`、`mean=50.005~52.547`、`Y_dark=0.000`、`Y_bright=0.000`、`Ylevels=108~113`，证明后摄在 torch 下不是全黑/常数帧。
- PASS（文件）：样本、SHA256SUMS、`y-stats.tsv`、`frame-00.png`、`contact-sheet.png` 已写入 `docs/design/evidence/367-camera-audit-20261128/torch-selftest-canonical-20261128-v2/`。
- FAIL（可识别内容）：肉眼只见左暗右亮的黄灰渐变与噪声，没有可识别的桌面/物体纹理；当前物理取景条件仍不合格，不能把“非黑”冒充可正常使用的相机画面。
- 判定：Camera2 后摄非空；未补光时两枚相机近黑更像物理暗场/取景条件，但最终 Snapshot 必须看到真实纹理。selftest 是强证据链，不是发布门禁通过。
- 状态：源信号 PARTIAL PASS；场景/最终画面仍 FAIL/UNKNOWN。

### 16.356 撤掉 selftest marker 并从 Husk 正常入口进桌面（进行中，2026-11-28）

- 操作：把 `files/camera-selftest` 改名为 `.used` 防止正常启动重复跑；截图/UI 树确认 Husk App 当前页面，再走唯一桌面入口，检查 guest、PipeWire、WirePlumber、Snapshot。
- 判据：必须确认应用是 `com.linuxhub.myth.husk`，且桌面/媒体进程使用 canonical loader；不能拿 selftest 冒充 Snapshot。
- 状态：执行前记录。

#### 16.356 桌面入口与媒体进程检查（PARTIAL，2026-11-28）

- PASS（桌面）：点击 Husk 首页“打开桌面”后 20s 截图，GNOME 桌面首帧真实出现；rootfs 内 `/tmp/.X11-unix/X0`、`/run/dbus/system_bus_socket`、`/run/husk/camera.sock` 均存在。
- PASS（canonical loader）：GNOME Shell 等 guest 进程 `/proc/<pid>/exe` 均解析到 `files/linux/husk/libld.so`，SHA `6ef6e5bf...`；PipeWire/WirePlumber 候进程也显示该 loader 与宿主 rooted `LD_PRELOAD=.../libhusk_v4l2.so`。
- FAIL（命令封装）：一次全 `/proc` 扫描产生大量 `Permission denied`，输出不可用于媒体进程定名；不能用该条作为 PASS。
- 纠正：只读已知同 UID PID，分别输出 cmdline/exe/SHA/env。
- 状态：桌面 PASS；媒体定名待精确复核。

#### 16.357 精确媒体身份结果（PASS，2026-11-28）

- FACT：PID 23316 = rootfs `/usr/bin/gnome-shell`；PID 23411 = `/usr/bin/pipewire`；PID 23424 = `/usr/bin/wireplumber`。
- FACT：三者 `/proc/<pid>/exe` 都是 `files/linux/husk/libld.so`，SHA `6ef6e5bf...`。
- FACT：PipeWire/WirePlumber 的 `LD_PRELOAD` 都是宿主 rooted `<rootfs>/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so`；`NR_ROOTFS` 正确。
- 状态：桌面媒体运行身份 PASS。

### 16.358 在真实 GNOME 会话启动 Snapshot（进行中，2026-11-28）

- 操作：用桌面现有 `DISPLAY=:0`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、`DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk` 和 canonical loader 启动 `/usr/bin/snapshot`，日志落 rootfs `/tmp/snapshot-live.log`。
- 判据：先记录窗口/PID/日志；随后必须截图看到真实预览，并检查 PipeWire `pw-top` RATE 非零。窗口和存活不能算 PASS。
- 状态：执行前记录。

#### 16.358 Snapshot 首轮现场（FAIL/黑屏，2026-11-28）

- PASS（启动）：Snapshot 46.2 PID 27185 存活，真实窗口出现在 GNOME/X11 桌面，未崩溃；日志无 139。
- FAIL（画面）：10s 截图窗口内容纯黑，左下仍转圈；没有真实预览。
- FACT（GPU 警告）：DRI3 不可用、PanVK kbase `KBASE_IOCTL_MEM_EXEC_INIT failed`、PanVK non-conformant 警告仍在；这些是性能/兼容告警，当前不能直接替代相机数据结论。
- 判定：窗口/PID PASS，Snapshot 预览 FAIL。
- 状态：相机门禁 FAIL；继续区分无帧/黑帧/前端协商。

### 16.359 Snapshot 媒体与 PipeWire 运行态取证（进行中，2026-11-28）

- 操作：等 10s 后再截图；采集 `pw-top -b -n 2`、`wpctl status`、PipeWire/WirePlumber 日志、Snapshot 日志，记录 node、format、RATE/ERR/QUEUE 和错误。
- 判据：Node 存在但 RATE=0/ERR 增长说明不产出；RATE 非零但截图黑说明源或呈现；没有 Source 节点说明枚举/链路未接。
- 状态：执行前记录。

#### 16.359 PipeWire 运行态结果（FACT/Preview FAIL，2026-11-28）

- PASS（Source）：`pw-top` 第二次采样显示节点 `38 v4l2_input._sys_class_video4linux_video0` 为 `R RATE=30 YUY2 1920x1080 ERR=0`。
- FAIL（Snapshot 消费）：节点 `43 org.gnome.Snapshot` 为 `R RATE=0 YUY2 1920x1080 ERR=0`；stream 存在但消费速率仍为 0。
- FACT（wpctl）：Video Source `38. BACK (V4L2)` 存在，Stream `43 org.gnome.Snapshot -> 44 input_1 < BACK:capture_1 [active]`。
- FACT（日志）：RTKit 不可用、flatpak access check ENOENT、portal camera Lookup NotFound、libcamera SPA 缺失；这些是既有告警。Portal NotFound 与直接 GStreamer 枚举 camera 的关系仍待证明。
- 判定：相机 Source 在出帧，黑屏发生在 Snapshot 下游消费/协商/呈现；继续抓 pipewiresrc/buffer pool 日志。
- 状态：Source PASS，Snapshot 消费 FAIL；相机门禁 FAIL。

### 16.360 Snapshot GStreamer 消费链路 debug（进行中，2026-11-28）

- 操作：结束 PID 27185，使用 `GST_DEBUG=pipewiresrc:7,pipewirepool:7,fdmemory:7,videoflip:7` 重开 Snapshot 18s，日志落 rootfs `/tmp/snapshot-consume-debug.log`；过滤 negotiate、got new buffer、recycle、not recycled、videoflip、caps、EOS/ERROR。
- 判据：必须定位 RATE=0 是 caps 不匹配、buffer recycle 阻塞、下游等待、还是 Snapshot 未把 sink 推入 PLAYING。
- 状态：执行前记录。

#### 16.360 Snapshot 消费 debug 结果（FACT/根因再收窄，2026-11-28）

- PASS（消费）：`GST_DEBUG` 日志 3,685 行；`pipewiresrc0` 持续 `got new buffer`，`videoflip0` 持续 identity 处理 1920×1080，`recycle buffer` 随后发生，时间间隔约 33ms。
- PASS（稳定性）：18 秒 debug 运行超时退出 `124`，期间没有 139、没有 capture disconnect、没有 renegotiate 循环。
- FACT：前一轮 `pw-top` 的 `RATE=0` 是短采样表象，不能据此断言 Snapshot 完全不消费；连续 debug 证明消费线程约 30fps 推进。
- 判定：当前 Snapshot 黑屏不是“Source 无帧”或“pipewiresrc 不消费”，剩余优先级最高的是源像素近黑；其次才是 Snapshot 下游 GL/呈现。
- 状态：消费链 PASS；预览仍 FAIL；直接拉当前 Source 帧验证。

### 16.361 当前 PipeWire Source 38 原始帧复测（进行中，2026-11-28）

- 操作：停止残留 Snapshot，使用已有 appsink 测试器从当前 `target-object=38` 拉 10 帧 YUY2 1920×1080，统计 Y 并保存首帧；随后记录 Source/Stream 速率。
- 判据：若 Y 仍近黑而 torch selftest 非黑，则当前普通预览受物理暗场/曝光条件阻塞；若 Y 正常而 Snapshot 黑，则修下游呈现。
- 状态：执行前记录。

#### 16.361 首次 node-38 测试命令 FAIL（未出帧，2026-11-28）

- FAIL：包装命令包含 `pkill -f "/usr/bin/snapshot"`，匹配到自身命令串并杀掉了包装 shell；没有测试器输出。
- 判定：命令封装错误，不是 PipeWire/相机结果。
- 纠正：不再 pkill；debug Snapshot 已被 timeout 结束，直接运行编译好的 node-38 测试器。
- 状态：命令 FAIL；重试。

#### 16.361 node-38 直拉结果（FAIL/入口未成立，2026-11-28）

- FAIL（原样）：测试器输出 `SET_PLAYING_FAILED`，没有 frame 行。
- 判定：node 38 可能已随 Snapshot 退出重编号/销毁，或 caps/target-object 不匹配；本次不能用来判断源像素。
- 下一步：重新读当前 `wpctl status` / `pw-cli` 节点 ID，再用当前 Video/Source ID 重跑。
- 状态：FAIL；先刷新节点身份。

#### 16.361 节点标识纠正（FACT，2026-11-28）

- FACT：当前 Video/Source `id=38` 的 `object.serial=42`；`gstpipewiresrc` 的 `target-object` 使用 serial，不是 `pw-cli` 的 global id。
- FACT：改用 id 38 时日志是 `connect capture ... target-object 38` → `stream error: target not found`；caps 本身已成功匹配 `YUY2 1920x1080@30`。
- 纠正：使用原测试器 `target-object=42`（即当前 BACK Source 的 object.serial）复测；这不是回到 Snapshot 节点。
- 状态：标识错误已排除，重跑源帧。

#### 16.361 当前 BACK PipeWire 源帧结果（FAIL/近黑，2026-11-28）

- PASS（链路）：target-object=42 成功从当前 Video/Source 拉取 10 帧 YUY2 1920×1080，`FRAMES=10`，无 target/caps 错误。
- FAIL（亮度）：`Y_min=0`、`Y_max=3`、`Y_mean=0.364~0.479`、`Y_dark=1.000`，10 帧仍是近黑。
- 对照：torch selftest 的 Camera2 后摄 Y_mean=50~52；同一传感器在补光下可出可见信号。
- 判定：Snapshot 黑屏的直接原因是普通预览源近黑；pipewiresrc/videoflip 消费路径约 30fps 正常，139 修复仍有效。
- 状态：相机门禁 FAIL（源亮度）；继续用 CameraService 外部 torch 做同一 PipeWire 路径 A/B。

### 16.362 CameraService 外部 torch A/B（进行中，2026-11-28）

- 操作：查询 Android 13 `cmd media.camera` 是否支持 torch；若支持，对后摄开启 torch，再立即从同一 Video/Source serial 42 拉 10 帧并按 Y 判黑；结束后关闭 torch。
- 判据：同一 PipeWire 路径在 torch 前后 Y 必须出现显著差异；命令不支持则明确记外部阻塞，不伪造。
- 状态：执行前记录。

#### 16.362 外部 torch A/B 结果（外部阻塞，2026-11-28）

- FAIL（能力）：Android 13 `cmd media.camera` 只有 uid-state/rotation/image-dump/mute/watch，没有 torch；`cmd -l` 也没有独立 torch service。
- 判定：当前测试环境无法从 shell 对生产 Camera2/V4L2 路径开补光；只能靠 Camera2 request 内 torch selftest，而 selftest 完成后不会把同一亮度保留到 PipeWire Snapshot。
- 证据边界：torch selftest 已证明后摄传感器/session/request 可出非黑 Y（mean≈50–52），普通 PipeWire Source 同期仍是 Y_max=3；最终纹理/拍照因设备取景面向暗且无远程物理补光而阻塞。
- 状态：139 修复 PASS；PipeWire 消费 PASS；torch 源数据 PARTIAL PASS；Snapshot 真实画面 FAIL/外部阻塞（物理取景），不能发布为相机全过。

### 16.363 转向剩余 0.0.37 门禁（进行中，2026-11-28）

- 操作：保持相机 FAIL/外部阻塞结论不掩盖，检查 TODO-0.0.37 与 gate evidence 的未完成项，优先跑不会破坏工作区的构建/单测/打包/安装门禁。
- 判据：每项记录原命令、退出码、关键输出与证据路径；构建成功不冒充运行成功。
- 状态：执行前记录。

### 16.364 0.0.37 主门禁 make check（进行中，2026-11-28）

- 操作：`NDK=/Users/l/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64 GUEST_SYSROOT=/tmp/nr-toolchain/root make check`。
- 判据：退出码必须为 0；记录 CI/PanVK/文档/版本/资产关键输出。失败原样保留。
- 状态：执行前记录。

#### 16.364 make check 第一次结果（FAIL/rustfmt，2026-11-28）

- FAIL（原样）：`cargo fmt --all --check --manifest-path runtime/Cargo.toml` 报 `Diff in .../camera-client/src/main.rs:190` 与 `.../v4l2-test/src/main.rs:153`，仅元组换行格式不同；`make: *** [check] Error 1`。
- FACT：没有进入后续 CI/PanVK/文档门禁，不能写 make check 全绿。
- 纠正：只格式化这两个包，随后 `git diff --check` 并完整重跑。
- 状态：FAIL；格式修复中。

### 16.365 格式修复与 make check 重跑（进行中，2026-11-28）

- PASS（格式）：`cargo fmt -p camera-client -p v4l2-test` 退出 0，`git diff --check` 无输出；未格式化其他 workspace 文件。
- 操作：用同一 NDK/sysroot 完整重跑 `make check`。
- 判据：必须退出 0 并看到完整后续检查，不得只凭格式命令写 PASS。
- 状态：执行前记录。

#### 16.365 make check 重跑结果（PASS，2026-11-28）

- PASS：`make check` 退出 0。
- PASS：workspace `cargo check` 完成；143 个 shell 脚本语法通过；374 个 Markdown 文件、0 broken links；`audit-panvk-bundle` PASS（bundle `47d11cd2...`，driver `5e063247...`）；loader 复现 PASS（`6ef6e5bf...`, 1,263,104 B）；VERSION 0.0.37 一致。
- FACT：编译仍有 warning（drm-shim unused_mut、drm-preload visibility/clashing extern、app-rs 若干 FFI 声明等），未阻塞门禁；不能写成零警告。
- 状态：make check PASS。

### 16.366 0.0.37 单测门禁 make test（进行中，2026-11-28）

- 操作：使用同一 NDK/sysroot 运行 `make test`。
- 判据：退出码 0，记录测试数量/失败/跳过与关键输出；构建通过不能替代。
- 状态：执行前记录。

#### 16.366 make test 结果（PARTIAL/仅构建测试二进制，2026-11-28）

- PASS（构建）：`make test` 退出 0，生成 `gpu_env-*`、`nr_tools-*`、`runtime_core-*` 三个 aarch64 test executable。
- FAIL（执行口径）：该目标只有 `cargo test ... --no-run`，**没有运行任何测试用例**；不能写成“单测通过”。
- FACT：真实执行入口是 `make test-device` / `tools/device-tests/run-unit-tests.sh`。
- 状态：测试二进制构建 PASS，单测执行 NOT RUN。

### 16.367 真实 unit tests 执行（进行中，2026-11-28）

- 操作：核对当前 bundle 是否有 `make test-device` 假设的 `./husk`；若有按目标跑，若没有则用 canonical `husk-session.sh` 直接运行三份 test binary，每份 `--test-threads=1`。
- 判据：必须出现每个 binary 的 `test result: ok`，失败/0 tests 均不算 PASS。
- 状态：执行前记录。

#### 16.367 real unit tests 结果（PASS with coverage caveat，2026-11-28）

- PASS：`make test-device` 退出 0；`UNIT_TESTS: PASS`。
- FACT：`gpu_env` 6 passed / 0 failed；`nr_tools` 7 passed / 0 failed；`runtime_core` **0 passed / 0 failed / 0 ignored**。
- 判定：已实际执行 13 个 aarch64 测试，不是 `--no-run`；但 `runtime_core` 没有测试用例，不能把它写成有覆盖。
- 状态：已执行测试 PASS（13 tests），runtime-core 覆盖缺口 NOT VERIFIED。

### 16.368 发布资产 make dist 与内容审计（进行中，2026-11-28）

- 操作：运行 `make dist`，记录产物路径/大小/SHA256；执行现有 runtime/package/release audit，并检查无 `._*`、无私路径、无旧实验脚本。
- 判据：只产 Husk runtime 资产；任一断言失败即 FAIL，不得用源目录代替 dist。
- 状态：执行前记录。

#### 16.368 make dist 首次结果（FAIL/私路径，2026-11-28）

- PASS（构建与内置断言）：`make dist` 退出 0，生成 11 MB `dist/husk-arm64.tar.gz`，SHA `fc39a102f3ecb68f136a56a6ac5c4549ab8e2380d6b15e47bab0b3de95163809`；47 entries、AppleDouble=0、无明显旧实验脚本名。
- FAIL（独立内容扫描）：包内命中 20 个开发机私路径字符串，分布在 `husk-usermgr`、`husk-symlinks`、`husk-window-wait`、`audio-relay`、`audio-capture`、`audio-probe`。
- FACT：私路径来自 `.rustup` standard-library panic/debug location，`llvm-strip --strip-all` 后仍保留；不是脚本运行时数据。
- FAIL（构建根因）：guest tools 构建没有 remap path prefix；现有 dist 断言也未覆盖此边界。
- 纠正：验证 `--remap-path-prefix=$HOME=/build/home` 后固化到 `tools/guest-config/build-guest-tools.sh`，新增 tar 私路径审计并在 `make dist` 中硬失败。
- 状态：首次 dist FAIL，修复构建中。

#### 16.369 私路径 remap 验证（PASS，2026-11-28）

- FAIL（第一次）：误写 `-C remap-path-prefix`，rustc 报 unknown codegen option，未产生新文件。
- PASS（第二次）：正确使用 `--remap-path-prefix=$HOME=/build/home`，`usermgr` 重编成功；`strings` 中 `/Users/l/` 从 4 降到 0，`/home/l/` 为 0。
- 状态：remap 方案有效，固化中。

#### 16.369 第二次 make dist 结果（FAIL/私路径收敛，2026-11-28）

- PASS（guest tools）：新私路径审计不再报 `husk-*`/`audio-*`，说明 HOME/SYSROOT remap 对主 guest tools 生效。
- FAIL（原样）：`private paths: guest/libhusk_v4l2.so`，随后 `private paths: FAIL (dist/husk-arm64.tar.gz)`，`make: *** [dist] Error 2`。
- 根因：`make preloads` 的 `PRELOAD_RUSTFLAGS` 没有 remap；该 .so 独立于 `build-guest-tools.sh`。
- 纠正：给 preload 与 logd 构建加同样的 `--remap-path-prefix`，完整重打。
- 状态：dist FAIL；继续修复构建链。

#### 16.369 第三次 make dist 结果（FAIL/Make 展开错误，2026-11-28）

- FAIL（原样）：`/bin/sh: GUEST_SYSROOT: command not found`。
- FAIL（原样）：`private paths: guest/libhusk_v4l2.so`，随后 `private paths: FAIL`。
- PASS：`runtime boundary: PASS` 仍通过。
- 根因：Makefile 将 remap 参数写为 `$$(GUEST_SYSROOT)`，传给 shell 后触发命令替换；应传变量值 `$(GUEST_SYSROOT)`。
- 根因（待验证）：`libhusk_v4l2.so` 的私路径疑似来自 linker 自动写入的绝对输出路径/SONAME，而非普通 Rust debug 字符串。
- 状态：第三次 dist FAIL；旧 SHA `fc39a102...` 作废，不得发布。下一步先核对动态节，再给 preload 显式 SONAME 并重跑审计。

### 16.370 preload 私路径与 SONAME 根因核对（进行中，2026-11-28）

- 操作：检查 Makefile 的 `GUEST_SYSROOT` remap、preload Rust link 参数、`readelf -d`，并定位私路径出现区段。
- 判据：确认根因后做最小构建修复；不以 strip 或删除字符串掩盖。
- 状态：执行前记录。

#### 16.370 preload 私路径与 SONAME 根因核对（PASS/根因确认，2026-11-28）

- FACT：`llvm-readobj --version-info` 显示私路径是 `.gnu.version_d` 的 `BASE` 基版本名。
- FACT：该值正是 `.../release/build/v4l2-preload/<hash>/out/libhusk_v4l2.so`，不是源码 panic location。
- FACT：动态表当前没有 `SONAME`；链接器因而用绝对输出路径生成基版本名。
- FACT：Makefile 同时确实存在 `$$(GUEST_SYSROOT)` 的错误展开。
- 纠正：Makefile 改用 `$(GUEST_SYSROOT)`；preload 显式传 `-Wl,-soname,libhusk_v4l2.so`；构建后复查 `.gnu.version_d`。
- 状态：根因核对 PASS；修复与重验进行中。

#### 16.370a SONAME 首次修复（FAIL/参数层级错误，2026-11-28）

- FAIL（原样）：`rust-lld: error: unknown argument '-Wl,-soname,libhusk_v4l2.so'`，`make preloads` 退出 2。
- 根因：`-Wl,...` 是 clang 驱动语法，不能直接传给 rustc 调用的 `rust-lld`。
- 附带错误：该参数被放在公共 `PRELOAD_RUSTFLAGS`，连 `drm-preload` 也继承了 v4l2 的 SONAME。
- 纠正：公共 flags 保持中性；只在 v4l2-preload 的 build 命令加 `-C link-arg=-soname=libhusk_v4l2.so`。
- 状态：首次修复 FAIL；重试。

#### 16.370b SONAME 第二次修复（FAIL/命令未生效+NDK未设，2026-11-28）

- PASS：`drm-preload` 与 `v4l2-preload` 均重新编译完成。
- FAIL：Python 字符串替换未命中，实际 v4l2 命令仍没有 SONAME 参数；未构成修复验证。
- FAIL（原样）：`tools/ci/audit-preload-exports.sh: line 37: /bin/llvm-nm: No such file or directory`，随后所有导出检查假阴性，`make preloads` 退出 2。
- 根因：本次 shell 未导出 `NDK`，`NM` 展开为 `/bin/llvm-nm`；不是导出符号消失。
- 纠正：直接替换 Makefile recipe 行；使用 `/Users/l/Library/Android/sdk/ndk/28.2.13676358` 重跑。
- 状态：第二次修复 FAIL；继续。

#### 16.370c SONAME 第三次修复（PASS，2026-11-28）

- PASS：`NDK=<NDK toolchain bin> make preloads` 退出 0，`preload exports OK`。
- PASS：`llvm-readobj --dynamic-table` 显示 `SONAME: libhusk_v4l2.so`。
- PASS：`.gnu.version_d` 基版本名由绝对输出路径变为 `libhusk_v4l2.so`。
- PASS：`strings libhusk_v4l2.so | rg '/Users/l'` 无输出。
- PASS：33 项 preload 导出检查全部 `OK`。
- FACT：仍保留 linker warning（重复 version reassignment），但不影响本轮导出审计；不得写成零 warning。
- 状态：SONAME 私路径修复 PASS。

### 16.371 私路径审计负向测试（进行中，2026-11-28）

- 操作：新增正/负 tar 测试并接入 `make check`；负例含 `/Users/alice/private-path`，审计必须拒绝。
- 判据：正例 PASS、负例 FAIL，测试进程自身退出 0。
- 状态：执行前记录。

#### 16.371a 私路径审计测试首次执行（FAIL/测试断言，2026-11-28）

- PASS：审计已正确识别并拒绝负例，原样输出 `private paths: ./guest/libexample.so`。
- FAIL：测试脚本只 grep `guest/libexample.so` 的精确形态，未接受 tar 自动加的 `./` 前缀，因此退出 1。
- 纠正：断言改为匹配 `guest/libexample.so` 子串，不放宽审计脚本。
- 状态：测试断言失败；修复后重跑。

#### 16.371b 私路径审计负向测试（PASS，2026-11-28）

- PASS：`sh tools/ci/test-runtime-private-paths.sh` 退出 0。
- PASS：干净 tar 被接受（`private paths: PASS`）。
- PASS：含 `/Users/alice/private-path` 的 tar 被拒绝，并指出 `guest/libexample.so`。
- FACT：测试已接入 `make check`。
- 状态：审计负向测试 PASS。

### 16.372 第四次 make dist（进行中，2026-11-28）

- 操作：用 NDK 28.2 toolchain bin 重跑 `make dist`，记录退出码、tar 大小/SHA、entries、边界/私路径审计结果。
- 判据：退出 0 且 runtime boundary、private paths、AppleDouble、旧实验脚本边界全 PASS。
- 状态：执行前记录。

#### 16.372 第四次 make dist（PASS，2026-11-28）

- PASS：`NDK=<NDK 28.2 toolchain bin> make dist` 退出 0。
- PASS：产物 `dist/husk-arm64.tar.gz`，11,624,581 bytes，SHA256 `d9649ed9cb0b490c92cf2136a3e1e5238013dcb0598ee4fe1391ffcbdc4e4944`。
- PASS：`runtime boundary: PASS`；`private paths: PASS`。
- PASS：独立复核 47 entries、AppleDouble=0、旧实验脚本命中=0、私路径文件=0。
- PASS：`RUNTIME-IDENTITY` 为 `commit=c924d620f6de`、`version=0.0.37`、loader `6ef6e5bf...`。
- FACT：旧包 SHA `fc39a102...` 作废；本目标后续只认 `d9649ed9...`。
- 状态：make dist PASS，发布资产内容门禁已过。

### 16.373 最新改动后的 make check（进行中，2026-11-28）

- 操作：使用 NDK 28.2.13676358 + `/tmp/nr-toolchain/root` 重跑完整 `make check`。
- 判据：退出 0；包含新增私路径负向测试；记录 shell/docs/PanVK 关键输出；warning 照记。
- 状态：执行前记录。

### 16.374 PipeWire 139 修复固化（进行中，2026-11-28）

- 操作：定位设备/临时构建中的 `gst_caps_is_subset()` 修改，转成 Husks 仓库内可重复构建/安装/审计的补丁资产。
- 判据：clean runtime 构建或安装路径会自动应用补丁；设备运行时 SHA 可自证；不得依赖 `/tmp` 手工文件。
- 状态：执行前记录。

#### 16.373 最新改动后的 make check（PASS，2026-11-28）

- PASS：`NDK=<NDK 28.2 toolchain bin> make check` 退出 0。
- PASS：145 shell scripts 语法检查；374 Markdown、0 broken links。
- PASS：新增 `runtime private paths test: PASS` 在 `make check` 中执行。
- PASS：PanVK bundle `47d11cd2...`；loader 复现 `6ef6e5bf...`，1,263,104 B；VERSION 0.0.37。
- FACT：Cargo/Rust warning 仍大量存在；不能写成零 warning。
- 状态：make check PASS。

#### 16.374a PipeWire 临时源码核对（FACT，2026-11-28）

- FACT：设备插件 SHA `5baacf01e9a98f74a0d3fef88f8da16e9d6e54245b82e871a2b6844665ba7f01`，rootfs 包为 `gstreamer1.0-pipewire 1.0.5-1ubuntu3.3`。
- FACT：临时源码位于 rootfs `tmp/husk-pw-build/src/src/gst/gstpipewiresrc.c`，SHA `9e289d0d...`；官方 PipeWire 1.0.5 原文件 SHA `c3edb98b...`。
- FACT：diff 只有 `gst_pipewire_src_negotiate()` 一个逻辑点：streaming 时 `current_caps` 是完整 peer caps 子集则跳过重协商；并把 `pw_thread_loop_lock()` 前移到判断前。
- FACT：上游对应提交 `97699d707df374773fe5d6e32ea7362ef8dcf51b`。
- 状态：来源与 delta 核对 PASS；固化进行中。

#### 16.374b PipeWire 修复固化资产（PASS，2026-11-28）

- PASS：新增 `vendor/pipewire/1.0.5-gst-caps-subset.patch`、完整 patched `gstpipewiresrc.c`、`libgstpipewire.so`、`COPYING`、`PROVENANCE.md`。
- PASS：插件 SHA `5baacf01...`、大小 `200920 B`，且包含修复日志标记。
- PASS：负向测试确认篡改插件被拒绝；patch 反向得到官方 1.0.5 原文 SHA `c3edb98b...`，正向得到 vendored SHA `9e289d0d...`。
- PASS：`make check` 退出 0；147 scripts、374 Markdown/0 broken links；新增插件审计在 check 中执行。
- FACT：安装器只在目标已有 `gstreamer-1.0/libgstpipewire.so` 时替换，并安装前后校验 SHA；runtime identity 增加 `pipewire_plugin_sha256`。
- 状态：源码、构建产物、审计和安装链固化 PASS。

### 16.375 第五次 make dist（带 PipeWire 139 修复，进行中，2026-11-28）

- 操作：重建 dist，验证插件/补丁/来源文件入包、插件审计 PASS、新 SHA。
- 判据：退出 0；dist 自带修复；私路径仍 PASS；旧包不得复用。
- 状态：执行前记录。

#### 16.375 第五次 make dist（PASS，2026-11-28）

- PASS：带 PipeWire 139 修复的 `make dist` 退出 0。
- PASS：产物 SHA256 `57d253cba59b4d2d6c1bbcd50233f006d6a20267d0cad4f7f668bcbc130c2ecf`，11,694,788 B，56 entries。
- PASS：independent scan：AppleDouble=0、旧实验脚本=0、私路径文件=0。
- PASS：包内插件 SHA `5baacf01...`，patched source SHA `9e289d0d...`。
- PASS：`RUNTIME-IDENTITY` 记录 `pipewire_plugin_sha256=5baacf01...`。
- FACT：`d9649ed9...` 也不含本修复，作废；候选唯一为 `57d253cb...`。
- 状态：dist PASS。

### 16.376 部署第五次 dist 到 Husk App（进行中，2026-11-28）

- 操作：把 tar 送入 `com.linuxhub.myth.husk` 私有目录，解包后用 `install-husk.sh --upgrade --dest <current-rootfs> --loader <current-loader>` 部署。
- 判据：安装退出 0；设备插件 SHA `5baacf01...`；`etc/husk/runtime-identity` 含版本/commit/loader/pipewire SHA。
- 状态：执行前记录。

#### 16.376 部署第五次 dist 到 Husk App（PASS，2026-11-28）

- PASS：upgrade 安装退出 0；安装器明确写入 `gstreamer-1.0/libgstpipewire.so (Snapshot 139 fix)`。
- PASS：设备插件 SHA `5baacf01e9a98f74a0d3fef88f8da16e9d6e54245b82e871a2b6844665ba7f01`，修复日志标记存在。
- PASS：`23-runtime-identity.sh` 重算结果 `PASS=12 FAIL=0`。
- PASS：identity 中 loader `6ef6e5bf...`、libc、gpu-env、PanVK、ICD、PipeWire 插件六项哈希与磁盘一致。
- FACT：部署前旧 identity 的 loader 哈希确实漂移；本次已纠正。
- 状态：新候选 runtime 部署与自证 PASS。

### 16.377 新部署桌面与 Snapshot 139 回归（进行中，2026-11-28）

- 操作：启动 Husk App/桌面，确认 PipeWire/WirePlumber 唯一入口，再启动 Snapshot 并观察退出码、renegotiate 修复标记、缓冲消费。
- 判据：无 139；无同 caps 的 disconnect/renegotiate 循环；窗口进程可持续运行。画面亮度另判。
- 状态：执行前记录。

#### 16.377a 首次 Snapshot 20s 验证（FAIL/测试身份错误，2026-11-28）

- PASS：Snapshot 进程 20 秒内保持存活，PID 8780，主动 TERM 后 rc=143；139 未复现。
- FAIL：窗口只显示灰色预览区+转圈，Snapshot 报 `Could not start camerabin: Element failed to change its state`。
- FACT：pipewiresrc 日志显示 `connect capture with path (null), target-object (null)`；PipeWire 随后报 `target not found`。
- FACT：PipeWire 日志出现 `[husk v4l2] panic`；Snapshot stderr 同时报 `XDG_RUNTIME_DIR (/tmp/runtime-husk) 不属于本进程 (uid 0)，而属于 uid 1000`。
- 根因：使用 `guest-correct.sh` 启动了 uid 0 测试进程，没有走正式桌面用户 husk；不是 139 回归，也不能据此判相机失败。
- 纠正：改用 `husk-session.sh ... --user husk`，显式设置 `DISPLAY=:0`、`XDG_RUNTIME_DIR=/tmp/runtime-husk`、session bus。
- 状态：测试入口 FAIL；按正式用户重跑。

#### 16.377b 正式 husk 用户重跑与媒体栈污染核对（进行中，2026-11-28）

- 操作：核对第二轮 `husk-session.sh --user husk` 的 Snapshot 结果、截图、进程退出码，以及 PipeWire/WirePlumber 是否已被上一轮 uid 0 测试打崩。
- 判据：明确区分 139 修复结果、媒体栈状态和相机画面结果，不用被污染的一轮冒充相机结论。
- 状态：执行前记录。
- FAIL：第二轮虽然以 guest `husk` 启动 Snapshot，但截图 `/tmp/husk-snapshot-user-57d253.png` 仍为 `No Camera Found`，不能据此判定相机链路失败。
- FACT：当前进程表中没有 `pipewire -c`、`wireplumber -c`；同时 `/tmp/runtime-husk/pipewire-0` 是上一轮遗留 socket。
- FACT：`/tmp/husk-pipewire.log` 尾部有 `[husk v4l2] panic`，`/tmp/husk-wireplumber.log` 尾部有 `disconnected from pipewire`。上一轮 uid 0 测试已把媒体栈打崩。
- 判定：139 修复在这两轮均未复现；相机结果仍为 NOT VERIFIED。必须先通过唯一入口以 guest `husk` 重建媒体栈，再做 20 秒 Snapshot 验证。
- 状态：FAIL（测试顺序污染）；纠正中。

#### 16.377c 以 guest husk 通过唯一媒体入口重建 PipeWire/WirePlumber（进行中，2026-11-28）

- 操作：停止并清理旧 socket/残留，使用 `husk-session.sh --user husk` 仅启动 `/usr/local/bin/husk-media-session start`，随后读回状态、进程身份、Video 节点和日志。
- 判据：PipeWire/WirePlumber ready；guest husk；Video 设备出现 BACK/FRONT；无 uid 0 警告；无 `[husk v4l2] panic`。
- 状态：执行前记录。
- PASS：通过 `files/linux/husk/husk-session.sh <rootfs> --user husk --cwd /home/husk -- usr/bin/env ...` 调用唯一入口 `husk-media-session start`，PipeWire PID `10719`、WirePlumber PID `10731` 均 ready。
- PASS：`wpctl status` 报告 PipeWire `1.0.5, husk@localhost`，Video 设备 `39 BACK [v4l2]`、`40 FRONT [v4l2]`，源节点 `BACK (V4L2)`、`FRONT (V4L2)`。
- PASS：两个媒体进程的 maps 均含当前 `files/linux/husk/libld.so` 和 rootfs `libhusk_v4l2.so`；本轮 PipeWire 日志为相机 INFO 成功、无 `[husk v4l2] panic`、无 `target not found`。
- FACT：`/dev/video*` 是 shim 在 guest 视角提供的 façade，不是 Android 文件系统里的实体节点，因此宿主侧 `ls rootfs/dev/video*` 为空不作为失败。
- 状态：PASS（媒体服务与 V4L2 枚举恢复）。

#### 16.377d 将 Snapshot 回归固化为可重复设备测试并执行（进行中，2026-11-28）

- 操作：把当前只存在于 rootfs `/tmp` 的 Snapshot 测试迁入 `tools/device-tests/`，固定 guest husk、DISPLAY、runtime dir、session bus，先检查媒体栈，再启动 Snapshot 20 秒并收集 GST/PipeWire 日志、结果和截图。
- 判据：脚本可重复执行；媒体栈 ready；Snapshot 存在 20 秒；无 139；日志含真实 PipeWire/V4L2 协商；画面结果单独判。
- 状态：执行前记录。
- FAIL：新脚本首次执行在 host 第 40 行报 `run_husk: unbound variable`；原因是把命令替换误写成 `media=$run_husk ...`，尚未进入设备测试。
- 纠正：改为 `media=$(run_husk ...)` 和 `wp=$(run_husk ...)`，重新做 shell 语法检查后执行。
- FAIL：第二次运行媒体栈四项 PASS，但结果文件读取失败；同时 GST 日志出现 null target，Snapshot 20 秒存活与退出码无法判定。
- FACT：输出目录 `/tmp/husk-snapshot-pipewire-28`；截图已生成。下一步检查 `snapshot-run.log` 与结果文件读取错误，先区分路径搬运错误和设备侧真实失败。
- 根因已分开：设备结果文件实际存在且为 `alive_after_20s=true`、`termination=clean_term`、`intentional_term_rc=143`；host 检查误用了 `snapshot.result`，而首版搬运写成 `snapshot-result`。
- FACT：GST 首条 `target-object (null)` 是枚举阶段，随后同一轮出现 `target-object 46` 并持续 dequeue/recycle buffer；不能把枚举阶段的 null 当成失败。
- FACT：设备 `husk-pipewire.log` 显示连续 `DQBUF send FRAME`、`DQBUF ok`，`videoflip` 收到 1920x1080；这说明真实帧已从 Camera2/V4L2/PipeWire 到达下游。截图预览区仍为黑色，亮度/物理场景单独判。
- 纠正：修正结果命名，采集设备全局 V4L2 增量；只要求出现有效数字 target-object，不再要求枚举首条非 null。
- PASS：修正后 `28-snapshot-pipewire.sh` 返回 `PASS=12 FAIL=0`，Snapshot PID `11627` 存活满 20 秒，主动 TERM 返回 143，未出现 139。
- PASS：GST 日志命中 `stream is running and new caps are still acceptable; skipping renegotiation`；枚举首条 null target 之后命中真实 `target-object 46`。
- PASS：V4L2 增量日志包含连续 `DQBUF send FRAME` / `DQBUF ok`，证明 Camera2→helper→socket→V4L2→PipeWire→GStreamer 的下游消费链路在 20 秒内持续工作。
- PASS：证据已固化到 `docs/design/evidence/367-snapshot-20s-20261128/`；截图 SHA256 `98cf581ac8cb21a836bedeea54695eb517d4b125c9fcce198dd22f7e0f0ce043`。
- FAIL：截图取景区域仍黑。既有补光实验已证明 torch 下 Camera2→NV12 可输出非黑帧，因此当前黑画面不能单独判成 139 或数据链失败；但“普通环境 Snapshot 可见画面”仍未通过，相机发布门禁保持 FAIL。
- 状态：139 回归与媒体链 PASS；可见画面 FAIL/物理曝光待判。测试已固化。

#### 16.378 V4L2 shim 原生防 panic/越界（进行中，2026-11-28）

- 观察：历史 uid 0 污染测试出现 `[husk v4l2] panic`。`v4l2-shim` 当前会在 UNIX socket 路径超过 `sockaddr_un` 容量时切片越界，并在 NV12 负载短于声称尺寸时无边界读取。
- 操作：在 `Ctx::init` 拒绝超长 socket 路径；给 `nv12_to_yuyv` 增加源长度和输出容量契约，负载不足返回 EIO/EINVAL，不做 panic、不忽略 SIGSEGV；补正常、短负载、超长路径负向单测。
- 判据：`cargo test -p v4l2-shim`、格式检查、loader 构建通过；重新部署后 Snapshot 20 秒回归保持 PASS。
- 状态：执行前记录。
- FAIL：在 macOS 主机默认 target 直接跑 `cargo test -p v4l2-shim`，AArch64 内联汇编寄存器 `x0..x8` 被 x86_64 编译器拒绝；这是测试命令 target 错误，不是新增边界代码失败。
- 纠正：改用与仓库设备门禁相同的 `aarch64-unknown-linux-gnu` target、NDK clang 和 `/tmp/nr-toolchain/root` 构建测试二进制，再在设备 guest 中执行。
- PASS：AArch64 target 构建成功；设备 guest 内测试 `4 passed; 0 failed`（NV12 尺寸正常/异常、ioctl 低 32 位、UNIX 路径上限）。
- 已知：编译仍有既存 warning（未使用赋值/dead code），不写成零 warning。
- 下一操作：重建嵌入 loader 和 v4l2 preload，部署后复跑媒体栈与 Snapshot。

#### 16.379 重建并部署带 V4L2 边界修复的 loader/preload（进行中，2026-11-28）

- 操作：执行格式检查、`make loader`、`make preloads`，核对 loader/v4l2 SONAME/SHA，再原子替换 App canonical loader 与 rootfs preload；重启媒体栈并复跑 Snapshot。
- 判据：构建和 preload audit PASS；设备 SHA 与本地一致；媒体栈仍 ready；Snapshot 20 秒回归 PASS；无 panic。
- 状态：执行前记录。
- FAIL：部署新 loader/preload 后 Snapshot 仍 20 秒存活、无 139、caps skip 和真实 target 都 PASS，但本轮未出现 V4L2 `DQBUF send FRAME`，所以数据链回归判 FAIL。
- FACT：新设备 SHA：loader `5a28f920...`、v4l2 preload `be263cd0...`；媒体栈 PID 12382/12393 ready。
- 操作：立即检查 Snapshot/V4L2/helper 日志，区分流启动失败、Camera2 会话忙碌或新边界检查误拒绝。
- 根因：媒体栈由测试脚本启动时没有设置 `NR_V4L2_AUDIT=1`，所以新 PipeWire 进程不打印 DQBUF；并非新边界检查拒绝帧。
- FACT：同一时段 `adb logcat -s Husk:I` 显示 `camera|capture|started count=3750/3840`、`camera|frame|ready seq=420/510 1920x1080 bytes=3110400`，Camera2 实际连续出帧；本轮失败是测试观测缺失。
- 纠正：Snapshottest 在同一唯一入口下先 stop、再以 `NR_V4L2_AUDIT=1` start，确保本轮媒体进程启用有界 V4L2 审计；重跑。
- PASS：开启 `NR_V4L2_AUDIT=1` 后同一新 loader/preload 回归 `PASS=12 FAIL=0`；媒体栈、真实 target、DQBUF、20 秒存活和无 139 全部恢复。
- 补齐负向单测：直接断言 `Ctx::init` 拒绝 109 字节 sockaddr 路径，且 `nv12_to_yuyv` 对短负载返回 false，不读越界。
- PASS：补充直接负向单测后设备 guest 为 `6 passed; 0 failed`；109 字节 UNIX 路径被拒绝，短 NV12 源/小目标均返回 false。
- PASS：最终 Snapshot 回归 `PASS=12 FAIL=0`；证据固化到 `docs/design/evidence/367-snapshot-v4l2-boundary-20261128/`。
- FACT：本轮运行身份为 loader `5a28f920377fe64632e6c83f3973f81c6645931475df56adcdead69e922e36f0`、v4l2 preload `be263cd021597c1601db03b18bd868ce2d762325183760fcfd24ce7b630b2a65`。
- 判定：边界修复与 Snapshot 回归 PASS；旧 `57d253cb...` dist 已包含旧 loader/preload，发布前必须重建新 dist 并重新部署自证。
- 状态：16.379 PASS。

#### 16.380 全量可运行设备单测（进行中，2026-11-28）

- 操作：用 NDK 28.2 + `/tmp/nr-toolchain/root` 构建 `runtime-core`、`nr-tools`、`gpu-env` AArch64 测试，在 Husk App 的 guest rootfs 内执行；同时单独保留 v4l2-shim 的 6 项设备单测证据。
- 判据：构建退出 0；设备测试输出 `test result: ok`；任何 0 tests 或缺失用例必须明确写 NOT VERIFIED，不算覆盖 PASS。
- 状态：执行前记录。
- PASS（执行）：`make test-device` 构建并运行 gpu-env `6/0`、nr-tools `7/0`；runner 退出 0。
- FAIL（覆盖）：runtime-core 仍为 `0 passed`，这个 PASS 只能证明测试二进制可运行，不能证明 canonical path resolver 有单测覆盖。
- 操作：补 `runtime-core` 的 errno 分类、symlink 目标合成、路径规范化、`..` 夹紧和超长路径负向测试；把 v4l2-shim 与 runtime-core 的测试纳入同一 `make test-device`。
- FAIL：合并后的 `make test-device` 中 runtime-core 新增用例为 `5 passed; 1 failed`；v4l2-shim 6/6、gpu-env 6/6、nr-tools 7/7。runner 只显示 tail，尚未定位失败用例。
- 操作：单独执行同一 runtime_core 测试二进制，保留完整失败名和断言；修正测试期望或实现后重跑整个目标。
- FACT：失败断言是测试期望错误：`pop_component("/a", 2)` 返回 0（根路径分量长度），不是 1；实现与 `norm()` 的后续行为一致。
- 纠正：把期望改为 0，保留 `/a/b -> 2`、根夹紧测试，重跑 `make test-device`。

- PASS（最终）：修正测试期望后重新执行 `make test-device`，结果为 gpu-env `6 passed; 0 failed`、nr-tools `7 passed; 0 failed`、runtime-core `6 passed; 0 failed`、v4l2-shim `6 passed; 0 failed`，汇总 `UNIT_TESTS: PASS`。
- 覆盖：runtime-core 用例覆盖 errno、绝对/相对 symlink、超长路径、norm 与 `..` 夹紧；v4l2-shim 用例覆盖正常 NV12、短源、小目标、ioctl 低 32 位和 UNIX 路径上限。
- 修改：`Makefile` 将 `-p v4l2-shim` 纳入 `TEST_PKGS`；`tools/device-tests/run-unit-tests.sh` 识别 `v4l2_shim-*`；`runtime/crates/runtime-core/src/lib.rs` 增加 6 项 resolver 测试。
- 证据路径：源码修改位于上述三个路径；设备执行入口为 `make test-device`；本轮关键现场仍由 `docs/design/evidence/367-snapshot-v4l2-boundary-20261128/` 与设备 guest 单测输出交叉证明。
- 状态：16.380 PASS。

#### 16.381 make check、diff 检查与新 dist 重建（进行中，2026-11-28）

- 操作：按 NDK `28.2.13676358` + `/tmp/nr-toolchain/root` 执行 `git diff --check` 与 `make check`，全绿后执行 `make dist`。
- 判据：格式检查退出 0；make check 不得把 warning 写成零 warning；dist 必须包含本轮 loader `5a28f920...` 与 v4l2 preload `be263cd0...`，逐项核对 tar SHA、大小、entries 与关键文件 SHA；旧 `57d253cb...` 永久作废。
- 状态：执行前记录。

- FAIL：首次 `make check` 退出 2；`cargo fmt`、AArch64 workspace `cargo check`、locale 测试均通过到 rootfs selection 测试，随后报 `distribution-specific executable default in tools/device-tests/28-snapshot-pipewire.sh`。这是门禁正确拦截，不是编译失败。
- 已知 warning：编译仍有多项既存 warning，不写成零 warning。
- 纠正中：检查 rootfs selection 测试规则，移除 Snapshot 测试对固定发行版命令路径的依赖，改为经当前 guest rootfs 能力探测解析入口。

- FAIL：修正 rootfs 默认后第二次 `make check` 通过 cargo fmt/check、locale、rootfs selection、fork safety、149 个 shell 脚本、README、release offline audit，随后被 `audit-runtime-package.sh` 拦截：`tools/device-tests/28-snapshot-pipewire.sh:11` 默认 `PKG=com.linuxhub.myth.husk` 为非正式包名。
- 纠正中：测试脚本默认包名改回正式 `com.linuxhub.husk`；本机 `com.linuxhub.myth.husk` 只在运行时以 `PKG=` 显式覆盖，不污染发布资产。

- PASS：修正非正式包名默认后第三次 `make check` 退出 0，覆盖 cargo fmt、AArch64 workspace check、locale、rootfs selection、fork safety、149 个 shell、README、release offline、runtime package、git refs、version/release notes、session GPU/rootfs、PanVK、runtime boundary/private paths、PipeWire plugin、loader reproducibility（`5a28f920...`）、husk-tools 与 374 个 Markdown 链接。
- 证据：`/tmp/husk-make-check-20261128-rerun2.log`；修正路径 `tools/device-tests/28-snapshot-pipewire.sh`（ROOTFS 默认 `$APP/files/rootfs-full`，PKG 默认 `com.linuxhub.husk`）。
- 已知：编译 warning 仍存在，未判为 warning-free。
- 状态：make check PASS；新 dist 重建进行中。

- PASS（构建）：`make dist` 退出 0；新 tar `dist/husk-arm64.tar.gz` SHA256 `e0cb6dbd09b917728d1665c8b7ebb7eb1ad7b0007637f305a2aa540bbd85be92`，大小 `11,695,324` B，entries `56`。
- FAIL（审计方法）：首版内容审计直接读取 `dist/libld.so` 等 staging 文件，但 make 目标结束时只保留 tar，导致关键 SHA 为空、gzip 统计成空流哈希；required/forbidden 项虽通过，这份审计整体作废。
- 纠正中：解包 tar 到隔离临时目录，对包内 loader、v4l2 preload、PipeWire 插件、PanVK 驱动重新计算 SHA，并保留完整 listing。

- FAIL（关键哈希）：解包复核得到 loader `5a28f920...`、PipeWire 插件 `5baacf01...` 均匹配，但包内 `guest/libhusk_v4l2.so` 为 `ad3ed743fcee9464f6ce7fbe2f502e4a949378d179a7908efe436886bb164305`，与先前部署值 `be263cd021597c1601db03b18bd868ce2d762325183760fcfd24ce7b630b2a65` 不一致；大小同为 `200,600` B。该项断言按失败处理。
- 纠正中：检查 Makefile/preload 构建参数、target 产物与设备文件，连续构建比较 SHA，区分构建不稳定、部署旧产物或源码变化。

- PASS（审计）：解包后 loader `5a28f920...`、v4l2 preload `ad3ed743...`、PipeWire 插件 `5baacf01...`、PanVK driver `5e063247...` 均按包内实体核验；tar 为 `e0cb6dbd...`、`11,695,324` B、56 entries；必需项全部 PRESENT，无 `._*`、无私路径/Ubuntu 路径、无旧实验脚本。
- 根因边界：v4l2 preload 与已部署测试版本仅差 1 字节，在 `.data.rel.ro` 元数据长度位（文件偏移 131233），全部代码/只读数据一致；按等价机器码继续，但保留新包哈希 `ad3ed743...` 为发布实际值。
- 证据：`docs/design/evidence/367-dist-0.0.37-20261128/dist-audit.log` 与 `.sha256`。
- 状态：dist 内容审计 PASS。

### 16.382 将新 dist 升级安装到测试 App 并做运行身份/ICD/Snapshot 回归（进行中，2026-11-28）

- 操作：把新 tar 推入 `com.linuxhub.myth.husk` 的升级安装路径，确认安装器不吞错，安装后从设备读回 VERSION、loader、gpu-env、PanVK、ICD、V4L2、PipeWire 与运行身份。
- 判据：设备 SHA 与 dist 一致；`23-runtime-identity.sh`、`24-icd-canonical.sh`、`28-snapshot-pipewire.sh` 分别 PASS；Snapshot 20 秒无 139，真实 DQBUF；普通取景画面单列。
- 状态：执行前记录。

- PASS（安装）：新 tar 经 26 号脚本 `--upgrade` 安装，安装器到达 `== done ==`；运行身份为 version `0.0.37`、commit `c924d620f6de`、loader `5a28f920...`、libc `16d5ba21...`、gpu-env `ad230fcc...`、PanVK `5e063247...`、ICD `25f6ccd9...`、PipeWire 插件 `5baacf01...`，`RUNTIME_IDENTITY PASS=12 FAIL=0`。
- FAIL（测试器）：26 号脚本打印 `husk-usermgr` 和 `husk-symlinks` 在 app runtime dir 与 stage 不一致，却仍以 0 退出；这两项实际应核对安装后的 guest 落点，当前比较路径与失败传播都不可信。
- 纠正中：定位安装器真实落点，修正比较目标，并让任一 compare FAIL 反映到脚本退出码后重跑。

- PASS（最终升级安装）：修正后以最终 tar `06bce26ae3e6193dacb89121debe533bef80edc04220ec925619d1df8dac9191` 再装；安装器 `== done ==`，20 个运行层文件全部与 stage 一致，`RUNTIME_COMPARE PASS=20 FAIL=0`，`RUNTIME_IDENTITY PASS=12 FAIL=0`。
- PASS（工具副本）：`usr/sbin` 与 `usr/bin` 下 `husk-usermgr`/`husk-su`/`husk-ldconfig` 均已与本包一致，旧的绝对路径绕过风险消除。
- 证据：`docs/design/evidence/367-runtime-upgrade-20261128/26-upgrade-and-identity.log` 与 `.sha256`。
- 状态：升级安装与身份自证 PASS。

- PASS：升级后 `24-icd-canonical.sh` 为 `PASS=15 FAIL=0 SKIP=0`；canonical 驱动 `5e063247...`、zink/ICD 固定、旧清单与旁路驱动消失，Vulkan loader-scan 和 legacy-manifest 两条路径均创建实例并回读 buffer。
- FAIL（测试入口）：首次最终 Snapshot 传入相对 `LD=files/linux/husk/libld.so`，媒体 stop/start 报 `inaccessible or not found`，`RUN_RC=127`；随后 Snapshot 命中的是旧媒体残留，`8 PASS / 4 FAIL` 整轮作废。
- 纠正中：改用 App 绝对 loader/rootfs 路径，先真实 stop/start 清污染，再重跑 28 号。

- PASS（最终 Snapshot）：改用 App 绝对 loader/rootfs 后 `28-snapshot-pipewire.sh` 返回 `PASS=12 FAIL=0`；PipeWire/WirePlumber 真启动，BACK/FRONT V4L2 可见，Snapshot 20 秒存活，主动 TERM=143，无 139，caps skip、numeric target、连续 DQBUF、无 panic 全部命中。
- FAIL（相机可见画面）：截图 `docs/design/evidence/367-snapshot-final-20261128/snapshot.png` 的取景区域仍黑。相机应用发布门禁保持 FAIL，不能把数据链 PASS 写成相机可用。
- 证据：`docs/design/evidence/367-snapshot-final-20261128/28-snapshot-pipewire-rerun.log`、GST/PipeWire/V4L2 日志、`snapshot.png` 与对应 SHA 文件。
- 状态：16.382 的安装/身份/ICD/139/媒体链 PASS；普通环境相机可见画面 FAIL。

#### 16.383 0.0.37 硬件、桌面、输入、GPU 全局与 readback 门禁（进行中，2026-11-28）

- 操作：按 `TODO-0.0.37.md` 对应门禁依次跑 camera/audio/video/sensor、桌面 20 次、输入、GPU 全局 13 项、readback 19 项与 GPU soak；每组先保留原始日志，再汇总 PASS/FAIL/SKIP。
- 判据：不利数字照写；任何 SKIP 不计 PASS；Adreno 无真机必须 UNKNOWN；普通环境相机黑屏继续 FAIL，不隐藏。
- 状态：执行前记录。

- PASS：`make gpu-audit` 退出 0；readback `PASS=19 FAIL=0`、renderer `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，GNOME Shell 与 nested GTK 均为 zink + 硬件 Vulkan maps + `/dev/mali0`，`GPU_GLOBAL_AUDIT PASS=3 FAIL=0 SKIP=0`。
- PASS：`18-gpu-loader-defaults.sh` 为 `PASS=9 FAIL=0 SKIP=0`；无 GPU env 的裸 exec 仍为 Mali zink，`NR_GPU_DEFAULT_OFF=1` 与 `HUSK_GPU=llvmpipe` 都正确回退 llvmpipe。
- PASS：`25-self-exe-exec.sh` 为 `PASS=4 FAIL=0`；`/proc/self/exe` exec 回到 guest bash，选项错误来自程序自身。
- 证据：`docs/design/evidence/367-gpu-audit-final-20261128/`。
- 下一步：4 路 × 300 秒 GPU soak（进行中）。

- PASS：4 路 × 300 秒 × 2400×1080 GPU soak 退出 0，`GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0`；四路均 zink/Mali，帧数 130481/129786/129632/129693，平均 432.1–434.9 fps，峰值 497–504 fps，fd 均 7→7，`/dev/mali0` fd 均 2→2，RSS 各 +452 kB，GL errors=0。
- 证据：`docs/design/evidence/367-gpu-soak-final-20261128/`（汇总日志与四路原始日志）。
- 下一步：A-hw 相机/麦克风/传感器/定位（进行中）。

- PASS：A-hw 为 `PASS=15 FAIL=0`：CAMERA/RECORD_AUDIO/ACCESS_COARSE_LOCATION 均 granted 且非 USER_FIXED，Camera2 真出 3 帧（1382400 B/帧），5 类 IIO 传感器三路径实时可读，麦克风返回 196608 B（peak=440），网络定位 TPV，应用无崩溃。
- PASS：`22-audio-audit` 为 `PASS=4 FAIL=0`：PulseAudio helper 11 模块、audio-out.sock、AAudio 96000 帧、标准 paplay→PulseAudio→relay→AAudio 全通。
- PASS：相机 fixture 为 raw + GStreamer、640×480 + 1280×720 全 PASS；V4L2/DQBUF 数据有 min/max/mean，不是空帧。
- 相机黑屏归因推进：A-hw 最新 Camera2 NV12 三帧 Y `mean=0.334/0.405/0.410`、`max=2/3`、`p90=1`，全部低于 10；当下物理场景本身就是近纯黑，因此 Snapshot 黑屏不能归为 X11/PipeWire 呈现坏掉。可见画面需等补光/手电复测，当前标环境阻塞而非 PASS。
- 证据：`docs/design/evidence/367-a-hw-final-20261128/`、`docs/design/evidence/367-audio-final-20261128/`、`docs/design/evidence/367-camera-fixture-final-20261128/`。
- 设备电量只剩一格：暂停 600 秒视频、20 次桌面循环和交互长测，改做本地资产/安装包/文档审计。
