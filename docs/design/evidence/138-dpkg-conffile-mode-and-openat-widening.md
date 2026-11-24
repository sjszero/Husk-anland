# 138 · dpkg 拆 conffile：`先 chmod 再写` 在假 uid 下必 EACCES，修在 `openat`

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
rootfs：App 自己从 `assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar` 解出来的那棵
（`distributions/ubuntu-24.04/rootfs`），loader `33dcd681…`。

## 0 · 结论（FACT）

137 修完两条根因后，App 流程走到 `CONFIGURE_SUDO`，死在：

```text
dpkg: error processing archive /tmp/sudo.deb (--install):
 unable to open '/etc/sudoers.d/README.dpkg-new': Permission denied
```

根因不是路径层，也不是 seccomp：**dpkg 拆包时先按包里的模式 `chmod`，再用同一个名字
重新 `open(O_WRONLY)` 写内容**。sudo 的 `/etc/sudoers.d/README` 是 `0440`（owner 无写位）。
我们报给 guest 的 uid 是 0，但内核看到的是真实 app uid（`u0_a265`），`0440` 的 owner 位
对它就是只读——于是 EACCES。真 root 不受 owner 位约束，所以这个模式在 proot/chroot 里
根本不会出现。

修法（`runtime/crates/path-shim/src/lib.rs`，两处，缺一不可）：

1. `openat_inner()`：`O_CREAT` 时 `mode | 0o600`。我们自己建的文件永远保留 owner 写位，
   后续同 uid 的写不会被自己刚设的位挡住。
2. `openat()` 外层：写意图 + `EACCES` + `NR_FAKE_UID=0` → `widen_owner_write()`
   把目标 `chmod 0600` 后**重试一次**。这条覆盖「文件已经在、模式是 0440」的情况，
   也就是 dpkg 的实际顺序：create → write → chmod(0440) → **再 open 写**。

顺带（诊断用）：`openat` 失败且 `NR_DEBUG` 开着时，把 `errno/flags/path` 记进
`<rootfs>/tmp/nr-exec.log`。dpkg 自己的报错只说 "Permission denied"，不给 flag 字，
这一行是唯一能定位到具体 open 的证据来源。widen 成功时也留一行。

## 1 · 为什么 1 不够、2 不能省

只加 1（`mode | 0o600`）时仍失败：dpkg 在内容写完之后把文件 `chmod` 回包里的 `0440`，
然后**重新** `open` 它（dpkg 自己的 `--force-confold`/conffile 处理路径）。这一次
`O_CREAT` 不成立，`mode|0600` 那条分支根本不执行。

只加 2（EACCES 重试）也能成，但每次失败都要付一次 syscall + errno 往返，且
`O_EXCL` 的 create 语义下第一次写就已经失败；两条一起，正常序走 1、异常序走 2。

## 2 · 实测（同一棵 App 的树，`run-as` 同一 uid）

失败序（修之前，07:5x）：

```text
$ dpkg -i /tmp/sudo.deb
…
unable to open '/etc/sudoers.d/README.dpkg-new': Permission denied
$ echo $?      → 1
```

修之后（`NR_DEBUG=1`，先 `dpkg --purge sudo` 再从同一个 deb 装）：

```text
$ dpkg --purge sudo   → purge_rc=0
$ dpkg -i /tmp/sudo.deb
openat widened owner write /etc/sudoers.d/README.dpkg-new
$ echo $?             → inst_rc=0
$ ls -l /usr/bin/sudo
-rwsr-xr-x 1 10265 husk10265 335120 Oct  6 13:51 /usr/bin/sudo     ← setuid 位保留
$ ls -l /etc/sudoers.d/
-rw------- 1 10265 husk10265 1068 Jan 29  2024 README
```

`nr-exec.log` 里整轮**只有一行** `widened`，就是那个 conffile；其它 `openat denied`
全是 `ENOENT`（`/var/lib/dpkg/arch`、`tmpfiles.d`）和 `/proc/sys/kernel/cap_last_cap`
的 `EACCES`（`/proc` 本来就不给改），都不是缺陷。

## 3 · 已知偏差（FACT，尚未修）

`/etc/sudoers.d/README` 最终是 `0600`，包里写的是 `0440`：`widen_owner_write()` 用的是
`chmod 0600`，把 group/other 位一起冲掉了。功能无影响（owner 就是我们自己，sudo 也不读
这个文件），但**模式保真度是缺陷**：chroot 里应该是 `0440`。干净的做法是记
(fd → 原模式)、`close` 时恢复；这需要 `close` interpose + 一张有界表，属于 137 §4
「静态/保真类别」的后续项，本轮不动，先保门禁 A。

## 4 · 一个方法论更正

137 记过「门禁结论只能在 App 进程里量」。**本文是例外，且要说清为什么**：这里的差异来自
`mode` 位对真实 uid 的判定，与 seccomp 过滤器无关，`run-as` 与 App 是同一个 uid，
结论等价。凡是涉及 seccomp / SIGSYS / 进程域的判断，仍然必须回到 App 进程里量。

## 5 · 指纹

```text
ld-embedded.so  33dcd681…（旧 7b4300a4…）
libpath_shim.so 06abee35…（旧 1b86173b…）
libc-nop.so.6   32d4d3f9…（不变）
```

`docs/ENGINEERING.md` 两个基线已同步。App 侧资产还需把 `libld.so` 换成 `33dcd681…`，
`ASSET_REVISION` 提到 `husk-runtime-spawn-link-7`（6 已被设备用过，不换会让
`ensureInstalled()` 跳过覆盖）。
