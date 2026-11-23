# USB / 蓝牙 / NFC / 指纹：平台外设的可达性审计

> 审计日期：2026-11-23。目的：在写任何代码前，先确定这四类东西在
> **不 root、untrusted_app** 的前提下**到底能不能接**、接到哪一层、guest 里长什么样。
> 结论按 FACT / INFERENCE / UNKNOWN 分开；没有实测的一律不写 PASS。

## 1. 本机事实（FACT，2026-11-23，RMX3357）

| 检查 | 结果 |
|---|---|
| `pm list features` | `android.hardware.usb.host`、`android.hardware.bluetooth`、`bluetooth_le`、`android.hardware.nfc`（含 `nfc.hce` / `nfc.hcef` / `nfc.ese` / `nfc.uicc`）、`android.hardware.fingerprint` 全部存在 |
| `ls /dev/bus`（shell uid） | **不存在** —— 本机内核/Android **没有** usbfs 节点，不是权限问题 |
| `ls /sys/class/bluetooth` | **空** —— 没有内核 HCI 设备节点 |
| `ls /dev/vhci` | **不存在** —— 没有 VHCI，无法给 BlueZ 造假 hci0 |
| `ls /sys/class/nfc`、`/dev/nfc*` | **不存在** |
| `/dev/hidraw*` | 不存在（shell 视角） |

NDK 29 头文件：`android/` 目录里**没有** USB / Bluetooth / NFC 任何平台 API；
只有内核 uapi：`linux/usbdevice_fs.h`、`linux/usb/`、`linux/nfc.h`、`linux/usbip.h`。

推论（INFERENCE）：这四类都不可能像 `/dev/mali0` 那样"guest 进程直接打开节点"。
一旦要接，只能是 **app 内平台 API / HAL + 桥**，而平台 API 的形态决定了 guest 侧接口。

## 2. USB：前人怎么做的（源码级）

读了 `libusb/libusb` @ `a46f434`（2026-11-17）：

- Linux/USBFS 后端在 Android 上**不做设备发现**：`LIBUSB_OPTION_NO_DEVICE_DISCOVERY`
  （`linux_usbfs.c:452`），设备靠 **`libusb_wrap_sys_device(ctx, fd, &handle)`**
  （`core.c` / `op_wrap_sys_device`，`linux_usbfs.c:1581`）把**已有的 usbfs fd**包进来。
- 这个 fd 在普通 Android 应用里只有一个来源：Java `UsbManager` → `UsbDeviceConnection`
  → `getFileDescriptor()`（系统服务通过 binder 把 `/dev/bus/usb/00X/00Y` 的 fd 递进来）。
- 包装之后所有传输都是 usbfs ioctl，实际用到的 20 个命令字（FACT，`grep -o`）：
  `CONTROL / BULK（含在 IOCTL 路径）/ SUBMITURB / REAPURBNDELAY / DISCARDURB /
   CLAIMINTERFACE / RELEASEINTERFACE / SETCONFIGURATION / SETINTERFACE / CLEAR_HALT /
   RESET / CONNECTINFO / GET_CAPABILITIES / GET_SPEED / GETDRIVER / ALLOC_STREAMS /
   FREE_STREAMS / DISCONNECT_CLAIM / CONNECT / DISCONNECT`。

### Husk 的 USB 方案（INFERENCE，未实现）

1. guest 里 `libusb` 照常看 `/dev/bus/usb/00X/00Y`；path-shim 把这些 open 重定向到
   桥的 socket（伪 fd），并在 **ioctl(fd, USBDEVFS_*) 这一层**翻译成 RPC ——
   和 GPU 那条 `/dev/dri` → kbase 的 DRM 语义层是同一个模式，只是换成 usbfs。
2. app 侧 Rust 从 Java 垫片拿到 `UsbDeviceConnection` 的 fd，然后用同一套
   `linux/usbdevice_fs.h` ioctl 在 fd 上执行（NDK 自带这个头，**不需要 Java 做传输**）。
3. **Java 垫片不可完全避免**：NDK 没有 USB 平台 API。最小垫片 = 申请权限 +
   `openDevice()` + 把 fd 递出来，其余全在 Rust。这与"零 Java"目标冲突，
   按目标里的例外条款处理：写成 **平台强制的最小 JNI 垫片**并记录在案。

