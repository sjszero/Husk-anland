# 341 · 运行时最终回归：loader 同步与 syslog 测试契约

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：`make check` 通过。
- **FACT**：`make server-parity` 修正测试断言后为 `PASS=82 PARTIAL=5 FAIL=2 UNKNOWN=1`。
- **FACT**：门禁 A 修正设备 loader 后为 `PASS=38 FAIL=0 SKIP=0`；后续 coverage、V4L2、DRM、fd、身份、groups、rootfd、path race 均通过。
- **FACT**：当前 loader 连续两次构建哈希一致：
  `67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d`。
- **FACT**：剩余 `server-parity` 失败仍是 `/sys/fs/cgroup` 列表被 Android 拒绝，以及 `/dev/full` 不在当前设备节点实现范围；不是本轮新引入的回归。
- **UNVERIFIED**：从 APK 全新解压或全新安装时自动把 loader 同步到 `files/linux/husk/libld.so` 的路径，本轮只验证了现有设备树的显式同步。

## 1 · 第一次门禁 A：真实失败

命令：

```sh
DEV=UKHMMB8PYTGECMNV ROOTFS=files/rootfs-full \
  LD=files/linux/husk/libld.so make gate-a
```

前 38 项验收已经通过：

```text
PASS=38 FAIL=0 SKIP=0
```

随后

```text
--- loader mismatch: refusing to run
    device /data/data/com.linuxhub.husk/files/linux/husk/libld.so
      sha256=69d591b5baef4ba69a257bf59d0b53b75f1f00aee6ce5c8b705e69cb33ff951e
    built  /Users/l/Documents/Codex/2026-08-10/husk/tools/loader/ld-embedded.so
      sha256=67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d
```

结论：这是设备运行副本过期。APK 内 `lib/arm64-v8a/libld.so` 已经是 `67f543…`，但设备数据目录仍保存旧 loader。门禁拒绝继续是对的，不能把它当成通过。

## 2 · 修复与正向结果

保留旧 loader 为 `files/linux/husk/libld.so.before-67f`，将当前构建同步到运行路径后：

```text
67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d  files/linux/husk/libld.so
```

门禁 A 复跑：

```text
PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: PASS（没有发现未翻译的 guest 绝对路径）
DPKG_CONFFILE_MODE|PASS
V4L2_REF|PASS
drm-fake|verdict|PASS
fd-identity-probe: PASS
IDENTITY_RACE rounds=300 fails=0
GROUPS_RACE rounds=20 fails=0
session-guard-probe: PASS
ROOTFD_RACE stat_ok=8 stat_fail=0 range_ok=500 range_fail=0
PATH_RACE_DONE
```

## 3 · server-parity：两个真实问题和两个宿主边界

### 3.1 `/dev/log` 与 syslog

新增的静态 `husk-logd` 启动后，`/dev/log`、`syslog` 和 `syslog_libc` 都有真实日志落地。完整日志中的格式是：

```text
parity[18541]: syslog3 probe
```

修前测试断言查找 `parity: syslog3 probe`，因此没有匹配；这不是 `syslog(3)` 没写入。修后改用每次运行唯一的消息，并给 datagram 最多 3 秒落地窗口。

修前汇总：

```text
PASS=81 PARTIAL=5 FAIL=3 UNKNOWN=1
```

修后汇总：

```text
PASS=82 PARTIAL=5 FAIL=2 UNKNOWN=1
```

已转绿项：`/dev/log`、`syslog`、`syslog_libc`、`/proc/version`、`/sys/class/net`、`getent_dns`。

### 3.2 仍保留的失败

```text
R|sys|cgroup|FAIL|cgroup tree (listing denied => no limits)
R|dev|/dev/full|FAIL|missing
```

- `/sys/fs/cgroup`：Android 普通 App 域不允许列目录，不能伪造成真实 cgroup 限制视图。
- `/dev/full`：需要额外拦截 `write(2)` 才能实现 `ENOSPC` 语义，当前明确不属于设备节点最小拦截范围；继续保留 FAIL，不改成 PASS。

此外保留的 `PARTIAL` 项是 `setuid`、`hardlink`、`icmp`、`systemd`、`su`；`ld_cache` 为 `UNKNOWN`，不会把环境跳过说成已验证。

## 4 · loader 可复现性

使用固定 NDK 和 guest sysroot 连续构建两次：

```text
67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d  tools/loader/ld-embedded.so
67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d  tools/loader/ld-embedded.so
```

## 5 · 范围

本轮没有修改 X11、Wayland、宿主 App 的 X server/输入/Surface，也没有把桌面塞回 Husk APK。Husk 仍只交付 Linux ARM64 运行时资产。
