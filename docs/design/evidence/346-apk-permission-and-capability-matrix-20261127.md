# 346 · APK 权限链路与能力矩阵复验（H33）

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：重新执行 `make apk` 成功；APK 为
  `versionCode=36`、`versionName=0.0.36`，SHA256
  `e82587852fcdbe61765f0e567ec5a0183d7b6265510c3c1499ad62787648934f`。
- **FACT**：`adb install -r` 未带 `-g`，覆盖安装后 CAMERA / RECORD_AUDIO /
  ACCESS_COARSE_LOCATION 仍是 `granted=true`，升级没有丢授权。
- **FACT**：在系统设置里撤销 CAMERA / RECORD_AUDIO 后冷启动 App，系统弹出
  “Husk”需要使用摄像头、麦克风的权限框；不是只写日志。
- **FACT**：连续拒绝两次后，系统权限设置页被打开，用户可以重新授权。
- **FACT**：重新授权后三项权限恢复 granted，A-hw `PASS=15 FAIL=0`。
- **FACT**：能力矩阵已写成五类 IIO 三路径，且不再出现
  `iio-sensor-proxy D-Bus remains UNVERIFIED`。

## 1 · APK 构建与覆盖安装

```text
make apk: BUILD SUCCESSFUL
package: versionCode='36' versionName='0.0.36'
sha256 e82587852fcdbe61765f0e567ec5a0183d7b6265510c3c1499ad62787648934f
adb install -r: Success（没有 -g）
```

覆盖安装后的权限：

```text
android.permission.CAMERA: granted=true
android.permission.RECORD_AUDIO: granted=true
android.permission.ACCESS_COARSE_LOCATION: granted=true
```

## 2 · 首次权限申请与拒绝路径

撤销两项权限后：

```sh
adb shell am force-stop com.linuxhub.husk
adb shell am start -W -n com.linuxhub.husk/android.app.NativeActivity
```

实际顶层 Activity：

```text
com.android.permissioncontroller/.permission.ui.GrantPermissionsActivity
```

UI 文本：

```text
“Husk”需要使用以下权限
摄像头   使用时允许
麦克风   使用时允许
确定
取消
```

第一次点取消后再次冷启动，权限框再次出现。第二次点取消后，顶层变为：

```text
com.oplus.securitypermission/com.oplusos.securitypermission.permission.PermissionGroupsActivity
```

同一界面可见：

```text
摄像头   不允许
麦克风   不允许
```

这证明拒绝后的用户可见引导存在，且不是无响应。

## 3 · 恢复授权与硬件回归

在系统权限页把摄像头、麦克风重新改为“使用时允许”，重启 App 后：

```text
android.permission.CAMERA: granted=true
android.permission.RECORD_AUDIO: granted=true
```

A-hw：

```text
PASS=15 FAIL=0

PASS captured 3 frame(s) through Camera2
PASS app opened a Camera2 session
PASS five default sensors readable through three IIO views
PASS microphone returned 200704 PCM bytes (peak=1449)
PASS gpsd delivered a last-known fix
PASS no crash records for the app process
```

## 4 · 能力矩阵

宿主能力文件 `files/husk-capabilities.txt` 包含：

```text
caps|sensors|present=yes|husk=degraded|...
  five default sensor classes are readable through /sys/class, /sys/bus and
  /sys/subsystem IIO views, with continuous-stream checks in A-hw;
  iio-sensor-proxy D-Bus claims verified (evidence 302/322/344)
```

旧文案：

```text
iio-sensor-proxy D-Bus remains UNVERIFIED
```

已从新 APK 的实际输出中消失。

## 5 · 工程门禁

```text
make check: PASS
shell syntax: 110 script(s) checked
runtime package audit: PASS
release offline audit: PASS (version=0.0.36)
docs links: 372 markdown files, 0 broken
```

**UNVERIFIED**：本轮使用现有 App 数据目录做覆盖安装和权限撤销/恢复，没有执行
`pm clear`；因此不会破坏设备上已有的 rootfs。真正的“全新用户首次安装”仍需在
一台干净设备或新建 Android 用户上复跑。

