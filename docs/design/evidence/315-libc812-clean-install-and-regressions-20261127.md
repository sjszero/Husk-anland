# 315 · libc 升级到 8.12、全新安装与门禁回归（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`
发行包：`dist/husk-arm64.tar.gz`
新安装树：`files/release-tree-libc812-20261127/root`

## 0 · 结论

- **FACT**：修前包数据库已经记录 `libc6 2.39-0ubuntu8.12`，但活动树与实际发行树的两份
  `libc.so.6` 都是 `19554f27...`，对应 8.5 输入的补丁产物。包记录和运行字节不一致。
- **FACT**：当前 Rust `nr-tools libc-patch` 对固定 8.12 stock 输入产出
  `2ed8771f220ba55ed0c224e114c83a30cb9d34fb333d375fc690f7267bd138bb`；
  101/106 个 hook 打上、5 个声明拒绝、`clone3` 1→0，4K/16K/64K 页均 congruent。
- **FACT**：用重建后的 bundle 做全新安装，安装器明确打印
  `tree libc 5a6548c51d04 != bundle 2ed8771f220b: restoring`，最终两份 libc 和
  `.husk-install-state` 都是 `2ed8771f...`。
- **FACT**：全新安装树 Gate A `PASS=34 FAIL=0 SKIP=1`，Tree Integrity
  `PASS=23 FAIL=0`，GPU defaults `PASS=9 FAIL=0 SKIP=0`。
- **FACT**：把 App 稳定入口 `files/rootfs-full` 切到新树并冷启动后，A-hw
  `PASS=11 FAIL=0`：Camera2 三帧，每帧 1,382,400 bytes；麦克风 196,608 PCM bytes，
  `peak=387`。
- **FACT**：同树读回 `PASS=19 FAIL=0`，mount audit `PASS=18 FAIL=0`，
  fd identity `21/21 PASS`。
- **UNVERIFIED**：Adreno/KGSL、16/64 KiB 真机和独立 X11 桌面不在本次范围；不能把本机
  Mali-G77 结果扩大成跨设备兼容声明。

## 1 · 固定输入与产物（FACT）

```text
libc6_2.39-0ubuntu8.12_arm64.deb
deb sha256      2578bc27d4e077f5e93c283d2502bc693c516d9590a20a27840b548a4d436820
stock libc      2544e7902ec1bd3a68b526b6af427c34d2a98e56b9dbf8395b4b498dd2112cd6
patched libc    2ed8771f220ba55ed0c224e114c83a30cb9d34fb333d375fc690f7267bd138bb
101/106 hooks, 5 declared refusals
clone3 1 -> 0, set_robust_list 2 -> 0
```

重建 bundle：

```text
dist/husk-arm64.tar.gz  375a4a65cdffa384a5f1d1bed5eefc83cb49dd78d711ee34aa804e8c2c981359
dist/libld.so           96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b
```

## 2 · 全新安装（FACT）

从 `release-tree-v2.tar.gz` 解到空目标，使用新 bundle 的安装器：

```text
== libc ==
   tree libc 5a6548c51d04 != bundle 2ed8771f220b: restoring
   libc 2ed8771f220b matches the bundle
== done ==
```

安装后：

```text
usr/lib/aarch64-linux-gnu/libc.so.6  2ed8771f...
lib/aarch64-linux-gnu/libc.so.6      2ed8771f...
.husk-install-state libc_sha256=2ed8771f...
dpkg Version: 2.39-0ubuntu8.12
```

## 3 · 回归结果（FACT）

```text
Gate A                 PASS=34 FAIL=0 SKIP=1
A-hw                   PASS=11 FAIL=0
GPU loader defaults    PASS=9  FAIL=0 SKIP=0
Tree integrity         PASS=23 FAIL=0
Readback suite         PASS=19 FAIL=0
Mount audit            PASS=18 FAIL=0
fd identity            21/21 PASS
```

A-hw 关键日志：

```text
captured 3 frame(s) through Camera2 (1382400 bytes each)
capture-probe|PASS|chunks=48|frames=49152|bytes=196608|silent_chunks=1|peak=387|elapsed_ms=1075
PASS=11 FAIL=0
```

## 4 · 顺带修复 H12（FACT）

`tools/device-tests/16-umount-legacy.sh` 依赖 `awk`，而有效发行树可以只有 `mawk` 而没有
`/usr/bin/awk` 链接。修前：

```text
awk: command not found
PASS=7 FAIL=5
```

改为纯 POSIX shell 的 tab 字段解析后，同一棵树：

```text
PASS=12 FAIL=0
```

这不是 umount 运行时缺陷；它是门禁测试的依赖误报。

## 5 · 边界

- S6 的 `ENOSYS(38)` / `EOPNOTSUPP(95)` 统一仍按原计划暂存，未混入本提交；fd identity
  本次只确认 21 项现有断言通过。
- 本次不修改独立 X11 桌面的宿主边界；该能力仍按 evidence 312 的结论单独验收。
- 本次不以单机回归替代 Adreno、16/64 KiB 页和其它 Android 版本的验证。
