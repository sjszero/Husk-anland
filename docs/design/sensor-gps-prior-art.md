# 传感器与 GPS：前人做法源码级审计

> 审计日期：2026-11-23。写这份是为了在做【E 硬件 · 传感器 / 位置】之前先把别人的解法读完，
> 而不是自己从零设计。**只写读到的源码与本机实测，推断单列。**

## 1. 读了什么（FACT）

| 来源 | 位置 / 版本 | 读了哪些文件 |
|---|---|---|
| DroidNative | `/tmp/refs2/DroidNative_DroidNativeApp`（`fa59101`） | `app/src/main/kotlin/app/droidnative/devices/SensorBridge.kt`（127 行，全文）、`LocationBridge.kt`（128 行）、`docs/device-passthrough.md` §Sensors: IIO sysfs / §Location: NMEA / §Android restrictions |
| 本机 Android | RMX3357，NDK 29 | `pm list features`；`/system/lib64/libandroid.so` 导出表 |

## 2. DroidNative 的传感器做法（FACT，逐条来自源码）

guest 里看得到的**不是** `/dev/iio:deviceN`，而是一棵**伪造的 IIO sysfs 树**，
绑到 `/sys/bus/iio/devices`：

- `prepare()` 每发现一个 `SensorManager.getDefaultSensor(type)`，就建 `iio:deviceN/`
  目录，写 `name`（内核风格名，如 `accel_3d`）、`label`（Android 的传感器名）、
  以及静态属性（`in_accel_scale`、`in_accel_mount_matrix` 等）；
- 值文件（`in_accel_{x,y,z}_raw`、`in_anglvel_*_raw`、`in_magn_*_raw`、
  `in_illuminance_input`、`in_pressure_input`、`in_proximity_raw`）预先创建，
  之后由 `SensorEventListener` **原地覆盖写**；
- 写频率上限 `MIN_INTERVAL_MS = 50`（20 Hz），注册用 `SENSOR_DELAY_UI`；
- 每个值**右对齐补空格到固定宽度 15** 再换行 —— 目的是让一直开着 fd 的读者
  不会读到「旧的长值 + 新的短值」混在一起；
- 换算：accel/gyro `scale=0.001`（原始值 = 值 / 0.001 取整），
  magn 原始值 = µT / 100 / 0.0001（gauss）；
- **没有** `/dev/iio:deviceN` 缓冲接口，**没有** udev 事件，作者明确写了
  `iio-sensor-proxy` 找不到这些传感器，只能直接读 sysfs。

GPS：`/dev/gps0` 输出校验过的 `$GPGGA` / `$GPRMC`（CRLF），像串口 GPS；
只在有程序打开时更新（app 侧每 2 s 检查一次），打开即先给最后一次定位；
不支持 tty ioctl（波特率等）；provider 按权限走 GPS / fused / network。

权限边界（他们踩过的坑）：camera / mic / location 只在前台或有对应类型前台服务时可用，
他们的环境服务是 `specialUse`，所以窗口不可见时相机打不开、麦克风录到静音、定位停更。

## 3. 本机实测（FACT，2026-11-23，`untrusted_app`）

```
run-as com.linuxhub.husk ls /sys/bus/iio/devices   → Permission denied
run-as com.linuxhub.husk ls /dev/snd               → Permission denied
run-as com.linuxhub.husk ls /dev/gps*              → No such file or directory
adb shell (shell uid) ls /sys/bus/iio/devices      → iio:device0 iio:device1 iio:device2
pm list features → sensor.accelerometer / compass / gyroscope / light / proximity /
                   stepcounter / stepdetector / location.gps 全部存在
libandroid.so 导出 ASensorManager_* 与 ASensorEventQueue_*（getInstance、
  getDefaultSensor、createEventQueue、enableSensor、setEventRate、getEvents、
  createSharedMemoryDirectChannel、createHardwareBufferDirectChannel …）
```

结论（INFERENCE，依据上面两条）：guest 侧**不可能**直接读真 sysfs（SELinux 挡住），
所以 DroidNative 的「伪造树」不是偷懒，是唯一可行形态；我们照抄这个形态，
但**不需要 bind mount**（我们没有 namespace），直接把 `/sys/bus/iio/devices/...`
在 path-shim 里重定向到 `$F/sensors-iio/`，和现有的路径重定向是同一套机制。

## 4. Husk 的决定

1. **接口形态**：guest 看到 `/sys/bus/iio/devices/iio:deviceN/*`（与 DroidNative 一致）。
   不用 `/dev/iio:deviceN`、不用 udev 事件 —— 与 `device-interface-choices.md` 的
   「guest 里长得像内核」原则一致。
2. **app 侧零 Java**：用 NDK `ASensorManager_*`（已确认本机 libandroid.so 导出），
   在 Rust 里注册 `ASensorEventQueue` + `ALooper`，把值写进私有目录的伪造树。
   DroidNative 用 Kotlin `SensorManager`，我们不必跟着用 JVM。
3. **写格式照抄**：右对齐定宽 + 原地 pwrite + 20 Hz 上限。这是前人用真实读者验证过的
   防撕裂办法，我们没有更好的理由换掉。
4. **GPS**：guest 侧 `/dev/gps0` 用 FIFO 实现（app 侧 Rust 写 NMEA，path-shim 把
   `open("/dev/gps0")` 重定向到 `$F/run/gps0.fifo`），NMEA 句子用 `LocationManager`
   的 NDK 等价物 —— **UNKNOWN**：NDK 无 location API，`libgps` 也没有；
   见 §5。

## 5. 缺口与未决（明确标 UNKNOWN）

| 项 | 状态 |
|---|---|
| NDK 有没有定位 API | **UNKNOWN / 倾向于没有**：NDK 头文件里没有 location 相关声明。DroidNative 走 Java `LocationManager`。若确认无 NDK API，GPS 桥需要一段最小 JNI 垫片（与 USB 同类问题，见 `platform-peripherals-prior-art.md`）。 |
| `stepcounter` / `stepdetector` | 未做。NDK `ASensor` 能拿到，但 IIO 里没有对应文件；要给 guest 一个约定（`in_steps_input`?）需要先看 guest 侧谁消费。 |
| 传感器耗电 | 20 Hz 常开在 untrusted_app 里是否被限流，未测。 |
