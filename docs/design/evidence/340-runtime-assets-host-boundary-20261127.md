# 340 · 架构纠正：Husk 只发运行时资产，宿主拥有 X11/桌面

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未发布）

## 0 · 结论

- **FACT**：`v0.0.10`、`v0.0.20` 和 `v0.0.36` 的仓库都没有 `app-rs/src/desktop.rs` 或
  `third_party/xlorie`。这些版本的交付契约都是“运行时作为 assets 打进宿主 App”。
- **FACT**：`a41f022`（`v0.0.36`，2026-11-27 03:50）在 `docs/LIMITATIONS.md` 明确写明：
  X11/桌面是宿主能力，发行资产不包含 Xorg/Xvfb/应用内 X server。
- **FACT**：`895ef43`（2026-11-27 08:15）第一次把“宿主 App 没有自己的 X server”记成 Husk
  运行时缺陷 `G5`，并要求加入应用内 X server。这是方针污染点。
- **FACT**：`35ab14a`（2026-11-27 11:08）把该错误方针落成代码：新增 Xlorie、同进程
  `desktop.rs`、APK 内的 `libXlorie.so` 和对应桌面门禁。这是代码污染点。
- **JUDGMENT**：Husk 应继续以“可被任意宿主 App 装载的 Linux ARM64 运行时”为产品边界。
  X11 保持，但 X server、输入注入和 Surface 呈现属于宿主；Husk 发行包不内嵌 X server。

## 1 · 修前负向结果

历史对象：

```text
git log --all --oneline -- app-rs/src/desktop.rs third_party/xlorie
35ab14a feat(runtime): embed X11 session and desktop acceptance gates
```

修前构建要求四块 `.so`：

```text
libhuskapp.so libhuskguard.so libld.so libXlorie.so
```

修前 APK 审计也把 `libXlorie.so` 当成必需项。仅从 Makefile 删除复制命令还不够：
第一次重建时，旧的 `app/src/main/jniLibs/arm64-v8a/libXlorie.so` 仍被 AGP 自动打进 APK。
这说明必须有“禁止陈旧件”的负向门禁，而不是只删生产命令。

复现：

```sh
unzip -l app/app/build/outputs/apk/debug/app-debug.apk | grep libXlorie.so
```

## 2 · 修复范围

保留：

- `rootfs-full -> rootfs` 选择修复和 3 个 Rust 单测；
- Android 相机、定位、传感器、GPU loader 默认值；
- Rust screenshot/readback/probe 工具；
- 宿主需要的 guest 脚本和配置。

移除：

- `app-rs/src/desktop.rs`；
- `third_party/xlorie/`（源码、patches、预编译 `.so`）；
- NativeActivity 内的桌面启动/输入路由；
- APK 构建、Gradle guard、APK 审计和 CI 对 `libXlorie.so` 的依赖；
- 只验证错误内嵌链路的 `22-desktop-acceptance.sh` / `23-desktop-multiapp.sh`。

新增：

- `tools/ci/audit-runtime-boundary.sh`：发行 tarball 禁止
  `libXlorie.so`、Xorg/Xvfb/Xwayland、`xlorie/` 和 `.apk`；
- `tools/ci/test-runtime-boundary.sh`：有效 bundle 必须通过，注入 `libXlorie.so` 必须失败；
- `make dist` 直接调用 runtime boundary audit；
- CI 的 `scripts` 作业运行 boundary test。

## 3 · 负向门禁结果

### 3.1 运行时 tarball

```text
runtime boundary test: PASS (negative Xlorie rejected, valid bundle accepted)
```

### 3.2 APK 内容审计

对正常 APK 注入 `lib/arm64-v8a/libXlorie.so` 后：

```text
OK   lib/arm64-v8a/libhuskapp.so
OK   lib/arm64-v8a/libhuskguard.so
OK   lib/arm64-v8a/libld.so
FAIL forbidden host UI native piece in /tmp/.../bad.apk: lib/arm64-v8a/libXlorie.so
```

预期失败，实际 rc=1。

### 3.3 Gradle 陈旧件 guard

在 `jniLibs/arm64-v8a/libXlorie.so` 放入测试文件后：

```text
> Task :app:huskVerifyNativePieces FAILED
Husk: 检测到宿主 UI 原生件 libXlorie.so。Husk 只发运行时；X server/桌面由宿主 App 提供。
BUILD FAILED
```

预期失败，实际 rc=1。测试文件随后删除。

## 4 · 修复后正向结果

### 4.1 静态/构建门禁

```text
NDK=<ndk/bin> make apk
BUILD SUCCESSFUL
app-debug.apk = 3,305,888 bytes
```

APK 内只有：

```text
lib/arm64-v8a/libhuskapp.so
lib/arm64-v8a/libhuskguard.so
lib/arm64-v8a/libld.so
```

```text
OK   lib/arm64-v8a/libhuskapp.so
OK   lib/arm64-v8a/libhuskguard.so
OK   lib/arm64-v8a/libld.so
OK   forbidden lib/arm64-v8a/libXlorie.so absent
APK version contract OK: 0.0.36 (36)
```

`NDK=<ndk/bin> make check` 全绿，包括：

```text
runtime boundary test: PASS
rootfs selection: PASS (3/3 Rust tests)
cargo test -p husk-tools: 9 passed
runtime/target/debug/husk-tools check-links: 366 files, 0 broken
version ok: 0.0.36
```

### 4.2 真机运行

覆盖安装保留应用数据：

```text
Success
versionCode=36 versionName=0.0.36
```

NativeActivity 启动后，日志显示：

```text
I/Husk: session: guard pid=10618 ready
I/Husk: spawned pid=10631 fd=93 ...
I/Husk: session: running pid=10631 ... 80x24 fds=118
```

设备从睡眠唤醒后，终端截图显示 guest shell；输入：

```sh
uname -a
```

返回：

```text
Linux localhost 4.19.191 ... aarch64 GNU/Linux
```

截图：[340-runtime-only-terminal-20261127.png](../../assets/evidence/340-runtime-only-terminal-20261127.png)

### 4.3 Runtime 树体检

```text
DEV=... ROOTFS=files/rootfs-full EXPECT=base sh tools/device-tests/20-tree-integrity.sh
TREE_INTEGRITY PASS=14 FAIL=0
```

修正前同一条命令因把宿主桌面辅助件 `wallpaper-scale` 算进 runtime `base`，得到
`PASS=14 FAIL=1`。修正后该检查只在 `EXPECT=desktop` 执行。

### 4.4 门禁 A

```text
DEV=... ROOTFS=files/rootfs-full LD=files/linux/husk/libld.so \
  sh tools/device-tests/14-gate-a-acceptance.sh
PASS=38 FAIL=0 SKIP=0
```

覆盖 bash、中文 locale、useradd/chpasswd、dpkg、apt、host-path 隔离和真实 SIGSYS trap=0。

### 4.5 GPU loader 默认值

```text
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1: llvmpipe
HUSK_GPU=llvmpipe: llvmpipe
```

## 5 · 未验证 / 不在本轮

- **UNVERIFIED**：宿主 App 自己的 X11 server、输入注入和 Surface 呈现；它们不属于 Husk
  运行时，本轮没有把其他 App 的 socket 冒充成 Husk 证据。
- **UNKNOWN**：其他宿主实现、其他 GPU 家族、16/64 KiB 真机。
- **FACT**：本轮没有 bump `VERSION`，没有 tag，没有 Release。
