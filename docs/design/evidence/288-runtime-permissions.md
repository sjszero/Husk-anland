# 288 · 第 4 轮：普通用户装上去就能用（运行时权限 + A-hw）

- 日期：2026-11-26
- 设备：RMX3357 / MT6893 / Android 13，`UKHMMB8PYTGECMNV`
- 包：`com.linuxhub.husk`，0.0.36，`android:hasCode="false"` 未改
- 起手状态：这台机器之前一直是 `adb install -g` 装的，所以「相机能不能用」从来没有
  真正被验证过——权限是 `-g` 直接给的，用户从来没看见过对话框。

---

## 1 · 为什么没有 `onRequestPermissionsResult`

NDK 给原生代码的是一张固定的 `ANativeActivityCallbacks`，里面**没有**权限结果回调——
那个回调只存在于 Java 的 `Activity` 子类里，而 `hasCode="false"` 排除了它。所以签名是：
把 `requestPermissions` 记成 pending，下一次 `onResume` 用 `checkSelfPermission` 把答案读回来。

这个信号比回调弱（分不清「拒绝」和「拒绝且不再询问」），所以凡是要区分这两者的事情
——打开设置页——都由「同一个 pending 第几次回来还是 denied」驱动，而不是猜对话框按了什么。

`Activity` 实例只从 `ANativeActivity.clazz` 取，不反射 `ActivityThread.mActivities`。

---

## 2 · 三个真 bug（都是本轮量出来的，不是想出来的）

### BUG-1 · 只要权限没给，进程一启动就 SIGABRT（FATAL）

```
F libc : Fatal signal 6 (SIGABRT) ... in tid 31923 (m.linuxhub.husk)
F DEBUG: Abort message: 'JNI DETECTED ERROR IN APPLICATION: illegal class name 'java/lang/String;'
F DEBUG:     in call to FindClass
F DEBUG:     from void android.app.NativeActivity.onResumeNative(long)'
```

`NewObjectArray` 收的是**类名**不是描述符。传 `"java/lang/String;"` 让 `FindClass` 直接
abort 整个进程；正解是 `"java/lang/String"`。旧代码里这行已经存在了，只是之前一直是
`-g` 装的、`to_ask` 永远为空，所以从没执行到。**这就是「实测装一次就知道」的那类 bug。**

### BUG-2 · 对话框抢焦点的那次 resume 把两次机会一起用掉了

修掉 BUG-1 之后第一版行为（FACT，logcat 原文）：

```
19:17:16.402 STATE|camera=denied microphone=denied|asks=0|denials=0
19:17:16.413 REQUEST|CAMERA,RECORD_AUDIO
19:17:16.556 STATE|...|asks=1|denials=0
19:17:16.556 RESULT|CAMERA=denied (denial #1)      <- 48 ms 后，"答案"就回来了
19:17:16.594 REQUEST|CAMERA,RECORD_AUDIO          <- 同一趟里又弹一次
19:17:16.659 RESULT|CAMERA=denied (denial #2)
19:17:16.681 SETTINGS|opened app details          <- 用户一下都没点
```

`requestPermissions` 之后**48 ms** 就来了一次 resume，那时对话框刚抢到焦点、权限值还是
旧值（denied），我们把它当成了答案：两轮对话、两次 toast、两次设置页，全在 300 ms 内完成。
修法两条：
- 距离请求不足 `SETTLE_MS = 800 ms` 的 denied 不算答案（人类点不了那么快，弹窗抖动会）；
- 还有 pending 未答时绝不再发第二次请求。

修后（FACT）：

```
19:20:19.605 STATE|asks=0 → 19:20:19.614 REQUEST
19:20:19.728 STATE|asks=1|denials=0     <- 只记状态，不再弹、不再计数
```

### BUG-3 · 两个权限各开一次设置页

`open_app_settings` 在循环里，于是同一个页面被 `startActivity` 了两次（日志里连续两条
`permissions|SETTINGS|`）。改成每趟最多开一次。

---

## 3 · 四类验收（FACT，逐步实测）

