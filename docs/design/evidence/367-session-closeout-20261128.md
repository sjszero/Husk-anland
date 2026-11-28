# 367 — 0.0.37 收尾记录（2026-11-28）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB
测试 App：`com.linuxhub.myth.husk`（另一包 `com.linuxhub.husk` 不是本轮对象）
硬约束：不 commit / 不 push / 不 tag；只改 Husk 仓库；另一仓库只当黑盒宿主。

## 1. 本轮代码与资产改动（全部未提交）

| 文件 | 改动 | 状态 |
|---|---|---|
| `app-rs/src/video/bridge.rs` | 待取帧队列满时改为背压（原来静默丢帧，丢掉的 PTS 变成永久 `FRAME_NONE`）；`FRAME` 命令支持 `wait_ms`，客人可短轮询 | 已编译、已上机 |
| `runtime/crates/va-driver/src/video_client.rs` | `Frame` 增加 `pts_us`；新增 `fetch_any()`（`FRAME pts=-1`）；`fetch_frame` 带 `wait_ms` | 已编译、已上机 |
| `runtime/crates/va-driver/src/lib.rs` | 取帧从"一次阻塞 5 s"改为短轮询泵队列；新增 `frames_by_pts` 缓存（上限 24 帧，按 PTS 淘汰最旧） | 已编译、已上机 |

设备上的运行身份，可自证：

```text
files/linux/runtime/husk-bridges                       f1e893ce99aa3481417ccc5e7407412838405afad4e0915e0ff258a039584ca3
rootfs/usr/lib/aarch64-linux-gnu/dri/husk_drv_video.so 64790599ee0242375b6a32ea1d8012c8d91eba1baa8aba9539814953bc9dd00f
```

回滚备份：`husk-bridges.bak-v1`、`husk_drv_video.so.bak-v1`。
`husk-bridges` 由 PID 12753 运行（旧 32538 已被新二进制替换后重启）。

## 2. 本轮实测结论

- **PASS 接口层**：`LIBVA_DRIVER_NAME=husk` + `DISPLAY=:0` 时 `libgstvaapi.so` 注册出 `vaapih264dec`；`vainfo` 报 H264 Main / High / ConstrainedBaseline 三个 VLD profile。
- **PASS 解码正确性**：1080p High 片源取出的 I420 帧与 ffmpeg 参考帧逐像素全等（mean abs diff = 0.0）。**"High profile 合成 CSD 错"的旧推断作废。**
- **PASS 720p Constrained Baseline**：90/90 帧，约 2 秒。
- **FAIL 1080p / High profile 连续解码**：12–25 帧后停摆，`FRAME_NONE`，`pending=0`，`frames=4`；logcat `c2.mtk.avc.decoder [in=4 pipeline=0 out=13]`。三个改动上机后仍未解决。
- **FACT 计数器**：`decoded=` 是死计数器（代码从不自增），不能当判据。

## 3. 0.0.37 收尾还差什么（按 TODO-0.0.37.md）

已完成（有证据）：GPU audit / readback 19 / GPU soak 4×300 s / A-hw 15 / 音频 4 / 相机 fixture 双档 / 安装器升级 + 运行身份 12 / ICD canonical 15 / Snapshot 媒体链 12 / 139 崩溃修复。

仍缺，按优先级：

1. **视频门禁**：720p 已通，**1080p 停摆未修**——发布门禁里"视频"一项只能写 FAIL。
2. **相机可见画面**：数据链通，取景黑屏（亮度 mean<1，场景本身近黑），需补光复测 or 标环境阻塞。
3. **桌面 20 次循环 / 强退无残留 / 输入（触摸板、候选框）**：未跑。
4. **交互卡顿定量**：未跑（X11 呈现/读回链路，另案）。
5. **文档一致**：VERSION / CHANGELOG / README / STATUS / LIMITATIONS / COMPATIBILITY / RESULTS 需同步本轮 FAIL 口径。
6. **发布包**：本轮 apt 装的 `gstreamer1.0-vaapi` / `vainfo` / `plugins-bad` 只在设备 rootfs 里，**没有进 dist tar，也没有进 App assets**——发版前必须进包，否则等于发一个跑不了的运行时。

