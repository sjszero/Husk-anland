# 287 · A-20：`umount` 在 guest 里不可用，挂载表只进不出

- 日期：2026-11-26
- 设备：RMX3357 / MT6893 / Android 13 / 4 KiB 页，`UKHMMB8PYTGECMNV`
- 根树：`files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126`（usrmerge 修好、带 python3）
- 缺陷归属：第 3 轮 · fd 与路径语义（挂载语义）

---

## 1 · 修前（FACT）

`05-mount-audit.sh` 15/3 → 16/2 一直是这两条：

```
FAIL  store directories left behind            (B-02)
FAIL  chain stopped after one hop              (B-04)
umount: /mnt/ro: must be superuser to unmount.
mount: /mnt/mid: mount(2) system call failed: No space left on device.
```

直接量到的状态：

| 量 | 值 |
|---|---|
| `rootfs/run/husk/t*` 存储目录 | **240 个** |
| `rootfs/run/husk/mounts` 表 | **26 条**（其中 10 条是更早的会话留下的死项） |
| 表容量 | 64 条（0.0.36 前是 8 条） |

`240` 个目录就是「只进不出」的物证：每次 tmpfs mount 建一个 store 目录，`umount` 从来不删它。

---

## 2 · 根因（FACT，不是推断）

上一轮记的是 UNKNOWN，猜的是 capabilities 或 `/run/mount/utab` 属主。**两个都错。**

决定性的一条：`must be superuser to unmount` 不是 util-linux 在做超级用户判断，它是
**`umount2(2)` 返回 EPERM 时 libmount 的人话文案**。util-linux v2.39.3
`libmount/src/context_umount.c:1322-1323`：

```c
case EPERM:
    snprintf(buf, bufsz, _("must be superuser to unmount"));
```

所以问题不在「谁有权限」，而是**这次 `umount2` 根本没走到 shim，是内核直接回的 EPERM**。

为什么：libmount 自带两条路（`do_umount()`）：

```c
if (flags || ...)  rc = umount2(target, flags);
else               rc = umount(target);          /* 无 flag 走这条 */
```

- `umount <dir>`（无 flag）→ 调 **`umount()`**（一个参数的旧接口）。
- `umount -l <dir>` → 调 `umount2()`。

glibc 把 `umount()` 展开成内联的 `umount2(2)` 系统调用
（`sysdeps/unix/sysv/linux/umount.c`：`INLINE_SYSCALL_CALL (umount2, special, 0)`），
**它不调用 `umount2` 符号**。而 shim 只 interpose 了 `umount2`：

```
$ grep 'b"umount' runtime/crates/path-shim/src/boot.rs
1110:    b"umount2",          <- 只有这一个
$ grep -n mount tools/loader/libc-interpose.txt
64:mount
65:umount2                     <- 只有这两个
```

于是普通用户空间最常走的 `umount <dir>` 是一次裸系统调用，被 `untrusted_app` 域的 seccomp
判 EPERM；表项没人删，store 目录没人删，攒到 64 条之后 B-04 的两次 mount 直接
「No space left on device」——**这是 shim 自己的表满，不是内核 ENOSPC**。

判别实验（同一棵树，修前）：

```
== mount tmpfs ==
mount rc=0
table: 27
== plain umount ==
umount: /mnt/t1: must be superuser to unmount.
umount rc=32
table after plain umount: 27        <- 一条没少
== umount -l ==
umount -l rc=0
table: 27 -> 26                     <- 走 umount2，成功
```

同一台机器、同一条路径，只差一个 flag，一个失败一个成功——这就是「走了不同 libc 入口」的指纹。

---

## 3 · 修法（FACT）

1. `runtime/crates/path-shim/src/lib.rs`：把 `umount2` 的表操作抽成 `mnt_drop_one(target, by_src)`，
   新增 `#[no_mangle] pub unsafe extern "C" fn umount(target)` 走同一段逻辑。
   `by_src` 只对旧接口开：`umount <device>` 从 V7 起就允许按源匹配，util-linux 现在仍靠它。
2. `runtime/crates/path-shim/src/boot.rs`：`BODY_PATCH` 加 `b"umount"`（99 → 100 条），
   这样 glibc 内部调用者与 `dlsym("umount")` 也落到 shim。
3. `tools/loader/libc-interpose.txt`：加 `umount`（106 条）。

重建（两条都得重，libc 的 slot 页与 loader 的 extern 表是同一个布局）：

```
make libc-patch STOCK=/tmp/libc.so.6.stock     # 101 of 106 hooks, 5 declared refusals
NDK=$HOME/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64 make loader
```

