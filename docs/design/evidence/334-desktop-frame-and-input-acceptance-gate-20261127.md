# 334：内嵌桌面帧与输入验收门禁

日期：2026-11-27

## 结论

- **FACT**：修前没有自动检查内嵌桌面实际帧。`make gpu-audit` 能证明 GNOME/GTK 的
  GPU 路径，`check-desktop-session-contract.sh` 能证明 launcher 会写 `ready/failed`，
  但二者都不会拒绝一张全黑、只渲染半边或根本没进入 GNOME 的截图。
- **FACT**：新增纯 Rust `husk-tools screenshot-audit`。它直接读取 Android
  `screencap` 的 raw RGBA/RGBX/BGRA，解析 12/16 字节头，裁掉外圈 15%，统计中心亮度、
  近黑比例、量化色数和四象限；`--strict` 会拒绝全黑中心或“至少两个中心象限全黑、
  另一个象限明显有内容”的大块缺失。
- **FACT**：新增 `tools/device-tests/22-desktop-acceptance.sh`。它在启动前检查设备
  load、进程/线程数和可用内存；健康不达标时返回 2，不启动 Husk。健康达标后才只重启
  `com.linuxhub.husk`，等待 `husk-desktop.status`，检查 `wm/ding/gpu/input/im`、
  `x11=ready`、MIT-SHM 关闭、前台 Activity、raw 截图、Android tap/key 是否进入 Xlorie。
- **UNVERIFIED**：当前设备仍处于异常高负载，新增门禁只完成离线验证和“拒绝启动”负向验证；
  真实桌面帧、tap/key、多指、滚轮和软键盘尚未在本轮设备上通过。

## 修前问题

旧门禁存在两个空洞：

1. readiness 只看 guest 文件和日志，不看屏幕；
2. 输入路由只有静态调用检查，没有证明 Android 注入后的 `desktop|input|...` 日志真的出现。

因此一个 launcher 自检成功但画面全黑、或应用在前台但不接收触摸的会话，仍可能被人工误判。

## 修复

### Rust 帧审计

新增：

```text
runtime/crates/husk-tools/src/screenshot_audit.rs
```

命令行：

```sh
runtime/target/debug/husk-tools screenshot-audit --strict <raw-screencap>
```

它不引入外部 crate，不解析 PNG，不按主题或应用名猜测“桌面是否好看”。输出示例：

```text
screenshot-audit: PASS width=1080 height=2400 header=16 samples=141120 mean=... stddev=... near_black=... quadrant_mean=...
```

### 设备验收

新增：

```text
tools/device-tests/22-desktop-acceptance.sh
```

Makefile 入口：

```sh
DEV=<serial> make desktop-acceptance
```

设备健康门槛默认：

```text
load1 <= 12
process/threads <= 12000
MemAvailable >= 2 GiB
```

超限直接 `exit 2`。门禁只 force-stop 自己的 `com.linuxhub.husk`，不碰
`com.linuxhub.myth`、`com.linuxhub.myth:x11` 或 `u0_a300`。

## 验证

Rust 单测：

```text
cargo test -p husk-tools
5 passed / 0 failed

black_frame_fails_even_without_strict
uniform_visible_frame_passes
missing_right_half_fails_strict
png_or_truncated_data_is_rejected
twelve_byte_header_is_accepted
```

用当前真机真实 raw 截图验证解析器：

```text
file bytes       10,368,016
header           16 bytes
width x height   1080 x 2400
PixelFormat      1 (RGBA_8888)
samples          141,120
mean             0.00
stddev           0.00
near_black       1.0000
quantized colors 1
verdict          FAIL: centre crop is all black
```

这是设备屏幕当前全黑的负向证据；不能把它写成桌面已通过。

设备健康预检实际输出：

```text
load1=22.30
process/threads=31803
MemAvailable=3,260,368 kB
DESKTOP_ACCEPTANCE PASS=3 FAIL=2
exit=2 (BLOCKED: device health)
```

完整离线门禁：

```text
make check                              PASS
husk-tools tests                        5/5 PASS
shell syntax                            105 scripts PASS
desktop session contract                PASS (MIT-SHM off)
APK version                             0.0.36 / 36
APK native pieces                       4/4 PASS
libXlorie.so                            76b4abc6... PASS
app-debug.apk                           9b6f8c6215d2c662fcc3cb1bb116026864199bc36639441e806803769601a0c8
```

未执行安装和启动：设备健康门槛按设计拒绝了。

## 待验证

- `husk-desktop.status` 是否在 90 秒内为 `ready`，且 `wm=yes/ding=yes/gpu=zink/input=primed`；
- 真实桌面截图的中心/象限指标；
- tap 后 `desktop|input|touch`、按键后 `desktop|input|key`；
- 多指、相对触控板、滚轮、软键盘和中文输入；
- 会话期间 GPU、CPU、RSS、fd 长稳；
- 同一测试在 Adreno 和其他 Android 版本上的结果。

## 边界

本次没有 bump `VERSION`、没有 tag、没有 Release。帧审计只拒绝全黑和大块缺失，不证明
窗口布局、主题、圆角、动画或零拷贝呈现正确。