### 未决

- `getFileDescriptor()` 返回的 fd 是否真的可被任意 usbfs ioctl 使用（不同厂商 ROM 可能
  已经 claim 过接口）—— **UNKNOWN**，需要在真机上插一个 U 盘/键鼠验证。
- 等时传输（`SUBMITURB` + `REAPURBNDELAY`）需要桥做双向 URB 队列，工作量远大于
  只做 bulk —— 先做 **control + bulk**，等时（音频类 USB DAC）列第二阶段。

## 3. 蓝牙：没有 HCI，就没有内核意义上的蓝牙

FACT：guest 侧 BlueZ 需要 `AF_BLUETOOTH` 的 HCI socket（需 `CAP_NET_ADMIN`，普通应用没有），
或者 `/dev/vhci`（本机没有）。所以 **`hci0` 这条路是死的**，不要再去试 `hciattach`/`btvirt`。

唯一可行形态（INFERENCE，未验证）：**在 guest 里做一个 org.bluez 的 D-Bus 门面**，
背后由 app 侧的 Android 蓝牙栈执行，协议走我们自己的 socket 通道。

- `bluetoothctl` 恰好是 D-Bus 客户端，不是 HCI 工具，所以门面做对时它**能用**；
  `hcitool` / `btmgmt` 这类直接开 HCI socket 的工具**必然不能用**，要写进平台限制。
- 能力上限就是 Android 应用拿得到的那些：GATT 客户端/服务端、BLE 广播、
  经典 SPP（按 UUID）—— **没有**底层 HCI 包、没有任意 profile、没有 sniffing。
- Android 蓝牙 API 同样**没有 NDK 版本**，需要最小 Java 垫片（同 USB）。

## 4. NFC：只能做"读卡器"姿态

FACT：没有 `/dev/nfc*`、没有 `/sys/class/nfc`，内核 NFC 子系统对本机应用不可见。
`android.hardware.nfc` + `nfc.hce` 特性存在，说明**平台**有 NFC，只是只给 Java API。

可行形态（INFERENCE）：app 侧 `NfcAdapter.enableReaderMode` / `Ndef` / HCE 服务 →
guest 侧一个 `husk-nfc` 守护进程 + D-Bus 接口，把 tag 的 UID / NDEF / 技术列表递进来。
标准 Linux 生态（`neard`、`libnfc` 的 pn53x 驱动）**都不适用**，guest 里不会有 `/dev/nfc0`。
HCE（手机当卡）在纯后台不可用，必须在有可见 Activity 时 —— 与相机/麦克风同样的前台限制。

## 5. 指纹：明确判定为不可用（JUDGMENT，理由充分）

`BiometricPrompt` 必须绑定一个可见的 `Activity` 并由用户交互完成认证；Android 明确
不允许把它做成后台能力，也不提供任何原生 fd / NDK API。guest 里没有对应的 Linux 抽象
（`fprintd` 要 libfprint 直接驱动指纹设备节点，本机根本不暴露）。

**结论：指纹不做，写成平台限制。**guest 需要生物识别的场景给"在图库里选一张图"这类
降级，不去伪造传感器。

## 6. 汇总表

| 类 | guest 看得到的接口 | app 侧 | 是否需 Java 垫片 | 状态 |
|---|---|---|---|---|
| USB | `/dev/bus/usb/*`（usbfs ioctl 语义） | Rust + usbfs ioctl | 是（取 fd） | 设计完成，未实现 |
| 蓝牙 | org.bluez D-Bus 门面 | Rust + Android BT | 是 | 设计完成，未实现 |
| NFC | `husk-nfc` 进程 + D-Bus | Rust + Android NFC | 是 | 设计完成，未实现 |
| 指纹 | 无 | — | — | **平台限制，不做** |
| 传感器 | `/sys/bus/iio/devices/*` | NDK `ASensorManager`，纯 Rust | **否** | 见 `sensor-gps-prior-art.md` |
| GPS | `/dev/gps0`（NMEA，FIFO） | 需 location API | 是或 UNKNOWN | 见 `sensor-gps-prior-art.md` |
