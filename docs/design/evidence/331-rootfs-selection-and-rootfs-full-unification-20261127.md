# 331：桌面 rootfs 选择统一到 rootfs-full

日期：2026-11-27

## 结论

- **FACT**：应用内桌面入口原先硬编码
  `files/linux/distributions/ubuntu-24.04/rootfs`，而主运行路径、相机、定位和传感器
  使用 `files/rootfs-full`。两个入口在正式安装树上可能指向不同目录。
- **FACT**：现已统一为一个只依赖安装布局的解析器：
  `files/rootfs-full` → `files/rootfs` → 明确错误。运行时不再猜测发行版目录。
- **FACT**：桌面入口对选中的树额外要求 `usr/bin/bash`。如果 `rootfs-full` 存在但
  不完整，它直接报错，不会静默回退到另一棵旧树。
- **FACT**：选择器 4 项独立 Rust 测试通过；`make check`、APK 版本审计、APK 原生件审计、
  Xlorie 资产审计均通过。
- **FACT**：后续复查发现 Make 目标和 17 个真机脚本仍默认旧的发行版路径；现已统一为
  `files/rootfs-full`，并在同一门禁中禁止可执行配置再静默指向 Ubuntu 固定目录。
- **UNVERIFIED**：真机桌面、输入、GPU、MIT-SHM 禁用后的黑屏修复以及长稳，均未在本次
  继续验证，因为设备仍处于异常高负载状态。

## 修前问题

`app-rs/src/desktop.rs` 的旧逻辑等价于：

```rust
let declared = PathBuf::from(files)
    .join("linux/distributions/ubuntu-24.04/rootfs");
let rootfs = std::fs::canonicalize(&declared).unwrap_or(declared);
```

负向条件：只存在 `files/rootfs-full` 时，旧代码不会选择它，而会返回一个不存在的
Ubuntu 固定路径。`camera.rs`、`location.rs`、`sensors.rs` 又各自重复实现了另一套
`rootfs-full` / `rootfs` 选择，存在继续漂移的风险。

## 修复

新增 `app-rs/src/guest_rootfs.rs`：

1. 优先选择 `files/rootfs-full`；
2. 没有当前树时兼容旧开发布局 `files/rootfs`；
3. 两者都不存在时返回包含两个候选路径的明确错误；
4. 桌面入口 `resolve_desktop()` 再校验 `usr/bin/bash`；
5. `lib.rs`、相机、定位和传感器统一调用该解析器。

新增 `tools/ci/test-guest-rootfs-selection.sh`，直接编译并执行解析器测试，同时禁止
`app-rs/src` 再出现 Ubuntu 固定 rootfs 路径。该门禁已接入 `make check`。

后续补齐设备测试入口：`Makefile`、`run-guest.sh`、GPU 读回/长稳、树完整性、路径竞态、
身份竞态、DRM/V4L2 fd、dpkg conffile、locale 和相机 fixture 的默认 rootfs 全部改为
`files/rootfs-full`。静态门禁同时扫描 `ROOTFS` / `ROOTFS_REL` / `GUEST_ROOT` 的可执行
默认值；需要旧发行包树时必须显式传入 `ROOTFS=`，不能静默测错树。

## 负向与正向结果

独立 Rust 测试：

```text
test missing_tree_is_an_error_not_a_distro_guess ... ok
test accepts_legacy_fallback ... ok
test prefers_current_tree ... ok
test desktop_rejects_an_incomplete_current_tree ... ok
test result: ok. 4 passed; 0 failed
rootfs selection: PASS (rootfs-full -> rootfs, no distro-pinned app or device default)
```

其中 `desktop_rejects_an_incomplete_current_tree` 同时创建有效的旧 `rootfs` 和一个缺少
`usr/bin/bash` 的 `rootfs-full`，断言必须拒绝当前不完整树而不是静默换树。

构建与产物：

```text
cargo check app-rs --target aarch64-linux-android --lib --bin husk-session-guard  PASS
make check                                                                        PASS
audit-apk-version.sh                                                               PASS  0.0.36 / 36
audit-apk-contents.sh                                                              PASS  4/4 native pieces
audit-xlorie-asset.sh                                                              PASS 76b4abc…
app-debug.apk sha256  9b6f8c6215d2c662fcc3cb1bb116026864199bc36639441e806803769601a0c8
```

## 真机健康检查

只执行只读检查，没有启动 Husk：

```text
adb device                         UKHMMB8PYTGECMNV device
sys.boot_completed                 1
/proc/loadavg                      25.22 59.61 47.95 1/31915 26917
MemAvailable                       3,284,396 kB
com.linuxhub.husk PID              absent
```

当前进程/线程总数仍超过 3 万，未达到继续启动内嵌桌面并做稳定性判断的条件。不得为了本次
测试清理或杀死 `com.linuxhub.myth`、`com.linuxhub.myth:x11` 或 `u0_a300`。

## 待验证

- `desktop: start` 与新 Xlorie 实际进入桌面；
- MIT-SHM 禁用后 `screencap` 非黑、没有 `Failed to attach to x11 shm`；
- tap / swipe / key 输入；
- 多应用并发、GPU audit、GPU soak、门禁 A / A-hw。

## 边界

本次没有 bump `VERSION`、没有 tag、没有 Release。路径 shim 仍只负责 Linux 用户态兼容与
路径语义，不是恶意代码安全沙箱。
