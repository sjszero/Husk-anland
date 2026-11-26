# 247 · 用户态路径入口复核：5 个入口都落到已翻译的底层调用

日期：2026-11-26 10:54（CST）。设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / 4 KiB。
探针：`tools/probe/symbol-gap-probe.c`。运行 loader：0.0.22 候选版。

## 0 · 结论

| 用户态入口 | 本次结果 | 结论 |
|---|---|---|
| `realpath(3)` | `rc=0`，结果为 `/root/gap-tmp/c` | FACT：路径不再落到宿主 |
| `canonicalize_file_name(3)` | `rc=0`，结果为 `/root/gap-tmp/c` | FACT |
| `glob(3)` | `rc=0`，`glob_count=3` | FACT |
| `nftw(3)` | `rc=0`，`nftw_seen=4` | FACT |
| `statvfs(3)` | `rc=0`，块数来自 guest rootfs | FACT |

这次是**覆盖复核，不是运行时代码修复**。这些入口没有被单独重写，它们最终调用已经接管的底层路径入口；
所以不能把“探针通过”写成“又修了五个符号”。

同一探针里，`dlmopen` 与通过 libc 发起的 `execveat` 本次也已成功；raw
`syscall(SYS_execveat)` 仍以 exit 97 失败，这是已知的 raw-syscall 边界，不是 libc 路径回归。
`chroot` / `umount` / `swapon` / `swapoff` 的失败是 Android `untrusted_app` 权限边界，钩路径也无法改变。

## 1 · 原始输出（FACT）

```text
__xstat                    ok   rc=0
   size=3
__lxstat                   ok   rc=0
__fxstatat                 ok   rc=0
remove                     ok   rc=0
freopen                    ok   rc=0
pathconf                   ok   rc=255
truncate64                 ok   rc=0
shm_open                   ok   rc=5
sem_open                   ok   rc=0
dlmopen                    ok   rc=0
execveat(/usr/bin/true)    child exit=0  (exec 成功)
syscall(SYS_execveat)      child exit=97  (raw syscall，设计边界)
realpath                   ok   rc=0
   realpath=/root/gap-tmp/c
canonicalize_file_name     ok   rc=0
   canonical=/root/gap-tmp/c
glob                       ok   rc=0
   glob_count=3
nftw                       ok   rc=0
   nftw_seen=4
statvfs                    ok   rc=0
   statvfs_bsize=4096 blocks=28292600
chroot(/)                  FAIL errno=1 (Operation not permitted)
umount                     FAIL errno=22 (Invalid argument)
swapon                     FAIL errno=1 (Operation not permitted)
swapoff                    FAIL errno=1 (Operation not permitted)
```

## 2 · 边界

- FACT：`glob_count` 与 `nftw_seen` 只证明本次临时目录的遍历结果，不证明所有 glob 模式或深目录都正确。
- FACT：`statvfs` 只证明 guest `/root` 走对了文件系统，没有证明磁盘配额、挂载传播等完整语义。
- INFERENCE：这些入口依赖的 `openat` / `stat` / `getdents` 类底层入口已由 loader 接管，因此当前调用链成立。
- UNKNOWN：未在 Adreno、16 KiB / 64 KiB 真内核或非 Ubuntu rootfs 上重跑这组入口。