`make libc-verify STOCK=<stock>` → 「byte-identical to `tools/loader/libc-nop.so.6`」，
libc 与 loader 的 hook 数一致（101/106，5 条都是 `libc-interpose.expected` 里已声明的拒绝，
`umount` 不在拒绝里）。

---

## 4 · 修后（FACT，同一台设备）

### 4.1 单点复现脚本

```
== umount by mount point ==
mount rc=0   table 27 -> 28
umount rc=0
table 28 -> 27                      <- 修前 rc=32 且纹丝不动
== umount by source path ==
umount /run/husk/t30380392448  rc=0  -> table 0，store 目录已删
== umount of something not mounted ==
umount: /mnt/u1: not mounted.  rc=32  -> 表内容不变（不是假成功）
```

### 4.2 门禁脚本

| 脚本 | 修前 | 修后 |
|---|---|---|
| `05-mount-audit.sh` | 16 PASS / **2 FAIL** | **18 PASS / 0 FAIL** |
| 新 `16-umount-legacy.sh` | 不存在（`umount` 必红） | **12 PASS / 0 FAIL** |
| `14-gate-a-acceptance.sh`（`files/linux2` 全新装树） | 35/1 | **36 / 0** |
| `path-probe`（40 层符号链接、20 项） | 20/0 | **20 / 0**（重建 loader 后无回归） |
| `run-fd-identity-probe.sh` | PASS | **PASS** |
| `run-drm-fd-probe.sh` | PASS | **PASS** |

清理后状态：`stores=0 table=0`（240 → 0，26 → 0）。

门禁 A 那条 `installed packages 93 < 700` 是**门禁参数写错**：700 是完整桌面发行树的量，
`ubuntu-base` tarball 就只有 93 个包。改成按树给下限（base 默认 90，桌面树显式
`EXPECT_PKGS_MIN=700`），包数照原样打印。这条不属于 A-20，同日一起修的。

---

## 5 · 顺手修掉的两处「推荐的办法本身会把树弄坏」

`husk-session.sh` 发现 guest libc 与安装时记录的 sha 不一致就拒启动（这是对的：`apt upgrade`
换掉 libc 会让 `getent shadow`、`su`、fork 全坏）。但它的错误提示让人去跑
`make libc-patch`，而 `make libc-patch` 原来**只换 libc、不更新记录也不更新 rootfs 旁边的
那份 bundle**，于是照提示做完，树仍然拒启动：

```
husk-session: guest libc is 19554f2783e2 but this install recorded 69ceef3759f4,
              and no matching libc.so.6 sits next to the rootfs.
              ... Refusing to start a broken tree.
              (exit 74)
```

三处一起改（`Makefile`）：
- `libc-patch` 同步把新 libc 写到 `$(rootfs)/../libc.so.6`（会话的恢复来源）并改写
  `.husk-install-state` 里的 `libc_sha256`。
- 新增 `STOCK=<stock libc>`：安装器发的 libc 本身就是打过补丁的，拿它当输入会直接报
  `no clone3 site left`。
- 删掉「把当前 libc 存成 `.stock`」那一步——存的是**打过补丁的**副本，下次再拿它当原始
  输入必然失败。

实测 `make libc-patch STOCK=/tmp/libc.so.6.stock` 一条命令跑完
patch → 推送 → bundle 同步 → 记录 sha，之后 `husk-session` 正常放行。

---

## 6 · 仍然未修 / 未验证（照写）

| 项 | 状态 |
|---|---|
| `.husk-install-state` 里的 `loader_sha256` 在重装 loader 后仍是旧值 | **空转不一致**：目前 `husk-session` 只校验 libc，不校验 loader（FACT：读 `.husk-install-state` 见 `d93fe95…`，实际 `658f97e…`）。要么加校验，要么不记这个字段——待定 |
| 表里更早会话的死项 | 修好后不再增长，但**已经写进 `run/husk/mounts` 的死项不会自动回收**；本轮是手工清的（240 目录 + 26 条）。跨会话回收策略 UNKNOWN |
| `umount -l`（`MNT_DETACH`）语义 | shim 忽略 flags，等于立即卸载。真正的 lazy unmount（等最后一个用户走）没实现 |
| `move_mount` / `open_tree` / `mount_setattr` | 仍回 ENOSYS（`sigsys_answer` 表），util-linux 2.39 有走新接口的分支 |
| 跨设备 | 只有这一台（Android 13 / 4.19 / 4 KiB / Mali-G77）。16/64 KiB 页真机、Adreno、Android 14+ 全部 UNVERIFIED |
