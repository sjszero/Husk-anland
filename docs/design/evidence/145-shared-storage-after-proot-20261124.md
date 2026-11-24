# 145 · 删掉 `-b /storage/emulated/0:/sdcard` 之后：手机存储还是得看得见

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`（debug）。

## 0 · 结论（FACT）

删 proot（144）时把 `androidStorageBindings()` 一起删了，guest 于是**看不到手机存储**：

```text
修复前（loader 07c890f2… / 删 proot 后的第一个 APK）
$ ls /sdcard
ls: cannot access '/sdcard': No such file or directory
$ ls /storage/emulated/0
ls: cannot access '/storage/emulated/0': No such file or directory
```

现在（loader `d0fa2614…`，App 自带终端，即 App 自己的进程里量的）：

```text
husk@localhost:/$ ls -ld /sdcard
lrw-r--r--. 1 root root 21  8月 22  2024 /sdcard -> /storage/self/primary
husk@localhost:/$ ls /sdcard | head -3
about2.xml
about.xml
Alarms
husk@localhost:/$ touch /sdcard/husk-probe2 && echo WRITE-OK
WRITE-OK
```

`touch` 出来的文件属 `root husk9997`（`9997` 是 media provider 的真实 gid，guest 里没有它的
名字，所以按 evidence 129 的规则合成成 `husk9997`）。读、写、删都通。

## 1 · 为什么是"透传两个名字"，不是再做一个 bind

proot 用 `-b /storage/emulated/0:/sdcard` 把宿主目录挂进 guest。Husk 里没有挂载表条目
能指向宿主路径（`mount()` 的 source 会被当成 guest 路径解析），而 `/sdcard` 在 rootfs 里
根本不存在。真正需要说的是**"这两个名字属于手机的共享卷，不属于 guest 状态"**，所以在
`passthrough()` 里加一条 `shared_storage()`：

```text
guest /sdcard  → __ __ → 宿主 /sdcard      （内核把 /sdcard 解析到 /storage/self/primary
                                              → /storage/emulated/<本机用户>，多用户也对）
guest /storage → 宿主 /storage
```

能力边界没有变化：guest 进程就是 App 的 uid，能碰到什么仍然由 App 自己拿到的
`MANAGE_EXTERNAL_STORAGE`（本机实测 `appops get … MANAGE_EXTERNAL_STORAGE` = `Uid mode: allow`）
和内核权限决定，不是这条路径给的。`NR_SHARED_STORAGE=0` 可以单独关掉做对照。

## 2 · `..` 必须自己拦（第一版踩到了）

第一版只加了前缀匹配，实测 `/storage/../system` 变成**宿主的** `/system`（EACCES）：

```text
$ ls /storage/../system
usr/bin/ls: cannot access '/storage/../system': Permission denied     ← 走的是宿主命名空间
```

原因：内核在 `..` 上做解析，而这里没有挂载点可以"停住"。所以 `shared_storage()` 现在和
`/proc`、`/dev` 一样，路径里出现 `..` 分量就不算透传，交给 guest root 解析（于是落空）。

```text
$ ls /storage/../system
ls: cannot access '/storage/../system': No such file or directory
$ ls /storage/../data/data/com.linuxhub.myth.husk
ls: cannot access '/storage/../data/data/com.linuxhub.myth.husk': No such file or directory
```

## 3 · 回归（同一个 rootfs，全套重跑）

新 loader `d0fa2614…`（shim `535fca70…`），空 checkpoint 从 App 界面重跑：

| 判据 | 结果 |
|---|---|
| 包状态 | 814 `install ok installed`，`dpkg -C` 空 |
| `Bad system call` | 0 |
| seccomp 陷阱 | 1,181 × `nr=264` + 12 × `nr=452`，全部接住 |
| 初始化时长 | 548.2 s（EXTRACT 3.6 / CONTAINER 519.6 / USER 1.0 / SUDO 3.0 / ENV 16.2 / SAVE 4.7） |
| 门禁 A | 34 PASS / 0 FAIL |
| 监督器 | 8 PASS / 0 FAIL |
| 所有权 | `/etc/passwd` `0:0`、`/usr/bin/sudo` `0:0`、`/home/husk` `1000:1000` |
| guest 库目录 | `/etc/ld.so.husk-path` 22 项 |
| guest 进程数 | 10,208 |

548.2 s 与删 proot 那几次的 533 / 541 / 543 s 同量级（dpkg 自身波动）。

## 4 · 仍未闭环

- **GNOME 桌面首帧仍然没有画面**（两次都报"首帧超时，可见像素 0‰"）。这次两个 loader
  版本表现一致，但**仍然没有"带 proot 的 2.0"基线**，所以归因继续是 UNKNOWN，G 段延后。
- `/storage` 整棵透传（不只是 `/storage/emulated/0`）：`/storage/persist`、别的用户卷都
  看得见名字，读不到内容（内核权限）。要把面收窄到只暴露自卷，需要引入"挂载点父目录
  截断"的语义，现在不做，记账。
- 合成组名 `husk9997` 只是缺名字，不是假的权限结论（evidence 129 的既有行为）。

## 5 · 复现

```sh
# 探测（run-as 下 /sdcard 会 EACCES：那是 runas_app 域，不是 App 域，别用它下结论）
run-as com.linuxhub.myth.husk sh -c 'cd files/linux/distributions/ubuntu-24.04/rootfs &&
  NR_ROOTFS=$PWD NR_LD=/data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so \
  $NR_LD --library-path "$PWD/lib/aarch64-linux-gnu:$PWD/usr/lib/aarch64-linux-gnu" \
  usr/bin/env PATH=/usr/bin:/bin usr/bin/ls -ld /sdcard'

# 真实结论在 App 自带终端里量（App 进程 = untrusted_app + MANAGE_EXTERNAL_STORAGE）
```
