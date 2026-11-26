# 327 · 第 4 轮运行时权限真机复验

日期：2026-11-27（CST）
基线：`ff2c583`（修复前），修复后在当前工作树重新构建
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页
版本：`0.0.36`（未发版）
包：`com.linuxhub.husk`，`android:hasCode=false`

## 0 · 结论（FACT）

当前 APK 的普通用户路径已重新跑通：

- 不带 `adb install -g` 启动会弹出系统权限框；
- 第一次拒绝会写 `denial #1` 并显示中文 Toast，不打开设置页；
- 第二次拒绝会写 `denial #2`、显示中文 Toast，并只打开一次应用设置页；
- 在设置页重新授权后，下一次 `onResume` 报告三项全部 granted；
- 修复后授权状态文件归零为 `asks=0 denials=0 nudged=0 last=granted`；
- A-hw 真机结果为 `PASS=14 FAIL=0`，Camera2、麦克风和 last-known 定位都有实际数据。

本轮只证明参考机；其他 Android 版本、ROM 权限界面和 16/64 KiB 页仍为 UNVERIFIED。

## 1 · 权限重置方法（FACT）

这台 ROM 的 ADB shell 被厂商策略限制，直接 `pm revoke` / `pm clear-permission-flags`
会返回：

```text
SecurityException: revokeRuntimePermission: Neither user 2000 nor current process has
android.permission.REVOKE_RUNTIME_PERMISSIONS.
```

因此没有清 App 数据、没有动 rootfs，而是通过系统设置页把摄像头、麦克风和定位
逐项改为“不允许”，再回到 App 普通启动。权限状态文件先备份到主机，测试后由
App 的授权对账逻辑覆盖。

## 2 · 首次申请和两次拒绝（FACT）

首启 UI 原文：

```text
“Husk”需要使用以下权限
摄像头 / 位置信息 / 麦克风
确定 / 取消
```

第一次点“取消”后，logcat：

```text
permissions|STATE|...|asks=camera:1,microphone:1,location:1|denials=camera:0,...
permissions|RESULT|android.permission.CAMERA=denied (denial #1)
permissions|TOAST|Husk 需要相机权限，否则相机在 Linux 里是黑的。下次进来还会再问一次。
permissions|RESULT|android.permission.RECORD_AUDIO=denied (denial #1)
permissions|TOAST|Husk 需要麦克风权限，否则麦克风在 Linux 里是黑的。下次进来还会再问一次。
permissions|RESULT|android.permission.ACCESS_COARSE_LOCATION=denied (denial #1)
permissions|TOAST|Husk 需要定位权限，否则gpsd 没有可用坐标。下次进来还会再问一次。
permissions|REQUEST|android.permission.CAMERA,android.permission.RECORD_AUDIO,android.permission.ACCESS_COARSE_LOCATION
```

第二次点“取消”后：

```text
permissions|RESULT|android.permission.CAMERA=denied (denial #2)
permissions|TOAST|Husk 已经问过两次相机权限了。现在打开本应用的权限页面，把「相机」改成允许。
permissions|SETTINGS|opened app details for com.linuxhub.husk
permissions|RESULT|android.permission.RECORD_AUDIO=denied (denial #2)
permissions|RESULT|android.permission.ACCESS_COARSE_LOCATION=denied (denial #2)
```

设置页只打开一次；两个权限没有各自叠加同一个 Activity。

## 3 · 新发现并修复的 P20（FACT）

在系统设置页把三项都改回“使用时允许”后，旧 APK 的生效日志是：

```text
permissions|STATE|camera=granted microphone=granted location=granted|asks=camera:2,microphone:2,location:2|denials=camera:2,microphone:2,location:2
permissions|OK|all hardware permissions granted
```

状态文件仍是：

```text
asks=2  denials=2  pending=0  nudged=1
```

根因是旧逻辑只在 `pending=true` 的处理分支里清零；设置页返回时 `pending=false`，
所以授权被识别了，但拒绝历史没有清掉。下一次再撤权时 `asks=2` 会让 App 不再弹框。

修复：`app-rs/src/permissions.rs` 的 `Rec::mark_granted()` 把 granted 当作权威状态，
无条件清零 `asks / denials / pending / nudged`，并写入 `last=granted`。

重新 `make apk`、`adb install -r`（不带 `-g`）后，同一份旧状态文件被新进程对账为：

```text
android.permission.ACCESS_COARSE_LOCATION	asks=0	denials=0	pending=0	last=granted	nudged=0
android.permission.CAMERA	asks=0	denials=0	pending=0	last=granted	nudged=0
android.permission.RECORD_AUDIO	asks=0	denials=0	pending=0	last=granted	nudged=0
```

## 4 · A-hw 正向回归（FACT）

```text
PASS  CAMERA granted=true
PASS  RECORD_AUDIO granted=true
PASS  ACCESS_COARSE_LOCATION granted=true
PASS  app reports all hardware permissions granted
PASS  captured 3 frame(s) through Camera2 (1382400 bytes each)
PASS  app opened a Camera2 session
PASS  capability matrix readable
PASS  audio capture device present
PASS  microphone returned 200704 PCM bytes (peak=246)
PASS  gpsd delivered a last-known fix (coordinates=redacted|eph=2000|provider=network|mode=2)
PASS  no crash records for the app process
PASS=14 FAIL=0
```

麦克风原始输出：

```text
CAPTURE_OK frames_read=50176 silent_chunks=5 peak=246
capture-probe|PASS|chunks=49|frames=50176|bytes=200704|silent_chunks=5|peak=246|elapsed_ms=1085
```

## 5 · 回归（FACT）

```text
make apk                                      PASS
adb install -r（无 -g）                        PASS
17-gate-a-hw.sh                               PASS=14 FAIL=0
权限弹框 -> 第一次拒绝 -> 第二次拒绝 -> 设置页  PASS
设置页重新授权 -> 状态清零                      PASS
```

## 6 · 边界（UNVERIFIED / FACT）

- `FACT`：A-hw 的授权结果来自系统 UI 的真实点击路径，不是 `adb install -g`。
- `UNVERIFIED`：其他 ROM 的权限设置页层级、Android 14+ 的单次授权选项和
  `USER_FIXED` 特殊状态仍未在第二台设备上复验。
- `UNVERIFIED`：Android 15/16、16 KiB/64 KiB 页、Adreno 设备需要单独跑同一组门禁。
- `FACT`：Path Shim 不是恶意代码安全沙箱；权限通过只说明 App 能访问硬件，不改变
  raw syscall / 静态 ELF 的边界。
