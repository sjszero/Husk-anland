# Evidence 87 — VA 取帧吞吐上限，以及"第二个会话"为什么会静默零帧

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / 4.19.191 / 4 KiB / arm64
域：`untrusted_app`（`com.linuxhub.husk`），解码器 `c2.mtk.avc.decoder`

## 一、吞吐（FACT，均为 1920x1088 语义、1080p30 码流）

| 场景 | 帧数 | 墙钟 | 折算 | 说明 |
|---|---|---|---|---|
| 只提交不取帧（`vaapih264dec ! fakesink`） | 600 | 19 s | **≈33 fps** | 解码 + app 侧 I420 重排，不含取帧往返 |
| 取帧（`vaapih264dec ! videoconvert ! video/x-raw,format=I420 ! fakesink`） | 600 | 35 s | **≈18 fps** | 同上 + 每帧一次 guest↔app 往返 |
| 同上，改短轮询前（120 s 连续） | 1500 | 120 s | ≈12.5 fps | `drain_frames` 一次阻塞 50 ms |
| 同上，改短轮询后（120 s 连续） | 2100 | 120 s | ≈17.5 fps | 改成 2 ms 分片轮询 |

诊断（INFERENCE，基于上面两组）：瓶颈不是解码吞吐，而是**一次进一次出的取帧时延**。
提交侧已经能到 33 fps（每帧 ≈28 ms，含 3.1 MB 的 CPU 重排），取帧又叠加一次往返，
变成 ≈55 ms/帧。要把 1080p30 做满，需要帧级流水（码流里至少 2 个 AU 在飞），
也就是让客户端在等第 k 张图时先把第 k+1 个 AU 提交进去；`gst-vaapi` 的
`vaBeginPicture → vaRenderPicture → vaEndPicture → vaGetImage` 顺序是一帧一结的，
驱动侧无法自己造出流水。两条可行路线（未实现）：
1. app 侧保持 6-slot 窗口的既有 AHB 路径（evidence 80 已实测 600 秒 18000 帧，
   30 fps 稳定）——这是**产品播放路径**；
2. VA 系统层做输出延迟（缓冲一帧再交付），用 +1 帧延迟换吞吐，需要
   `vaCreateContext` 的 `num_render_targets`/延迟语义配合，未验证。

**所以：VA 系统层现在能正确解码，但吞吐只有 ≈18 fps，不能宣称"1080p30 流畅"。**
硬解 vs 软解 CPU 对比也仍未做。

## 二、第二个视频会话会静默零帧（FACT，已修）

复现：一个会话正在跑（长片），另一个进程同时 `OPEN`。

修复前：两个连接都拿到 `OPEN_READY`，第二个会话 `FRAME` 永远拿不到图，
`HUSK_VA_FRAME_NONE` 刷屏，客户端看到的是"健康 socket + 零帧"。
更糟的是它会把 presenter / event bridge 这两份进程级单例搅乱，
连第一个会话也可能被拖死；只有等 30 秒读超时把死会话回收才恢复
（这正是之前"杀掉 guest 后要等两分钟才恢复"的原因）。

修复后：

```text
HUSK_VA_BRIDGE_OPEN_FAILED context=1 size=1920x1088 video bridge OPEN failed: OPEN_ERROR another video session is active
```

- 一个进程只允许一个解码会话（`SESSION_ACTIVE`），连接被拒绝时给明确错误，
  不再伪装成功。
- accept 循环改成每个连接一个线程，这样第二个客户端一定能拿到上面那句错误，
  而不是在 backlog 里等着。

**更正（同日实测）**：`connect` 拿到 `EAGAIN (os error 11)` **不能**当成"backlog 满"的证据。
实测到了另一次：设备 `mWakefulness=Asleep`（屏幕关了 → app 被冻结 → 没人 accept），
连续 200 秒每次 connect 都是 `Resource temporarily unavailable`。所以看到 EAGAIN 先查
`dumpsys power | grep mWakefulness` 和 `mCurrentFocus`；这也是为什么测试前必须
`input keyevent KEYCODE_WAKEUP; svc power stayon usb`。

## 二点五、想让 gst-vaapi 流水的尝试失败（FACT）

在 `vaapih264dec` 和 `videoconvert` 之间插 `queue max-size-buffers=8`，希望解码器先攒几帧：

```text
200 秒超时，HUSK_VA_DELIVERED 一行都没有，RC=124
```

surface 池被 queue 吃光 → 解码器拿不到新 surface → 死锁。**gst-vaapi 是设计上一帧一结的**，
深度只能由客户端决定。对照 `lfdevs/termux-va`（见 `docs/design/prior-art-video.md`）：
他们桥的流水线深度是 6，流畅的客户端是 **FFmpeg 系**；Chromium/Firefox 硬解在
非骁龙 SoC（含 MTK）上是已知不可用。

## 三、两个踩过的坑（写给下一个我）

1. **`HUSK_VA_TRACE=0` 是开着的。** 代码里是 `var_os(...).is_some()`，设成 `0`
   同样开 trace。用 trace 判定"有没有帧"会得到反向结论：`HUSK_VA_FRAME` 是
   trace 门控的，**关掉 trace 就一条都看不到，但不代表没解出帧**。
   现在加了无条件的 `HUSK_VA_DELIVERED frames=N`（每 300 帧一行）和
   `HUSK_VA_FRAME_NONE`（前 5 次），就是用来消除这个歧义。
2. **只改 `jniLibs/*.so` 时 `./gradlew :app:assembleDebug` 不会重新打包**，
   APK 里的 `.so` 还是旧的（sha 对不上，实测过）。必须 `--rerun-tasks`，
   装完后用 `strings .../libhuskapp.so | grep <新字符串>` 复核。

## 四、用硬证据确认"真的出帧"（不依赖 trace）

`vaapih264dec ! videoconvert ! video/x-raw,format=I420 ! filesink location=...`
把整段 10 秒片写出：

```text
-rw-rw-rw- 1 u0_a302 u0_a302 466560000 frames.i420
466560000 = 150 × 1920 × 1080 × 3/2   # 150 帧紧排 I420，一字节不差
```

注意写出的 height 是 **1080**，不是 1088：合成 SPS 声明的是 1088（无 crop），
但 `h264parse` 自己知道真实可见高度，下游按 1080 取，多出来的 8 行没有泄漏给用户。

## 五、未解决的 bug（FACT，留给下一轮）

`gst-launch-1.0 ... ! filesink` 写完整个文件（466,560,000 字节，一字节不差）之后
**进程不退**，`RC=0` 也不返回：

```text
$ ps -A -o PID,NAME | grep libld
32069 libld.so            # 文件早就写完了，进程还活着
$ for t in /proc/32069/task/*; do cat $t/wchan; done
do_sys_poll / do_sys_poll / futex_wait_queue_me / 0 / do_sys_poll
```

后果：这条连接一直占着视频会话，后面的客户端会被 `OPEN_REFUSED` 挡掉——这很可能就是
之前"杀掉 guest 之后要等很久才恢复"的真正来源。当前靠租约（静默 45 秒可接管）兜住，
但**根因没修**：要么 `vaTerminate`/`vaDestroyContext` 没有把 `VideoClient` 的 socket 关掉
（`DriverContext` 存在静态表里，可能永远不会被 drop），要么 gst-vaapi 在 teardown 时
等一个我们没实现的 display 事件。下一轮第一步：在 `vaTerminate` 里显式
`*video = None` 并记一行日志，然后重跑上面这条命令，看进程是否正常退出。
