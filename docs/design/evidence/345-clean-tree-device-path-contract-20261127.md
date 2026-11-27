# 345 · 干净树设备路径契约（H34）

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：干净安装树 `files/clean-audio-344/*` 的主门禁已通过
  `PASS=34 FAIL=0 SKIP=1`，随后 coverage-probe 因路径契约失败。
- **FACT**：失败不是 loader、rootfs 或真实 Linux 系统调用失败；
  `device_path()` 只识别 `files/linux/*`、`files/rootfs-full/*`、`files/rootfs/*`，
  任意合法测试树名被原样传入 App 私有目录；coverage-probe 与 V4L2-ref 分别在
  `files/clean-audio-344/bundle/libld.so: inaccessible or not found` 处失败。
- **FIXED（静态契约，待门禁复跑）**：所有带 `device_path()` 的设备探针统一接受
  任意 `files/*`，并在跨越 `adb shell` 前映射到 `/data/data/$PKG/files/*`；
  `tools/ci/test-guest-rootfs-selection.sh` 增加反向检查，后续脚本再出现同类
  路径缺口会直接失败。

## 1 · 修前负向结果

复现命令：

```sh
make gate-a \
  DEV=UKHMMB8PYTGECMNV \
  PKG=com.linuxhub.husk \
  ROOTFS=files/clean-audio-344/root \
  LD=files/clean-audio-344/bundle/libld.so
```

核心门禁先通过：

```text
PASS=34 FAIL=0 SKIP=1
```

随后 coverage-probe 失败：

```text
--- coverage-probe: execution failed
husk-session.sh[274]: files/clean-audio-344/bundle/libld.so: inaccessible or not found
make: *** [gate-a] Error 1
```

修 H34 第一版后，coverage-probe 已通过；门禁继续到 V4L2-ref，又暴露同类入口：

```text
/data/local/tmp/husk-session.sh[274]: files/clean-audio-344/bundle/libld.so: inaccessible or not found
make: *** [gate-a] Error 127
```

原因是 `run-v4l2-ref-probe.sh` 使用手写 `case`，只识别 `rootfs-full` / `rootfs`，
没有进入第一版统一的 `device_path()`。这不是新根因，仍是 H34。

**INFERENCE**：`files/clean-audio-344/...` 被当成 `run-as` 当前目录下的相对路径，
而不是 App 数据目录下的绝对路径。该错误与 H29 属同一契约缺口，但触发路径不同，
因此追加 H34，不覆盖旧编号。

## 2 · 修复范围

统一规则：

```sh
case "$1" in
    /*) printf '%s\n' "$1" ;;
    files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
    files/*) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
    *) printf '%s\n' "$1" ;;
esac
```

覆盖：

```text
tools/probe/run-coverage-probe.sh
tools/probe/coverage-sweep.sh
tools/device-tests/run-dpkg-conffile-mode-probe.sh
tools/device-tests/run-drm-fd-probe.sh
tools/device-tests/run-egress-probe.sh
tools/device-tests/run-fd-identity-probe.sh
tools/device-tests/run-groups-race-probe.sh
tools/device-tests/run-identity-init-race-probe.sh
tools/device-tests/run-mount-audit.sh
tools/device-tests/run-path-race-probe.sh
tools/device-tests/run-rootfd-close-range-probe.sh
tools/device-tests/run-v4l2-ref-probe.sh
tools/device-tests/run-server-parity.sh
```

## 3 · 修后验证

**UNVERIFIED（提交时）**：完整 `make gate-a` 尚未在修复后的提交上复跑；结果将在
本文件后续追加，不能把静态检查当成真机通过。

