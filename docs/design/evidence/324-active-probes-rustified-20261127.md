# 324 · 当前门禁里的 Python 探针改为 Rust

日期：2026-11-27（CST）
基线：`ec72826`
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页

## 1. 修前状态（FACT）

基线仓库有 7 个 Python 文件。其中三个仍被当前门禁直接调用：

- `tools/device-tests/probes/readback.py` 被 `18-gpu-loader-defaults.sh` 调用；
- `tools/probe/audio-capture-probe.py` 被 `17-gate-a-hw.sh` 调用；
- 其余为 GPU 长稳、读回套件和一次性下载/并发诊断脚本。

这不是运行时语言混杂：核心运行时是 Rust。问题在于当前门禁本身仍依赖 Python，
开发机上缺 `python3` 时门禁无法独立复现。

## 2. 本轮改动（FACT）

### 2.1 A-hw 麦克风探针

新增 Rust crate `runtime/crates/audio-capture-probe`，实现同一套协议：

```text
PING -> PONG
OPEN rate=48000 channels=2 -> CAPTURE_READY
READ frames=1024 -> PCM_OK + PCM16LE
CLOSE -> CAPTURE_OK
```

它保留原来的 PASS 判据：请求的 PCM 字节必须真实返回，`peak` 和
`silent_chunks` 只作为样本证据。并发入口 `--concurrency` 合并进同一个 Rust
binary，检查一个空闲客户端不会把第二个 `PING` 堵住。

`tools/device-tests/17-gate-a-hw.sh` 改为交叉编译该 binary，通过 loader 直接执行，
不再调用 guest 内的 `python3`。

### 2.2 GPU loader 门禁

`tools/device-tests/18-gpu-loader-defaults.sh` 改用已存在的 Rust `gl-probe readback`。
判据仍是 renderer 字符串和两个反向控制，不改变 GPU 默认值语义。

### 2.3 删除不再需要的脚本

- `tools/probe/audio-capture-probe.py`：由 Rust 探针替代。
- `tools/probe/audio-bridge-concurrency.py`：由 Rust `--concurrency` 替代。
- `tools/device-tests/probes/readback.py`：由 `gl-probe readback` 替代。
- `tools/guest/fetch-debs.py`：仓库内没有调用点；rootfs 的包解析/下载已由
  Rust `nr-tools rootfs` 承担。

## 3. 真机正向结果（FACT）

A-hw：

```text
CAPTURE_OK frames_read=50176 silent_chunks=5 peak=266
capture-probe|PASS|chunks=49|frames=50176|bytes=200704|silent_chunks=5|peak=266|elapsed_ms=1079
PASS  microphone returned 200704 PCM bytes (peak=266)
PASS  gpsd delivered a last-known fix
PASS=14 FAIL=0
```

音频并发：

```text
SKIP  /run/husk/audio-out.sock: No such file or directory (os error 2)
PASS  /run/husk/audio-in.sock: second client answered while the first was idle
audio-bridge: PASS=1 FAIL=0
```

`audio-out.sock` 当时未创建，因此按 SKIP 记录，不计 PASS。

GPU loader 默认值：

```text
STATE plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
STATE off renderer: llvmpipe (LLVM 20.1.2, 128 bits)
STATE soft renderer: llvmpipe (LLVM 20.1.2, 128 bits)
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

## 4. 回归（FACT）

```text
cargo test -p audio-capture-probe                2/2 PASS
sh -n tools/device-tests/17-gate-a-hw.sh         PASS
sh -n tools/device-tests/18-gpu-loader-defaults.sh PASS
make check                                       PASS
make docs                                        PASS（351 个 Markdown，0 断链）
git diff --check                                 PASS
```

## 5. 后续迁移和剩余边界（FACT）

本轮随后把剩余活跃探针也合并到 Rust，当前受 Git 跟踪的 `.c` / `.py` / `.js`
文件数均为 **0**：

```text
git ls-files '*.c' '*.py' '*.js' | wc -l   # 0
```

- 7 个 Python 文件全部移除；`fixedgl.py`、`glsoak.py`、`readback-suite.py` 的能力
  并入 Rust `gl-probe`，`readback.py` 使用 `gl-probe readback`。
- 15 个 C 文件共 1456 行被移除。其中 6 个活跃探针（881 行）迁入
  `runtime-probe`，9 个已经结案的 X11 一次性诊断文件（575 行）直接删除，历史
  证据保留。
- `husk-wallpaper-scale.js` 迁入 Rust `wallpaper-scale`。

**这不代表 Rust 行数更少。** 当时看到的“删除约 1457 行、增加约 643 行”不是完整
diff：新 crate 还是 untracked，`git diff --stat` 不计入它们的行数。实际新增 Rust
代码约 1993 行（仅三个新 crate）再加 `gl-probe` 的约 781 行；当前完整工作树的
已跟踪统计是 `+948 / -3473`，仍未包含这些 untracked crate。代码量增加不等于
质量下降，也不能反过来用“行数变少”宣布迁移完成。

## 6. 尚未消除的边界（UNVERIFIED / FACT）

- Shell 脚本内仍有 `python3 -c` 或 Python heredoc（例如
  `05-mount-audit.sh`、`coverage-sweep.sh`、部分 bench/device harness）。因此
  “受跟踪扩展名为 0”不等于“运行时和门禁已完全无 Python 依赖”。
- `wallpaper-scale` 已在真机验证：320x240 输出成功，已经是目标尺寸时退出 3，
  坏图片和不可写目标均返回 1；桌面完整链路仍需在后续轮次回归。
- 该证据不宣布第 3 轮完成，也不进入版本发布；路径语义的正向/负向结果见 evidence 325。
