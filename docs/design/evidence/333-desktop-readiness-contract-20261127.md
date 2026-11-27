# 333：内嵌桌面建立 starting → ready / failed 状态契约

日期：2026-11-27

## 结论

- **FACT**：修前 `desktop::ACTIVE` 在 Xlorie 启动并 `fork()` 返回后立即置真；它只代表
  “服务端已启动、launcher 已 fork”，不代表 GNOME/WM 已就绪。guest launcher 以 70/71
  退出时，应用仍把这个状态当作桌面活动会话。
- **FACT**：`husk-desktop.sh` 现在原子写入
  `tmp/husk-desktop.status`：启动时 `starting`，成功时 `ready + wm/ding/gpu/input/im`，
  非零退出时 `failed + rc`。
- **FACT**：Rust 桌面桥在后台监控该文件；`ready` 后记录明确日志，`failed` 会撤销
  `ACTIVE` 并记录失败状态，60 秒未完成则记录 readiness timeout。
- **FACT**：能力矩阵 display 行现在区分 `x11=ready`、`x11=starting`、`x11=absent`。
- **UNVERIFIED**：真机是否能在 60 秒内进入 `ready`；设备仍处于 3 万级进程/线程和异常高
  load 状态，本轮没有启动 Husk。

## 修前问题

旧调用顺序：

```text
husk_xlorie_start -> husk_xlorie_connect -> fork launcher -> ACTIVE=true
```

guest launcher 中有多个会导致退出的真实路径，例如会话总线起不来 `exit 70`、WM 自检失败
`exit 71`。这些错误只写 `husk-desktop.log`，应用无法区分“正在启动”“桌面成功”“启动失败”，
能力矩阵也只能看到一个笼统的 active 状态。

## 修复

### Guest launcher

`tools/guest/husk-desktop.sh` 新增原子状态写入：

```text
starting
ready
wm=yes
ding=...
gpu=...
input=...
im=...
```

失败时：

```text
failed
rc=<nonzero>
```

写临时文件后 `mv` 到最终路径，Rust 侧不会读到半行 JSON/状态。

### Rust 桌面桥

`app-rs/src/desktop.rs` 新增：

- `READY` 原子状态；
- `monitor_readiness()` 每 250 ms 读取状态文件；
- `ready` 与 `failed` 的明确 logcat 行；
- 60 秒超时日志，不把超时升级成成功。

### 能力矩阵

`app-rs/src/caps.rs` 的 display 行现在输出：

```text
x11=ready
x11=starting
x11=absent
```

新增 `tools/ci/check-desktop-session-contract.sh`，并接入 `make check`，防止 launcher 状态、
Rust 监控和能力矩阵再次脱节。

## 验证

```text
cargo check app-rs --target aarch64-linux-android --lib --bin husk-session-guard  PASS
desktop session contract: PASS (starting -> ready/failed -> caps)
make check                                                                        PASS
shell syntax                                                                      104 scripts PASS
```

APK 已统一重打并通过版本/原生件审计：

```text
app-debug.apk sha256  9b6f8c6215d2c662fcc3cb1bb116026864199bc36639441e806803769601a0c8
versionCode / versionName  36 / 0.0.36
```

本证据不以编译成功代替真机桌面成功。

## 真机阻断

最后只读检查仍显示：

```text
/proc/loadavg     22.54 38.52 41.91 1/31765 2816
MemAvailable      3,271,020 kB
process/threads   31,765
Husk PID          absent
```

因此不能把 readiness contract 说成已经在真机进入 `ready`。需要设备恢复后检查：

- `desktop: ready ... status=ready;wm=yes;...`；
- `x11=ready` 是否写入能力矩阵；
- `screencap` 非黑；
- 失败时是否出现 `desktop: launcher failed ...` 而不是静默黑屏。

## 边界

本次没有 bump `VERSION`、没有 tag、没有 Release。readiness 判断只说明 guest launcher 的
自检通过，不证明桌面像素、输入、GPU 或长稳通过。