| 类别 | 做法 | 结果 |
|---|---|---|
| 首次申请 | 普通 `adb install`（**无 `-g`**），权限为 denied | 首启弹系统框「“Husk”需要使用以下权限：摄像头 / 麦克风」（截图与 `permissions|REQUEST|CAMERA,RECORD_AUDIO` 同一时刻）|
| 拒绝 | 对话框里两个都选「拒绝」→「确定」 | `RESULT|CAMERA=denied (denial #1)` + `TOAST|Husk 需要相机权限，否则相机在 Linux 里是黑的。下次进来还会再问一次。`，**不开设置页** |
| 第二次拒绝 | 再次启动 → 第二次对话框 → 拒绝 | `denial #2` + `TOAST|已经问过两次…现在打开本应用的权限页面` + `SETTINGS|opened app details for com.linuxhub.husk`（一次） |
| 重新授权 | 在该设置页把摄像头、麦克风改成「使用时允许」 | 下次启动 `STATE|camera=granted microphone=granted` → `RESULT|…=granted` → `OK|all hardware permissions granted`，`denials` 归零，不再弹框 |

「升级保留」：状态文件在 `internalDataPath`（`files/.husk-permission-state-v1`），本轮所有
测试都是 `adb install -r` 覆盖安装，记录和「已经问过」都没有丢，`asks` 计数继续沿用——
0.0.35 的旧标记 `.husk-runtime-permissions-v1` 也能读，迁移时按「已问过两次」处理，
所以升级不会重新弹框。

---

## 4 · 权限给上之后，硬件真的出货（FACT）

`camera-client` 打 App 的 Camera2 socket，三帧 1280×720 NV12：

```
camera-client|OPEN|OK w=1280 h=720 fourcc=NV12 bytes=1382400 id=0
camera-client|FRAME|seq=1|...|1280x720|bytes=1382400|min=0|max=128|mean=42.85
camera-client|FRAME|seq=2|...|mean=42.86
camera-client|FRAME|seq=3|...|mean=42.86
camera-client|PASS|frames=3|elapsed_ms=398|bye=BYE|seqs=1,2,3
```

`min=0 max=128 mean≈42.8` 说明不是全黑缓冲。这是 `openCamera` 通过的直接证据——
没有 CAMERA 权限时它以 `SecurityException` 失败。

新增门禁组 `tools/device-tests/17-gate-a-hw.sh`（**A-core 的 36 项不动**）：**9 PASS / 0 FAIL**。

它同时把两件事写清楚，避免以后被当成证据用：
- `camera|list|count=2` 与 `camera|v4l2|tree|devices=2` 在**权限被拒时也会出现**
  （列相机、发布设备节点不需要权限），所以它们只打 `NOTE`，不计 PASS；
- `USER_FIXED` 标志位必须不存在——它是「拒绝且不再询问」，一旦有这个标志，对话框永远
  不会再出现，只能走设置页。

---

## 5 · 未做 / 未验证（照写）

| 项 | 状态 |
|---|---|
| 麦克风**采集**端到端（AAudio capture → guest） | 只验到 `caps|audio-in|present=yes`（枚举）。真实录音链路本轮**没有**在「用户授权」路径下重跑，上一轮的证据是 `install -g` 给的权限 |
| `USER_FIXED`（拒绝且不再询问）下点开设置页的完整人工路径 | 走通过一次（设置页确实被拉起），但那次两个权限**同时**是 USER_FIXED，无法分辨系统是「用户第二次拒绝」还是「不再询问」触发的 |
| 权限运行时的处理放到独立线程 | 现在就在 `onResume`（主线程）里做，Toast/Intent 都合法，但 `checkSelfPermission` 每趟 resume 都调两次，没有缓存 |
| App 的桥接 socket 路径 | App 固定用 `files/rootfs-full/run/husk/*.sock`（`ahb.rs`/`audio.rs`），而我们手上的测试树是 `files/linux2/distributions/ubuntu-24.04/rootfs`。也就是说：**从这些测试树里起的 guest 够不到 App 的 bridge**，本轮的相机验证是直接打 socket 做的。这是测试树与 App 自带安装器的路径不一致，不是运行时缺陷，但「从 Linux 树里直接开 /dev/video0」本轮**没做成**，UNVERIFIED |
| 跨设备 | 只有这一台（Android 13 / 4.19 / 4KiB / Mali-G77）。Android 14+ 的「只允许本次」等新选项、16/64 KiB 页全部 UNVERIFIED |
