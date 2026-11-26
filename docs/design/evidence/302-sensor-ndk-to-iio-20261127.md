# 302 · 传感器：NDK 事件到 guest IIO 影子树

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
目标环境：`com.linuxhub.husk`
涉及文件：

- `app-rs/src/sensors.rs`（新增）
- `app-rs/src/lib.rs`
- `app-rs/src/caps.rs`
- `runtime/crates/path-shim/src/lib.rs`

## 0. 结论

- **FACT**：实现完全走公开 NDK C API，不新增 Java Activity / Listener；`hasCode=false` 不变。
- **FACT**：支持五类默认传感器：加速度、磁力、陀螺仪、环境光、距离。
- **FACT**：事件只来自真实 NDK 回调；首次事件前值为零，不伪造硬件采样。
- **FACT**：guest 路径 `/sys/bus/iio/devices`、`/sys/class/iio`、`/sys/subsystem/iio` 由 `path-shim` 映射到 app 数据目录下的 `sys-iio/` 影子树。
- **FACT**：`app-rs` 的 `cargo check --target aarch64-linux-android` 通过；`path-shim` 同目标 `cargo check` 通过。
- **FACT**：真机日志显示五类传感器全部 `READY`，事件队列 `RUN`；桥持续收到真实 NDK 事件。
- **FACT**：使用当前最新 loader（`96b96bbe...`）后，guest 已能通过三条标准路径读取 IIO 属性和枚举节点。
- **FACT**：三次采样中加速度数值从 `-97,417,9743` 变为 `-94,419,9728`；静止时保持稳定，不是伪造随机数。
- **UNVERIFIED**：本机没有安装 `iio-sensor-proxy`，因此没有宣称该守护进程识别成功。

## 1. 为什么不是直接读宿主 sysfs

**FACT**：`untrusted_app` 对真实 `/sys/bus/iio/devices` 没有读权限；前序设备探测记录过 `EACCES`。因此不能把真实 sysfs 原样暴露给 guest。

采用两层：

```text
Android NDK ASensorManager / ASensorEventQueue
             ↓ 真实事件
app-rs 传感器线程
             ↓ 原子替换属性文件
<app files>/sys-iio/{bus/iio/devices,class/iio,subsystem/iio}/iio:deviceN/*
             ↓ path-shim 只改路径
guest 看到的 /sys/bus/iio/devices/... 标准 IIO 接口
```

这是**中转**，不是直连。应用不需要改代码，也不需要 Java 侧监听器。

## 2. NDK API 集合

运行时 `dlopen("libandroid.so", RTLD_NOW)` 后解析：

```text
ASensorManager_getInstance
ASensorManager_getDefaultSensor
ASensorManager_createEventQueue
ASensorManager_destroyEventQueue
ASensorEventQueue_enableSensor
ASensorEventQueue_setEventRate
ASensorEventQueue_getEvents
ALooper_prepare
ALooper_pollOnce
```

事件结构按 NDK `ASensorEvent` 布局读取：`version/sensor/type/reserved0/timestamp/data[16]/flags/reserved1[3]`。

## 3. 节点和单位

| 设备 | Android 类型 | IIO 名称 | 属性 |
|---|---|---|---|
| `iio:device0` | 加速度 | `accel_3d` | `in_accel_{x,y,z}_raw`、`in_accel_scale`、`sampling_frequency` |
| `iio:device1` | 磁力 | `magn_3d` | `in_magn_{x,y,z}_raw`、`in_magn_scale`、`sampling_frequency` |
| `iio:device2` | 陀螺仪 | `gyro_3d` | `in_anglvel_{x,y,z}_raw`、`in_anglvel_scale`、`sampling_frequency` |
| `iio:device3` | 环境光 | `als` | `in_illuminance_raw`、`in_illuminance_scale`、`sampling_frequency` |
| `iio:device4` | 距离 | `proximity` | `in_proximity_raw`、`in_proximity_scale`、`sampling_frequency` |

三轴传感器用 `scale=0.001`，事件值乘以 1000 写成整数 raw；环境光和距离的原生单位与 scale `1.0` 一致，不再额外放大 1000 倍。

默认采样率请求 10 Hz（`100_000` 微秒）。

## 4. 更新方式

每个属性写入同目录临时文件后 `rename()` 覆盖。读者只能看到完整旧值或完整新值，不会看到截断到一半的十进制文本。三个轴分别更新，因此读者仍可能在极窄窗口看到不同事件批次的 x/y/z；这不影响数值有效性，但不宣称三轴是原子快照。

## 5. 失败行为

- 某类默认传感器不存在：日志 `sensors|ABSENT|...`，其余类型继续。
- 单类 enable 失败：日志 `sensors|ENABLE|... rc=<负数>`，不崩进程。
- 一类也没有启用：记 `sensors|FAIL|no default sensor enabled` 并销毁队列。
- `libandroid.so` 或符号缺失：记 `sensors|FAIL|...`，不拖垮 App。
- 整个线程用 `catch_unwind()` 隔离 panic。

## 6. 静态验证（FACT）

```text
$ cd app-rs && cargo check --target aarch64-linux-android
Finished `dev` profile target(s) in 0.80s

$ cd runtime && cargo check -p path-shim --target aarch64-linux-android
Finished `dev` profile target(s) in 0.06s
```

`rustfmt --check` 对 `app-rs/src/sensors.rs` 通过。

## 7. 真机正验证（FACT）

APK 启动后 logcat：

```text
sensors|TREE|/data/user/0/com.linuxhub.husk/files/rootfs-full
sensors|READY|type=1 name=accel_3d actual="lsm6dsm ACCELEROMETER"
sensors|READY|type=2 name=magn_3d actual="mmc5603 MAGNETOMETER"
sensors|READY|type=4 name=gyro_3d actual="lsm6dsm GYROSCOPE"
sensors|READY|type=5 name=als actual="OPLUS Fusion Light Sensor"
sensors|READY|type=8 name=proximity actual="tcs3701 PROXIMITY"
sensors|RUN|enabled=5
```

guest probe 通过最新 loader（`files/linux-clean/libld.so`，sha256 `96b96bbe...`）执行：

```text
/sys/bus/iio/devices/iio:device0/name=accel_3d
/sys/bus/iio/devices/iio:device0/in_accel_x_raw=-94
/sys/bus/iio/devices/iio:device0/in_accel_y_raw=419
/sys/bus/iio/devices/iio:device0/in_accel_z_raw=9728
/sys/class/iio/iio:device0/name=accel_3d
/sys/subsystem/iio/iio:device0/name=accel_3d
```

标准目录枚举也成功：

```text
total 18
drwx------. 2 husk husk10303 3488 iio:device0
drwx------. 2 husk husk10303 3488 iio:device1
drwx------. 2 husk husk10303 3488 iio:device2
drwx------? 2 husk husk10303 3488 iio:device3
drwx------? 2 husk husk10303 3488 iio:device4
```

跨采样变化：

```text
-97 417 9743
-94 419 9728
```

静止时连续三次读取均为：

```text
-94 419 9728
-94 419 9728
-94 419 9728
```

这与“事件驱动、无新事件时不制造变化”的实现一致。

## 8. 未验证边界

- `iio-sensor-proxy` 不在当前 807 包树中，未验证其 D-Bus 输出；标准 IIO sysfs 读取已验证。
- 只验证五类默认传感器。Android 暴露的未校准磁力、未校准陀螺、计步等类型没有映射到额外 IIO 节点。
