# 326 · 第 3 轮路径深度、小栈线程与 dirfd 真机收口

日期：2026-11-27（CST）
基线：`bc96f82`
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页
版本：`0.0.36`（未发版）

## 0 · 结论（FACT）

当前 loader 在真机上满足以下路径语义：

- 16、17、40 层符号链接可解析；41 层返回 `ELOOP`，行为与 Linux 的
  `MAXSYMLINKS=40` 一致；
- 40 层中间路径在 64 KiB guest 线程栈上仍可解析，没有把旧的递归栈溢出
  当作“提高深度上限”的代价；
- `openat` / `fstatat` / `faccessat` / `mkdirat` / `readlinkat` /
  `linkat` / `renameat2` / `fchmodat` / `utimensat` 的相对 dirfd 与
  `/proc/self/fd` 边界用例通过；
- 路径解析结果仍被约束在 guest rootfd 内，绝对链接和 `..` 没有逃到宿主。

本次只证明参考设备与当前加载器；不把它扩大成所有 Android 版本、页大小和
GPU 家族的兼容性结论。

## 1 · 修前负向结果（FACT）

先跑原始路径门禁，深度和 dirfd 用例本身全过，但一条测试把所有结果拖红：

```text
PASS depth-final-16 got=true want=true
PASS depth-mid-40 got=true want=true
PASS depth-final-41 got=false want=false
PASS depth-mid-41 got=false want=false
PASS depth-40-on-64k-stack got=true errno=Some(0)
PASS faccessat-nofollow
FAIL symlink-follow -- got [PRETTY_NAME="Ubuntu 24.04.5 LTS"] want [PRETTY_NAME="Ubuntu 24.04.4 LTS"]
#checks=13 fail=1
```

这是测试把发行版点版本写死造成的误报，不是路径解析失败。修复前后的同一条
链接都解析到了正确的 guest `/etc/os-release`。

## 2 · 修复（FACT）

`tools/device-tests/04-path-reliability.sh` 不再硬编码 Ubuntu 点版本：

```sh
guest_os=$(head -1 /etc/os-release)
chk symlink-follow "$(head -1 /tmp/husk-rel/abs)" "$guest_os"
```

这样测试验证的是“符号链接经过 guest root 解析到 guest 的 os-release”，而不是
绑定某一个发行版补丁版本。

## 3 · 修后正向结果（FACT）

同一设备、同一 rootfs、同一 loader 重新构建并安装 APK 后：

```text
local loader sha256 = 69d591b5baef4ba69a257bf59d0b53b75f1f00aee6ce5c8b705e69cb33ff951e
depth-final-16/17/40   PASS
depth-mid-16/17/40     PASS
depth-final-41         PASS (ELOOP expected)
depth-mid-41           PASS (ELOOP expected)
depth-40-on-64k-stack  PASS errno=0
depth-40-on-128k-stack PASS errno=0
depth-40-on-256k-stack PASS errno=0
depth-40-on-1024k-stack PASS errno=0
#checks=20 fail=0
PASS faccessat-nofollow
PASS relative-dirfd-contained
#checks=31 fail=0
#checks=15 fail=0
```

本机设备上的 loader 两份入口文件指纹一致：

```text
69d591b5baef4ba69a257bf59d0b53b75f1f00aee6ce5c8b705e69cb33ff951e  files/linux/libld.so
69d591b5baef4ba69a257bf59d0b53b75f1f00aee6ce5c8b705e69cb33ff951e  files/linux/husk/libld.so
```

## 4 · 回归和构建（FACT）

```text
make apk                                      PASS
PATH probe source                            tools/device-tests/path-probe.rs
DIRFD probe source                           tools/device-tests/dirfd-probe.rs
run-coverage-probe.sh                        PASS
run-fd-identity-probe.sh                     21/21 listed checks PASS
05-mount-audit.sh                            PASS=18 FAIL=0
groups-race                                  rounds=20 fails=0
identity-init-race                           rounds=50 fails=0
path-race                                    toggles=60000 operations=212050 successes=113978
rootfd-close-range                           stat_ok=8 stat_fail=0 access_ok=8 access_fail=0 range_ok=500 range_fail=0
make check                                   PASS
```

探针构建输入来自当前 guest rootfs 的 `libc.so.6`，执行时通过当前 loader 的
GOT interpose，不绕过被测路径解析层。

第二组回归的第一次 `path-race` 没有进入探针：脚本把 Makefile 的相对路径
`files/linux/libld.so` 原样交给 `husk-session.sh`，设备侧报：

```text
/data/local/tmp/husk-session.sh[259]: files/linux/libld.so: inaccessible or not found
--- path-race-probe: execution failed
```

`run-path-race-probe.sh` 已补上与同组探针一致的 `device_path()` 规范化，复验为：

```text
PASS host-sentinel-not-visible errno=2
STATS toggles=60000 operations=212050 successes=113978
PATH_RACE_DONE
PASS host sentinel unchanged
```

## 5 · 边界（UNVERIFIED / FACT）

- `FACT`：raw syscall、静态 ELF 和不经过 loader 的进程仍可能绕过 GOT 改写；
  Path Shim 不是恶意代码安全沙箱。
- `UNVERIFIED`：本次只覆盖 RMX3357 / Android 13 / 4 KiB 页；其他内核、页大小、
  文件系统和设备仍需分别跑同一组门禁。
- `UNVERIFIED`：完整第 3 轮仍要把 fd 身份、SCM_RIGHTS、usermgr 事务和只读挂载
  的现有证据与新 loader 一起纳入发布前回归。
