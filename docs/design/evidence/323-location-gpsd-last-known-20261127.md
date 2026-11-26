# 323 · Android last-known 定位到 gpsd Unix socket

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB 页
目标环境：`com.linuxhub.husk`
涉及文件：

- `app-rs/src/location.rs`
- `app-rs/src/lib.rs`
- `app-rs/src/permissions.rs`
- `app-rs/src/caps.rs`
- `app/app/src/main/AndroidManifest.xml`
- `runtime/crates/location-probe/src/main.rs`
- `tools/guest-config/build-guest-tools.sh`
- `tools/device-tests/17-gate-a-hw.sh`

## 1. 修前负向结果（FACT）

基线 `51a2446` 没有位置桥接实现，guest 侧也没有 `/run/husk/gps.sock`。定位只被笼统归类为
“需要 Java 回调”，因此连 Android 已经提供的 last-known 点也没有标准接口可用。

```text
$ git ls-tree -r --name-only 51a2446 | grep -E 'location|gpsd'
<no output>

$ GPSD_SOCKET=/tmp/husk-no-such-gps.sock \
    runtime/target/aarch64-unknown-linux-gnu/release/location-probe
location-probe|FAIL|connect=/tmp/husk-no-such-gps.sock: [Errno 2] No such file or directory
```

前一条是源码事实：基线没有位置桥接实现，也没有 socket 路径。后一条是探针在无 socket 时的
实际行为。两条合起来说明修前缺的是接口，不是权限已经给了却取不到数据。

## 2. 实现范围

保持 `android:hasCode="false"`，不新增自定义 Java 类。App 只调用平台 Activity 已有的方法：

```text
ACCESS_COARSE_LOCATION
  -> LocationManager.getLastKnownLocation(network/fused/gps)
  -> gpsd JSON
  -> /run/husk/gps.sock
```

支持的最小 gpsd 协议：

```text
?VERSION;  -> VERSION
?DEVICES;  -> DEVICES
?WATCH=... -> WATCH + 当前 TPV
?POLL;     -> 当前 TPV
```

`/run/gpsd.sock` 在未被真实 daemon 占用时指向 `husk/gps.sock`，所以普通 gpsd 客户端不需要
额外环境变量。若该路径已经是真实 socket 或普通文件，Husk 不覆盖。

socket 必须使用 Unix domain socket，不能监听无认证 TCP。后者会让同一台手机上的其他应用
绕过 Android 的逐应用位置授权，直接读取 Husk 拿到的位置。

## 3. 授权正向结果（FACT）

使用系统权限页授予位置信息后：

```text
$ adb shell dumpsys package com.linuxhub.husk | grep -m1 ACCESS_COARSE_LOCATION
android.permission.ACCESS_COARSE_LOCATION: granted=true, flags=[ USER_SET ... ]

$ adb shell run-as com.linuxhub.husk cat files/rootfs-full/run/husk/location/status
permission=granted
backend=android-location
interface=gpsd-unix:///run/husk/gps.sock
provider=network
updated_ms=<redacted timestamp>
mock=0
```

设备上的文件类型和权限：

```text
srwx------ ... files/rootfs-full/run/husk/gps.sock
lrwxrwxrwx ... files/rootfs-full/run/gpsd.sock -> husk/gps.sock
-rw------- ... files/rootfs-full/run/husk/location/last-known.json
```

guest 探针只打印脱敏结果：

```text
location-probe|PASS|coordinates=redacted|eph=2000.0|provider=network|mode=2
```

探针内部会检查 latitude/longitude 为有限值且位于合法范围；公开日志不输出坐标。

## 4. 撤权负向结果（FACT）

通过 OEM 系统权限页把位置信息改为“不允许”后，Android 运行时权限变为：

```text
android.permission.ACCESS_COARSE_LOCATION: granted=false, flags=[ USER_SET ... ]
```

App 轮询到拒绝后清除内存中的 FIX，并写入：

```text
permission=denied
backend=android-location
interface=gpsd-unix:///run/husk/gps.sock
fix=none
last-known-absent
```

这里同时检查文件本身：旧的 `last-known.json` 已删除，不是只把内存状态改成 `fix=none`。
同一时刻 guest 探针结果为：

```text
location-probe|FAIL|no TPV fix
```

这验证了两件事：撤权后不能继续返回旧坐标；gpsd socket 仍然存在但只返回 `TPV mode=1`，
客户端能明确区分“没有 fix”和“协议不可用”。

## 5. A-hw 回归（FACT）

重新授权后完整运行：

```sh
DEV=UKHMMB8PYTGECMNV sh tools/device-tests/17-gate-a-hw.sh
```

结果：

```text
PASS  ACCESS_COARSE_LOCATION granted=true
PASS  ACCESS_COARSE_LOCATION is not USER_FIXED
PASS  app reports all hardware permissions granted
PASS  gpsd delivered a last-known fix
       (coordinates=redacted|eph=2000.0|provider=network|mode=2)
PASS  no crash records for the app process
PASS=14 FAIL=0
```

同轮还覆盖了 Camera2 三帧、真实 PCM 录音和硬件能力矩阵，说明新增定位权限没有破坏原有的
相机/麦克风启动路径。

Linux 用户态回归也重跑当前活动树：

```text
$ DEV=UKHMMB8PYTGECMNV ROOTFS=files/rootfs-full \
    LD=files/linux/libld.so EXPECT_PKGS_MIN=700 \
    sh tools/device-tests/14-gate-a-acceptance.sh
PASS=34 FAIL=0 SKIP=1
```

唯一 SKIP 是离线树没有 apt package index，不计作 PASS。

## 6. 静态与脚本门禁

```text
cargo test --manifest-path runtime/Cargo.toml -p location-probe PASS
cargo check --manifest-path app-rs/Cargo.toml \
  --target aarch64-linux-android --lib       PASS
sh -n tools/device-tests/17-gate-a-hw.sh      PASS
git diff --check                              PASS
make check                                    PASS
make docs                                     PASS（350 个 Markdown，0 断链）
make libc-verify                              PASS（与锁定 stock 重建逐字节一致）
```

## 7. 能力边界

- **FACT**：当前位置接口只消费 Android 已有的 last-known 点。
- **FACT**：`network` provider 在参考机返回了有效 fix；`fused` / `gps` 作为后备 provider 查询。
- **FACT**：撤权后缓存 FIX 被清空，gpsd 返回 mode=1。
- **UNVERIFIED**：持续定位、位置监听回调、后台定位、GNSS 原始测量、NMEA 原始语句。
- **UNVERIFIED**：其他 Android 版本和 OEM 的权限页是否同样允许 coarse-only last-known 查询。

本证据不把 last-known 子集写成完整 GNSS 支持，也不把 gpsd JSON facade 写成直连 GPS 硬件。
