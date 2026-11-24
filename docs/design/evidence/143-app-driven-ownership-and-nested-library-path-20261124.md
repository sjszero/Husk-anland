# 143 · 带虚构所有权的运行时，按 App 的 UI 从头初始化两遍

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`。
两遍都是**空的 checkpoint**（上一棵树 `mv` 成 `distributions.attempt5/6`，去掉
`container.properties`），全部通过 App 自己的界面点下去，没有 adb 插手初始化本身。

## 0 · 结论（FACT）

```text
第 1 遍（loader d210644d，evidence 142 的产物）
START EXTRACT_SYSTEM 1795483095350 → INITIALIZATION COMPLETE 1795483628517   533.2 s
第 2 遍（loader 07c890f2，本文 §1/§2 的产物）
START EXTRACT_SYSTEM 1795484158485 → INITIALIZATION COMPLETE 1795484699820   541.3 s
```

两遍的判据一致：814 个包 `install ok installed`、`dpkg -C` 空、日志里
`Bad system call` 0 条、`dpkg: warning: symbolic link … size has changed` 56 条
（与 evidence 140 那一遍**完全同数**，属 dpkg 自己的已知噪声）、
`14-gate-a-acceptance.sh` 34 PASS / 0 FAIL、`12-supervisor.sh` 8 PASS / 0 FAIL。

第 1 遍的 seccomp 面（新 rootfs 的 `tmp/nr-sigsys.log` 从空开始，所以这是本遍的完整计数）：

```text
21,604 行 = 10,205 × "sigsys handler installed" + 10,205 × "sigaction|ret=0" + 1,194 条 trap
1,194 条 trap = 1,181 × nr=264 (name_to_handle_at) + 12 × nr=452 (fchmodat2) + 1 条无 nr= 的 init 标记
```

全部被 handler 接住，没有进程因此死掉。对照 evidence 140 的 1,202 条，属同一量级。

## 1 · 嵌套 exec 的库目录：不能靠"猜"（FACT）

第 1 遍的树初始化完成后，在 guest 的 shell 里直接敲 `sudo` 是失败的：

```text
$ bash -c '/usr/bin/sudo -n true'
/usr/bin/sudo: error while loading shared libraries: libsudo_util.so.0: cannot open shared object file
$ /usr/bin/sudo -n true                       # 同一个 loader，作为会话的第一个程序
sudo: PAM account management error: Socket operation on non-socket
```

原因：`--library-path` 是**替换**不是追加，每次嵌套 exec 都要重建它。
`exec_library_path()` 的优先级是「启动器给的 `NR_LIBRARY_PATH`」→「写死的 6 个目录」。
App 会导出 `NR_LIBRARY_PATH`（`HuskRuntime.kt:308`），但任何只按文档
`NR_ROOTFS`+`NR_LD` 起进程的调用方（设备测试脚本、手工探针）落到第二档，
那 6 个目录里没有 `/usr/libexec/sudo`，于是子进程里的 `sudo` 找不到自己的私有库。

修法：第二档不再猜，读 guest 自己写下的声明 `/etc/ld.so.husk-path`（evidence 141），
逐行加 rootfs 前缀接到搜索路径后面（execve 之前跑，所以只用裸系统调用）。
`NR_LIBRARY_PATH` 存在时（App 恒如此）这段代码根本不会执行。

同时把 `14-gate-a-acceptance.sh` 的 `husk-exec.sh` 改成也导出 `NR_LIBRARY_PATH` ——
验收脚本应当量 App 的启动契约，而不是它的远亲。

## 2 · 初始化结束那一刻，声明必须是全的（FACT）

第 1 遍结束后 `/etc/ld.so.husk-path` 只有 20 个目录、没有 `/usr/libexec/sudo`，
App 重启一次才变 21 个。原因很直白：那次重建发生在「包集刚装完」，
而 **sudo 是下一步 `CONFIGURE_SUDO` 才装的**。

修法：`SAVE_CONFIGURATION`（声明初始化完成之前）再重建一次，代价 4 s（141 的实测值）。
第 2 遍结束后立刻检查：

```text
$ grep -c '^/' /etc/ld.so.husk-path        → 21
$ grep -n sudo /etc/ld.so.husk-path        → 22:/usr/libexec/sudo
$ bash -c '/usr/bin/sudo -n true'          → PAM account management error（即：库找到了）
```

## 3 · 第 2 遍结束时 guest 看到的所有权（FACT）

```text
$ ls -ln /etc/passwd /usr/bin/sudo      → -rw-r--r--. 1 0 0 / -rwsr-xr-x. 1 0 0
$ ls -ln /home/husk                     → Desktop/Documents/… 全部 1000 1000
$ id husk                               → uid=1000(husk) gid=1000(husk) groups=1000(husk),27(sudo)
$ /var/lib/husk/owners                  → 31 条记录，681 字节（含 "0 50"、"0 999" 这类 root:group）
```

`OWNERS` 表在 App 重启后仍然生效（第 1 遍的树被 `am force-stop` 再拉起后，
`/home/husk` 依旧 1000:1000），它就是普通文件，不需要任何后台守护。

## 4 · 时间表（第 2 遍，App 界面）

```text
START EXTRACT_SYSTEM         → DONE    3.9 s
START CREATE_CONTAINER       → DONE  511.2 s   （729 项：GNOME、中文、厂商应用）
START CREATE_USER            → DONE    1.0 s
START CONFIGURE_SUDO         → DONE    4.1 s
START INITIALIZE_ENVIRONMENT → DONE   16.3 s
START SAVE_CONFIGURATION     → DONE    4.8 s  （含 husk-path 重建）
INITIALIZATION COMPLETE                 合计 541.3 s
```

## 5 · 指纹

```text
libld.so           07c890f2758b66e80fd8222028df3255bb9c5466fce8c483c70f5d605f84be32  (673,280 B)
libpath_shim.so    ee3ac422a4d5a6909a1b265652498d7fcf9e0e1179fb09b092ba4ff6e682bd5d  (277,808 B)
husk-usermgr       801b276b94638929799cdbc61efee1e04a8cf654929cb5e06c8fa61cbe9c83cf
libc-nop.so.6      32d4d3f9501a2c717cf2246a2df1a898f1361b49e0576614461d70fc94774b56  （未重建）
设备上 assets.version = husk-runtime-spawn-link-11
```

## 6 · 仍未做（UNKNOWN）

- `sudo` 真正的可用性仍卡在 PAM（`ENOTSOCK`）与 audit netlink —— 服务/桌面表面，
  不在门禁 A 判据里（同 142 §3）。
- 表格键是 `(dev, ino)`，只对 rootfs 的 `st_dev` 生效；跨文件系统的 bind 目标
  （如果将来真做 bind 到别的 dev）会保持真实 owner，未设计。
- 16K/64K 真机仍 UNKNOWN（本机只有 4 KiB）。
