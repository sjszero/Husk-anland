# 322 · IIO sysfs 布局修正与 iio-sensor-proxy D-Bus 验证

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB 页
目标环境：`com.linuxhub.husk`
涉及文件：

- `app-rs/src/sensors.rs`
- `runtime/crates/path-shim/src/lib.rs`
- `tools/guest/husk-system-services.sh`
- `tools/guest/husk-session.sh`
- `tools/guest/husk-desktop.sh`
- `tools/guest-config/linuxhub-system-bus.conf`
- `tools/guest-config/guest-accounts.txt`
- `tools/guest-config/net.hadess.SensorProxy.policy`
- `tools/guest-config/60-husk-sensor-proxy.rules`

## 1. 修前负向结果（FACT）

上一版把 `/sys/class/iio` 和 `/sys/bus/iio` 整棵指向 `sys/subsystem/iio`。因此
`/sys/class/iio/iio:deviceN` 实际落成
`sys/subsystem/iio/iio:deviceN`，缺少中间的 `devices/`：

```text
cat /sys/subsystem/iio/devices/iio:device0/name
accel_3d

cat /sys/class/iio/iio:device0/name
ENOENT

udevadm info -q property -p /sys/class/iio/iio:device0
Unknown device "/sys/class/iio/iio:device0": No such device
```

这不是“NDK 没有数据”，而是标准 sysfs 目录形状错误；NDK 桥本身仍正常。

## 2. 修正后的布局

当前 app 侧创建一棵真实目录树，并把两个视图按 Linux sysfs 形状逐设备链接：

```text
/sys/subsystem/iio/devices/iio:deviceN            # 真实目录和属性
/sys/bus/iio/devices/iio:deviceN
    -> ../../../subsystem/iio/devices/iio:deviceN
/sys/class/iio/iio:deviceN
    -> ../../subsystem/iio/devices/iio:deviceN
```

`path-shim` 将 `/sys/{class,bus,subsystem}` 父级和 IIO 子路径映射到同一个 guest
rootfs 的物理路径，不再把 `class`/`bus` 的父目录整体替换成符号链接。相对
`openat()`、`fstatat()`、`faccessat()`、`readlinkat()` 和 `statfs()` 也走同一映射。

## 3. 真机正向结果（FACT）

在已安装 APK、rootfs 为
`files/release-tree-libc812-20261127/root`、loader 为本次构建：

```text
loadersha256=921a816a93b4e7bfc11024eee16671170eeff294187ae362499c76e274ad5b9d

cat /sys/class/iio/iio:device0/name
accel_3d

cat /sys/class/iio/iio:device0/in_accel_x_raw
-104

cat /sys/bus/iio/devices/iio:device0/name
accel_3d

cat /sys/subsystem/iio/devices/iio:device0/name
accel_3d
```

`udevadm` 已能识别该设备：

```text
DEVPATH=/subsystem/iio/devices/iio:device0
DEVNAME=/dev/iio:device0
DEVTYPE=iio_device
IIO_SENSOR_PROXY_TYPE=iio-poll-accel
MAJOR=241
MINOR=0
SUBSYSTEM=iio
```

相对 `openat` 和文件系统身份也通过：

```text
test -r /sys/class/iio/iio:device0/name
access=ok

python statfs("/sys/class/iio")
rc=0 type=0x62656572

python os.open("/sys/class/iio") + openat("iio:device0/name", dir_fd=fd)
relative=accel_3d
```

说明：核心 GNU `stat -f` 自己仍会对目录做一次绕过 shim 的 `O_PATH` 打开并报
`ENOENT`；直接的 libc `statfs()`、`udevadm` 和 Python `openat` 均已通过。此项作为
coreutils 调用路径的未覆盖面保留，不冒充已全修。

## 4. D-Bus 消费端验证（FACT）

启动 guest 系统服务后，`net.hadess.SensorProxy` 可被列出：

```text
gdbus call ... Properties.GetAll net.hadess.SensorProxy
({'HasAccelerometer': <true>, 'AccelerometerOrientation': <'undefined'>,
  'HasAmbientLight': <true>, 'LightLevelUnit': <'lux'>, 'LightLevel': <0.0>,
  'HasProximity': <true>, 'ProximityNear': <true>},)

ClaimAccelerometer() -> ()
ClaimLight()          -> ()
ClaimProximity()      -> ()
```

`monitor-sensor` 输出：

```text
=== Has accelerometer (orientation: undefined)
=== Has ambient light sensor (value: 0.000000, unit: lux)
=== Has proximity sensor (near: 1)
```

这证明链路是：

```text
NDK ASensorManager_* -> app-rs 事件线程 -> IIO sysfs -> iio-sensor-proxy
                     -> D-Bus net.hadess.SensorProxy -> monitor-sensor
```

传感器仍是**中转**，不是直连；NDK 是宿主接口，IIO 和 D-Bus 是 guest 侧标准接口。

## 5. 回归测试

新增 `app-rs/src/sensors.rs` 单元测试，覆盖：

- `bus/iio` 和 `class/iio` 是真实目录，不是整棵 symlink；
- `class` 和 `bus` 下的设备链接目标；
- 从两个视图读取同一个 canonical 设备；
- 重复启动时重建视图而不丢失 canonical 属性。

设备侧还要同时检查：

```text
cat /sys/class/iio/iio:device0/name
cat /sys/class/iio/iio:device0/in_accel_x_raw
cat /sys/bus/iio/devices/iio:device0/name
cat /sys/subsystem/iio/devices/iio:device0/name
udevadm info -q property -p /sys/class/iio/iio:device0
timeout 6 monitor-sensor
gdbus call ... ClaimAccelerometer / ClaimLight / ClaimProximity
```

## 6. 未验证边界

- 只验证当前参考机和当前 Android/内核；不能据此宣布所有 Android 设备通用。
- 只映射五类默认传感器；未校准磁力、未校准陀螺、计步等没有额外 IIO 节点。
- `stat -f` 的 coreutils 目录预打开路径仍是未覆盖面；libc `statfs()` 与 `udevadm` 通过。
- 传感器 D-Bus 的跨会话、多用户和长时间热插拔没有验证。
