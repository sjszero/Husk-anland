# 133 · 嵌套 exec 必须继承 top-level 的完整 `--library-path`

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），
包：`com.linuxhub.myth.husk`，rootfs：
`files/linux/distributions/ubuntu-24.04/rootfs`，loader：`files/linux/husk/libld.so`。

## 0 · 结论（FACT）

`--library-path` 是 **替换**，不是追加。旧 `execve` 在每一次嵌套 exec 时只重建
`<rootfs>/usr/lib/aarch64-linux-gnu` 一个目录，于是私有库目录
（`usr/lib/aarch64-linux-gnu/systemd`、`mutter-14`、`gnome-shell`、`gvfs`）
在第二层进程里消失。systemd 的 postinst 就是这样坏的：

```text
…/rootfs/usr/bin/systemd-machine-id-setup: error while loading shared libraries:
libsystemd-shared-255.so: cannot open shared object file: No such file or directory
```

现在 launcher 用 `NR_LIBRARY_PATH` 把交给 loader 的**同一份**完整路径传给 path-shim，
所有嵌套 exec 原样复用；未设置时 shim 用内置多目录 fallback（直接设备测试仍可只给
`NR_ROOTFS`/`NR_LD`）。

## 1 · 同一命令，旧 loader / 新 loader（FACT）

设备上两份 loader：旧 `d727fd3c…`（`assets.version=husk-runtime-clone3-2` 部署的），
新 `59b25f2c…`（本仓库构建，见 134 的最终产物）。

```sh
cd files/linux/distributions/ubuntu-24.04/rootfs
R=$PWD; L=…/files/linux/husk/libld.so
LP="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu/systemd"
env -i NR_ROOTFS=$R NR_LD=$L NR_LIBRARY_PATH="$LP" NR_SIGSYS=enosys \
    GLIBC_TUNABLES=glibc.pthread.rseq=0 PATH=… \
    $L --library-path "$LP" usr/bin/sh -c 'usr/bin/systemd-machine-id-setup --print; echo NESTED_RC=$?'
```

旧 loader：

```text
…/usr/bin/systemd-machine-id-setup: error while loading shared libraries:
libsystemd-shared-255.so: cannot open shared object file: No such file or directory
NESTED_RC=127
OUTER_RC=0
```

新 loader：

```text
d79609407bf3677e4ef3f2a36b04872b
NESTED_RC=0
OUTER_RC=0
```

新 loader、**不设** `NR_LIBRARY_PATH`（fallback 分支）：

```text
d79609407bf3677e4ef3f2a36b04872b
NESTED_RC=0
```

## 2 · 实现（FACT）

`runtime/crates/path-shim/src/lib.rs`：

- `execve` 不再自己拼 `<root>/usr/lib/aarch64-linux-gnu`，改为
  `exec_library_path(root, envp, &mut lib)`；
- `exec_library_path` 先读 `NR_LIBRARY_PATH`（launcher→shim 的内部契约），
  缺失时退回 `DEFAULT_LIBRARY_DIRS`
  （`lib/aarch64-linux-gnu`、`usr/lib/aarch64-linux-gnu`、`…/systemd`、
  `…/mutter-14`、`usr/lib/gnome-shell`、`…/gvfs`），失败返回 `ENAMETOOLONG`。

启动契约因此是三个变量：

```sh
NR_ROOTFS=<rootfs> NR_LD=<loader> NR_LIBRARY_PATH=<完整库路径> \
  <loader> --library-path "$NR_LIBRARY_PATH" usr/bin/bash -c '…'
```

## 3 · 上机回归（FACT）

同一 rootfs、同一 loader：

```text
$ dpkg --configure -a                     → CONFIG_RC=0
  Setting up netplan-generator (1.1.2-8ubuntu1~24.04.3) ...
$ dpkg -C                                 → 无输出，rc=0
```

`dpkg --configure systemd` 早在上一轮就用同一 loader 实测成功（见 132 的上下文）。

## 4 · 边界

- 这是 2.0 集成现场（`com.linuxhub.myth.husk`）的结论，不改写旧 `com.linuxhub.husk` 的历史记录。
- `NR_LIBRARY_PATH` 是内部变量；对外文档口径仍是「两变量 + 可选第三变量」，只设两变量时行为靠 fallback，已实测。
