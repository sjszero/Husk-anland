# 128 · 外设族探测表：传感器 / GPS / USB / 蓝牙 / NFC / 指纹 在本机的真实答案

日期：2026-11-23　设备：RMX3357（`UKHMMB8PYTGECMNV`），`run-as com.linuxhub.husk`，guest 内执行
被测物：`gpu-probe periph`（新增模式，纯 Rust，静态 musl）

## 1. 为什么要这个

目标里 C 段的口径是"没设备就只做两件事：在探测表里留位置、缺失时明确降级且不崩"。
"本机没有传感器"必须是**测出来的**，而且要把"没有这个节点"和"有但没权限"分开——
二者对后续工作的含义完全不同（前者是硬件不存在，后者是 SELinux 挡住，换域/换机型可能就有了）。

## 2. 怎么建的（这台 Mac 上没有 gnu sysroot 的解法）

`gpu-probe` 以前是 `aarch64-unknown-linux-gnu` 动态编译、在 guest 里由我们的 loader 带起来。
这台机器上 `rust-lld` 找不到 `-lgcc_s/-lutil/-lrt/-lc`（没有 aarch64 gnu sysroot），
改用 musl 静态目标，一次到位、也不依赖 guest 里的 libc：

```sh
rustup target add aarch64-unknown-linux-musl        # 需要代理
cd runtime && CARGO_TARGET_DIR=/tmp/hcheck \
  RUSTFLAGS="-C target-feature=+crt-static -C linker=rust-lld" \
  cargo build --release --target aarch64-unknown-linux-musl -p gpu-probe
```

产物体检（`nr-tools check-elf`）：`p_align=0x10000 loads=4  4096:ok 16384:ok 65536:ok`。
静态 ELF 没有 `PT_INTERP`，shim 的 `execve` 直接把它交给内核，不经过 loader 重入。

## 3. 实测（FACT，`sh gexec.sh rootfs-full usr/bin/gpu-probe periph`）

| 家族 | 节点 | 结果 |
|---|---|---|
| sensor | `/dev/iio:device0` | **errno=13 (EACCES)** |
| sensor | `/dev/sensorhub` | errno=2 (ENOENT) |
| gps | `/dev/gps0` | errno=2 |
| usb | `/dev/bus/usb` | errno=2 |
| bluetooth | `/dev/rfkill`、`/dev/hci0` | errno=2 / errno=2 |
| nfc | `/dev/nfc0` | errno=2 |
| fingerprint | `/dev/fingerprint` | errno=2 |
| input | `/dev/input/event0` | **errno=13 (EACCES)** |
| hidraw | `/dev/hidraw0` | errno=2 |
| i2c / spi | `/dev/i2c-0`、`/dev/spidev0.0` | errno=2 / errno=2 |
| sysfs | `/sys/class/iio`、`/sys/class/gps`、`/sys/class/nfc` | errno=2 |
| sysfs | `/sys/class/bluetooth`、`/sys/class/input`、`/sys/bus/usb` | **errno=13 (EACCES)** |

最后一行 `periph|verdict|DONE|no device is faked; missing families are plain ENOENT`，
进程退出码 0。

## 4. 结论与不能说的话

- **FACT**：我们没有给任何外设造假节点；guest 拿到的就是内核/ SELinux 的真实答案，且不崩。
- **FACT**：`EACCES(13)` 与 `ENOENT(2)` 在这里是两类不同的事实。
  `/dev/iio:device0`、`/dev/input/event0`、`/sys/class/{input,bluetooth}`、`/sys/bus/usb`
  是**存在但被 SELinux 拒绝**（untrusted_app 域），不是"设备不存在"。
- **UNKNOWN**：本机到底有没有传感器/指纹硬件——`EACCES` 把这件事藏住了。要回答它必须换一个
  能看到这些节点的域（例如带 Shizuku 的 shell/adb 侧实测，或另一台机型）。
- **UNKNOWN**：Adreno、USB host 模式、蓝牙栈接入的实际可用性——没有设备，没有验证。
- 由于 `/dev` 目录不可列（evidence 118 的墙），"按名字打开"是唯一可用的探测手段，
  所以这张表只能是**已知候选名单**，不是完整枚举。

## 5. 复现

```sh
adb push /tmp/hcheck/aarch64-unknown-linux-musl/release/gpu-probe /data/local/tmp/gpu-probe
adb shell 'run-as com.linuxhub.husk sh -c "cp /data/local/tmp/gpu-probe \
  files/rootfs-full/usr/bin/gpu-probe && chmod 755 files/rootfs-full/usr/bin/gpu-probe"'
adb shell 'run-as com.linuxhub.husk sh -c "cd files && \
  NR_LD=/data/data/com.linuxhub.husk/files/libld5.so sh gexec.sh rootfs-full usr/bin/gpu-probe periph"'
```
