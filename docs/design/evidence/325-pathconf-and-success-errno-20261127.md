# 325 · pathconf / statvfs 与成功路径 errno 语义

日期：2026-11-27（CST）
基线：`ec72826`
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页
版本：`0.0.36`（未发版）

## 0 · 结论（FACT）

`pathconf(3)` 是 glibc 内部的路径查询包装，调用点没有经过可改写的 GOT/PLT 槽。
只修路径翻译还不够：成功路径上 `target_fd()` 的探测会留下 ENOENT，调用方看到
正确返回值却读到错误的 errno。现在 loader 同时导出 `pathconf`、用 pinned
`statfs` 翻译 guest 路径，并在成功返回前恢复进入函数时的 errno。

同一类问题也出现在 `statvfs`、`openat` 和 `access` 的成功路径。`openat`/`access`
现在保存入口 errno，仅在 `r >= 0` 时恢复；失败仍然保留真实错误。

## 1 · 修前负向结果（FACT）

旧 loader 没有 `pathconf` 导出：

```text
PATHCONF v=-1 errno=2
```

加入路径翻译但尚未恢复成功 errno 时：

```text
PATHCONF v=255 errno=2
FREOPEN ok=1 errno=2
CREATE_MODE open_fd=3 errno=2
OWNER_RESTORE open_fd=4 errno=2
```

返回值本身正确，但成功路径携带了残留 ENOENT。探针在每次调用前清零 errno，
因此这不是调用方预先设置的旧值。

## 2 · 修复（FACT）

- `runtime/crates/path-shim/src/lib.rs`
  - 新导出 `pathconf`，用 `xattr_pinned + SYS_STATFS` 翻译路径；
  - `_PC_NAME_MAX` 返回 guest 文件系统的 `f_namelen`（本机为 255）；
  - 成功返回时恢复入口 errno；未知 name 仍返回 `EINVAL(22)`。
- `statvfs_common` 成功返回时恢复入口 errno。
- `openat` 和 `access` 的外层入口保存/恢复成功路径 errno。
- `tools/probe/run-coverage-probe.sh` 把成功 errno 收紧为断言，不再只检查返回值。

## 3 · 修后正向结果（FACT）

同一设备、同一 rootfs、新 loader：

```text
FREOPEN ok=1 errno=0 exists=1
PATHCONF v=255 errno=0
STATVFS rc=0 errno=0 bsize=4096 namemax=255
CREATE_MODE open_fd=3 errno=0 mode=440 write=1 expected=440
OWNER_RESTORE open_fd=4 errno=0 mode_open=440 write=1 mode_final=440
ACCESS_ROOT read=0/0 write=0/0 exec0=-1/13 exec5=0/0
ACCESS_USER parent=-1/13 owned=0/0
--- coverage-probe: PASS
```

## 4 · 回归（FACT）

```text
run-coverage-probe.sh                          PASS（含新增 PATHCONF/STATVFS 断言）
run-fd-identity-probe.sh                       21/21 listed checks PASS
05-mount-audit.sh                              PASS=18 FAIL=0
groups-race                                    rounds=20 fails=0
identity-init-race                             rounds=50 fails=0
path-race                                      toggles=60000 operations=212050 successes=113522
rootfd-close-range                             range_ok=500 range_fail=0
cargo test -p husk-fd-identity -p usermgr      5 + 6 tests PASS
make check                                     PASS
wallpaper-scale                                320x240 output OK，exact exit=3，坏图/写失败 exit=1
git diff --check                               PASS
```

`make check` 覆盖 fmt、`cargo check --workspace`、可运行单测、clippy、shell 语法、
文档链接和版本契约；它不等于真机兼容性审计。

## 5 · 边界与后续（UNVERIFIED）

- `FACT`：raw syscall 的 `RAW_OPENAT fd=-1 errno=2` 仍是设计边界；GOT 改写不能
  拦截未经过 loader 的调用。
- `FACT`：`symbol-gap` 中 `chroot` / `umount` / `swapon` / `swapoff` 仍是
  `EPERM/EINVAL`，因为这些宿主权限没有授予，不能伪装成成功。
- `UNVERIFIED`：本次只验证了参考机；其他 Android 版本、页大小、GPU 家族和
  rootfs 仍需分别跑门禁。
- `UNVERIFIED`：Shell 内嵌 Python harness 尚未完全迁出，不能把扩展名清零宣传成
  “无 Python 依赖”。