## 4. 口径

- 不把"驱动能加载 / element 能注册 / 编译通过"写成"视频播放通过"。
- 不利数字照写：停摆帧数、超时秒数、`[in=4 out=13]` 原样保留。
- 1080p 那条现在是 **FAIL**，不是 UNKNOWN——它有实测数据。

## 5. 0.0.37 装机自证与文档口径（2026-11-28 追加）

- `dist/husk-arm64.tar.gz`（`34ddae50…`）在 `com.linuxhub.myth.husk` 上 `--upgrade` 部署：
  `RUNTIME_IDENTITY PASS=12 FAIL=0`、`ICD_CANONICAL PASS=15 FAIL=0`、
  `READBACK_AUDIT PASS=19 FAIL=0`、`GPU_GLOBAL_AUDIT PASS=3 FAIL=0`。
- A-hw 已在本包上复跑：`PASS=15 FAIL=0`。Camera/Mic/Location 权限均 granted 非 USER_FIXED、
  Camera2 真出帧、五类 IIO 传感器三视图可读、麦克风 200704 B PCM（peak=1891）、
  gpsd 给出 last-known fix（mode=3）、无崩溃记录。只跑门禁，未改相机代码。
- 文档口径：`README.md` 顶部新增「状态校正（2026-11-28）」；硬件表相机/视频两行按实测改写；
  性能表标注旧宿主、新宿主未重测前不承诺；FAQ 实测条件改为新宿主；交付段落写明
  X server 与呈现归宿主 App。`docs/STATUS.md`、`docs/LIMITATIONS.md`、`docs/RESULTS.md`、
  `CHANGELOG.md` 同步同一口径。`make version-check` → `version ok: 0.0.37`。
- `make release-audit` 仍有一条固定红：`release naming audit (tags=36 releases=36 version=0.0.37)`，
  原因是"不 commit/push/tag"约束下不可能有 v0.0.37 的 tag 与 Release。这是约束的必然结果，
  不是缺陷；发版前需要在发布说明里写明。

## 6. 旧宿主层残留清扫（2026-11-28）

把"还能被当成现行说明"的旧宿主引用全部改掉，历史记录按"旧宿主现场"标注保留：

| 文件 | 改动 |
|---|---|
| `README.md` | 整篇重写（314 行）：新增「唯一有效宿主」「现在能做什么」；硬件表相机/视频两行按实测改写；性能表标旧宿主；交付段落写明 X server 归宿主 App |
| `CONTRIBUTING.md` | 活动包名改为 `com.linuxhub.myth.husk`，旧包名只许留在 evidence 与 CHANGELOG 历史段 |
| `docs/GOALS.md` | 包名条目更新并注明旧宿主 |
| `docs/STATUS.md` | 头部加"宿主 App / 唯一有效宿主"，最后核验改 2026-11-28 |
| `docs/BENCHMARKS.md` | 顶部加口径块：数字采于旧宿主，重测前不承诺；复现需 `PKG=com.linuxhub.myth.husk` |
| `docs/design/lifecycle.md`、`docs/design/device-nodes.md`、`docs/design/audio-bridge.md` | 可执行示例里的包名与安装树路径改到宿主 App |
| `tools/device-tests/README.md` | 两条命令示例改到宿主 App 与新安装树路径 |

有意保留 `com.linuxhub.husk` 的地方只有三类：不可改写的 `docs/design/evidence/` 原始输出、
`CHANGELOG.md` 历史段、以及 README/GOALS 里"旧宿主作废"的说明本身。

未改的一类（并说明原因）：`tools/bench/*`、`tools/device-tests/*` 的 `PKG` 默认值仍是
`com.linuxhub.husk`——那是本仓库自带终端 APK 的包名，自包含探针要靠它跑；在宿主 App 上复现
用 `PKG=com.linuxhub.myth.husk` 覆盖。`tools/ci/audit-runtime-package.sh` 把该默认值钉住，
改默认值必须同时改审计与 APK 工程，属于包名迁移，不在 0.0.37 范围内。
