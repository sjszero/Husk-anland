# 348 · 硬件桥统一从活动 rootfs 解析路径

日期：2026-11-27（CST）  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页  
提交：`c924d62`  
APK：`0.0.36`，`versionCode=36`，SHA256
`acebd531501263b25819f1a5741cf19226136841e3d0f6dcb66d094fc1c61f4b`

## 0 · 结论

| 项 | 结果 | 等级 |
|---|---|---|
| 活动 rootfs 选择器之外仍有 10 处 `rootfs-full` 硬编码 | 已确认 | FACT |
| 旧树/切换树可能出现主会话能启动、硬件 socket 仍落在 `rootfs-full` | 已确认的静态路径风险 | FACT |
| 新统一入口 `guest_rootfs::resolve_path()` | 已实现并有单测 | FACT |
| H35 负向门禁 | 旧提交运行新门禁必须失败 | FACT |
| 修后安装、A-hw、音频、相机 | 通过 | FACT |
| 修后完整门禁 A | `PASS=38 FAIL=0 SKIP=0` | FACT |
| 修后覆盖扫描 | 1085 个候选、0 个未翻译 guest 绝对路径 | FACT |

这不是“换版本”提交。`VERSION`、`CHANGELOG`、tag、Release 都没有动；在全部发行门禁通过前不升版。

## 1 · 修前问题

`guest_rootfs::resolve()` 已经让主会话、相机、定位、传感器读活动树，但下列 app 侧硬件桥仍直接拼
`files/rootfs-full/...`：

- `app-rs/src/audio.rs`：播放和录音 socket、PulseAudio 启停 marker
- `app-rs/src/input.rs`：输入 socket 和 marker
- `app-rs/src/encoder.rs`：MediaCodec 编码 socket 和 marker
- `app-rs/src/ahb.rs`：AHB bridge metadata 和 socket
- `app-rs/src/video/ebridge.rs`：视频 AHB ring socket
- `app-rs/src/video/bridge.rs`：视频控制 socket、启用 marker
- `app-rs/src/video/present.rs`：SurfaceControl 呈现 marker
- `app-rs/src/caps.rs`：能力矩阵的 guest 镜像

当活动树是 legacy `files/rootfs/`，或被明确切换的树时，主会话可以起来，但这些辅助线程仍会去
`files/rootfs-full/`。这会表现为主环境可用、相机/音频/输入/视频桥单独失效，属于静默部分成功。

## 2 · 负向结果

用父提交 `c924d62^` 建立临时树，再把修后的
`tools/ci/test-guest-rootfs-selection.sh` 放进去执行：

```text
rc=1
running 3 tests ... test result: ok. 3 passed; 0 failed
rootfs selection: FAIL (bridge hardcodes an install-tree name)
```

这证明新增门禁不是只对当前工作树做正向断言；旧实现会被明确判红。

## 3 · 修法

`app-rs/src/guest_rootfs.rs` 新增：

```rust
pub(crate) fn resolve_path(files: &Path, guest_relative: &str) -> Result<PathBuf, String>
```

规则：

1. guest 传入路径必须是 rootfs 相对路径；绝对路径直接报错。
2. 先用 `resolve()` 选活动树：`rootfs-full -> rootfs -> 明确错误`。
3. 再拼到活动树根下。
4. 所有硬件桥只保留 `run/husk/...` 和 `tmp/...` 这类 guest 相对路径，不再知道安装树名字。

`tools/ci/test-guest-rootfs-selection.sh` 增加反向断言：除选择器外，`app-rs/src` 中再次出现
`.join("rootfs-full...")` 或 `const ... = "rootfs-full..."` 时直接失败。

## 4 · 修复后验证

### 4.1 静态与打包

```text
rootfs selector: 4 tests passed
make check: exit 0
app-rs aarch64-linux-android check: exit 0
make apk: BUILD SUCCESSFUL
APK versionCode=36 versionName=0.0.36
adb install -r（未使用 -g）: Success
```

### 4.2 真机硬件门禁

```text
A-hw: PASS=15 FAIL=0
AUDIO_AUDIT: PASS=4 FAIL=0
camera fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

音频延迟仍照实记录：

```text
直接 AAudio: avg=125.163 ms, max=128.000 ms, perf=10 (NONE)
paplay -> PulseAudio -> relay: avg=104.680 ms, max=106.667 ms, perf=10 (NONE)
```

没有独立人耳听音，因此音质/可听性仍是 **UNVERIFIED**。

### 4.3 完整门禁

```text
A-core: PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: 1085 candidates, 0 leaks
dpkg/V4L2/DRM/fd identity: PASS
supplementary-groups: PASS=1 FAIL=0
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
session guard: PASS
rootfd-close-range: stat_ok=8 range_ok=500
path-race: toggles=60000 operations=212050
final sentinel: unchanged
```

GPU 读回仍通过：

```text
READBACK_AUDIT PASS=6 FAIL=0
内部 19 项全部 PASS
renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

## 5 · 同轮暴露的发行阻塞

`make gpu-audit` 的第二段桌面全局继承失败：

```text
== existing desktop shell ==
  FAIL  gnome-shell process not readable (pid=missing)
== loader-default GL client ==
Error: unable to open display :0
GPU_GLOBAL_AUDIT PASS=0 FAIL=3 SKIP=0
```

原因不是 H35，而是本应用当前没有自己的 X server。PRoot 的 X server 属于另一个 Android uid，
不能借用。详见 evidence 349 和缺陷 H36。这个结果意味着当前不能宣称“桌面已全局吃到 GPU”，
也不能凭这版直接发 Release。
