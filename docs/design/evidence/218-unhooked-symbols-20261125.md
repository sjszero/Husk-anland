# 218 · 没钩的符号逐个实测：15 个里 2 个是真缺口

日期：2026-11-25 22:3x（CST）。设备：RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB。
探针：`tools/probe/symbol-gap-probe.c`（guest 内 `gcc` 编，源码入库）。

## 0 · 结论

| 项 | 数 |
|---|---|
| 需要路径翻译的符号（`tools/dev/path-symbols.txt`） | 111 |
| 修前覆盖 | 90 |
| 本次新增钩子 | 6（`__xstat` `__xstat64` `__lxstat` `__lxstat64` `__fxstatat` `__fxstatat64`） |
| 修后覆盖 | **96** |
| 仍未覆盖 | 15，其中**实测有后果的 2 个**：`dlmopen`、`execveat` |

## 1 · 被实测推翻的「按推理定性」

`__xstat` 家族以前被归到「老二进制才用、且要 body patch 才能补」，实测是**现在就该补**：
glibc 2.39 的 `.dynsym` 里这 6 个符号都在（`readelf -sW --dyn-syms` 共 2877 个），
它们是**给老二进制留的兼容导出**，只要程序直接引用就一定能被 GOT 改写到。

修前 / 修后（同一个探针、同一台设备，`NR_LD` 指旧 loader / 新 loader）：

```
# 修前（loader = libld.so.pre217）
__xstat   ver=0 rc=-1 errno=2 size=-1
__lxstat  ver=0 rc=-1 errno=2
__fxstatat ver=0 rc=-1 errno=2

# 修后
__xstat                    ok   rc=0
   size=3
__lxstat                   ok   rc=0
__fxstatat                 ok   rc=0
```

ENOENT 而不是别的：路径原样交给 Android 内核，`/root/gap-tmp/c` 在宿主上不存在。

**探针本身的坑**（记下来）：第一版先 `__xstat` 再建文件，得到的同样是 ENOENT，
但那是「文件不存在」而不是「路径没翻译」。两个完全相反的结论长得一模一样，
所以探针必须先建文件再测。

## 2 · 剩下 15 个的实测明细（FACT，2026-11-25 22:33）

```
__xstat                    ok   rc=0        <- 本次修好
__lxstat                   ok   rc=0        <- 本次修好
__fxstatat                 ok   rc=0        <- 本次修好
remove                     ok   rc=0
freopen                    ok   rc=0
pathconf                   ok   rc=255
truncate64                 ok   rc=0
shm_open                   ok   rc=5   (guest /dev/shm/husk-gap 真的建出来)
sem_open                   ok   rc=0   (guest /dev/shm/sem.husk-gap 真的建出来)
dlmopen                    FAIL errno=0
execveat(/usr/bin/true)    FAIL errno=2 (No such file or directory)
chroot(/)                  FAIL errno=1 (Operation not permitted)
umount                     FAIL errno=22 (Invalid argument)
swapon                     FAIL errno=1 (Operation not permitted)
swapoff                    FAIL errno=1 (Operation not permitted)
```

`dlmopen` 的 `dlerror()` 是
`/usr/lib/aarch64-linux-gnu/libm.so.6: cannot open shared object file: No such file or directory`，
而同一路径 `dlopen()` 正常——差别就在：`dlopen` 是我们钩的（先把路径翻好再交给
glibc），`dlmopen` 不是，glibc 的 `ld.so` 自己去 open。

## 3 · 怎么读这张表

- **11 个「通」不是运气**：`remove` `freopen` `pathconf` `truncate64` 的 glibc 实现
  内部转调的是已经被改写的入口（`unlink`/`rmdir`/`open`/`statfs`）；
  `shm_open`/`sem_open` 拼出 `/dev/shm/<名>` 再 `open`，而 `/dev/shm` 在 shim 里有
  专门映射；`chroot`/`umount`/`swapon`/`swapoff` 是权限层根本不给（EPERM/EINVAL），
  钩不钩都一样。
- **2 个「不通」是真缺口**，0.0.13 列进修单：`dlmopen`（要仿 `dlopen` 做路径翻译）、
  `execveat`（要仿 `execve` 做装载）。
- `faccessat2` 不在表里：它不是 libc 符号，只是系统调用号，glibc 走 `faccessat` 回退，
  439 已在 `sigsys_answer()` 里答 ENOSYS 强制回退。

## 4 · 覆盖度这条线现在是三道门禁

| 门禁 | 查什么 | 现状 |
|---|---|---|
| `tools/dev/hook-coverage.sh` | 静态差集：清单 vs 实际导出符号 | 96 / 111 |
| `tools/probe/coverage-probe.c` | 运行时：19 个路径入口在 guest 里真的通 | PASS |
| `tools/probe/coverage-sweep.sh` | 真实负载 `strace` 抓 1379 条路径，断言都带 rootfs 前缀 | PASS |

`make gate-a` 现在一次跑完：38 项验收 + 上面后两道。
