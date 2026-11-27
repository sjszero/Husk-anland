# 343 · 相机门禁相对路径与失败清理

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：相机门禁把仓库相对路径 `ROOTFS=files/rootfs-full`、`LD=files/husk-dist-current/libld.so` 原样送进 `adb shell` 后，guest 找不到 loader，报 `files/husk-dist-current/libld.so: inaccessible or not found`。
- **FACT**：同一负向用例在 guest 启动失败后仍无条件等待 fixture host，测试进程不会自行结束；本次人工中断，退出码 130。
- **FACT**：修正 `files/*` 到 App 私有目录的路径映射，并在 guest 非零退出时终止 fixture host 后，相同相对参数得到完整 PASS。
- **FACT**：复跑覆盖 640×480 raw、1280×720 raw、640×480 GStreamer 3 帧、1280×720 GStreamer 3 帧，全部通过。
- **UNVERIFIED**：本轮不重新证明 Camera2 权限、真实前后摄、宿主 X11 或桌面呈现链路。

## 1 · H29 修前负向结果

命令：

```sh
ROOTFS=files/rootfs-full \
LD=files/husk-dist-current/libld.so \
sh tools/device-tests/run-camera-fixture.sh UKHMMB8PYTGECMNV
```

实际失败：

```text
/data/local/tmp/husk-session.sh[274]: files/husk-dist-current/libld.so: inaccessible or not found
```

根因：仓库相对路径属于 macOS 工作树，不是 `/data/data/com.linuxhub.husk` 下的路径；测试脚本没有在跨
`adb shell` 边界前做映射。

## 2 · H30 修前负向结果

上面的 guest 启动失败后，脚本没有进入清理分支，而是继续执行：

```sh
wait "$p"
```

fixture host 仍等待客户端连接，因此负向测试挂起。继续等待 60 秒仍无输出，人工发送 Ctrl-C，进程退出 130。
这说明测试失败可能表现为“卡死”，而不是明确 FAIL。

## 3 · 修复

文件：`tools/device-tests/run-camera-fixture.sh`

- 新增 `device_path()`：把 `files/*` 映射为 `/data/data/$PKG/files/*`，绝对设备路径保持不变。
- `ROOTFS` 和 `LD` 在第一次 `adb shell` 前统一规范化。
- `run_raw()` 在 guest 返回非零时先 `kill $p`，随后有界执行 `wait $p`，避免 fixture host 永久等待。

## 4 · 修后正向结果

使用与修前完全相同的相对参数：

```text
v4l2test|done|frames=10|bytes=4608000|fps=86.94
v4l2test|done|frames=10|bytes=13824000|fps=39.36
GST_AUDIT|PASS|640x480|frames=3
GST_AUDIT|PASS|1280x720|frames=3
camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

命令正常退出，未再出现路径错误或挂起。

## 5 · 状态

- H29：FIXED（测试契约）。
- H30：FIXED（测试契约）。
- 本轮改动只影响设备测试入口，不改变 Husk 运行时、GPU 能力或发行边界。
- 版本保持 `0.0.36`，不构成发布。
