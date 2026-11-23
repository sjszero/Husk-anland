# 134 · 两处只有真文件系统语义才暴露的缺陷：`posix_spawn(NULL pid)` 与 `link()` 的写后失效

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），
包：`com.linuxhub.myth.husk`，loader：`tools/loader/ld-embedded.so`
（`59b25f2cddea361523867c6b248fd95760d7698ca18ebfce44b626b65688d549`）。

两条都是「发行版初始化门禁 A」踩出来的，现象同属一类：**动作成功、结果不对**。

## 1 · 现象（FACT）

`dpkg --configure -a` 全绿之后，`locale -a` 仍然只有 `C`/`C.utf8`/`POSIX`，
`postinst` 里的 localedef 报：

```text
  zh_CN.UTF-8...[error] character map file `UTF-8' not found: No such file or directory
[error] default character map file `ANSI_X3.4-1968' not found: No such file or directory
```

但 `usr/share/i18n/charmaps/UTF-8.gz` 存在、可读、内容正确：

```text
1f 8b 08 00                      # od -An -tx1 -N4
$ gzip -t …/UTF-8.gz             # rc=0（经 shim 读也是 0）
$ perl -e 'open(F,"<…/UTF-8.gz"); read(F,$b,4)' → 1f8b0800
```

`locales` 包只发 `.gz`（234 项，`var/lib/dpkg/info/locales.list`），
所以这不是「本机多一个未压缩副本」的问题。

## 2 · 根因一：shim 拒绝 `posix_spawn` 的 NULL `pid`（FACT）

`usr/bin/localedef`（libc-bin，未被改动）用 `pipe` + `posix_spawnp` 把压缩的
charmap 交给 `gzip -d -c`。设备上把二进制拉回主机反汇编，调用点如下：

```text
36174: mov x3, #0        // attr = NULL
36178: mov x0, #0        // pid = NULL
3617c: ldr x5, [x5]      // envp = environ
36180: bl  posix_spawnp@plt
```

shim 的 `spawn_common` 第一行是：

```rust
if pid.is_null() || path.is_null() { return EFAULT; }
```

于是 `localedef` 根本拿不到解压流，只能报「charmap 没找到」。
旁证：`NR_DEBUG=1` 时 shim 对 `UTF-8.gz` 打出了成功的 open，却**没有任何**
`spawn|search=1|gzip` 行——因为 EFAULT 在打日志之前就返回了。

glibc 自己允许 `pid` 为 NULL（`spawni.c` 只在指针非空时写回子进程号），
修法就是对齐它：

```rust
if path.is_null() { return EFAULT; }          // 只校验 path
…
if !pid.is_null() { *pid = child; }           // 只在非空时回写
```

修复后（`charmaps` 里只留 `.gz`）：

```text
$ localedef -i zh_CN -c -f UTF-8 tmp/loc-fix      # 无任何输出
$ ls -l tmp/loc-fix/LC_CTYPE
-rw-rw-rw- 1 … 361292 2026-11-24 07:03 tmp/loc-fix/LC_CTYPE
```

## 3 · 根因二：`link()` 的 copy 模拟不跟随后续写入（FACT）

`link(2)` 在本域被内核拒掉：

```text
$ ln host-a host-b
ln: cannot create hard link from 'host-a' to 'host-b': Permission denied
```

shim 一直用 copy 兜底（`nrshim: hard link emulated by copy`）。但 glibc 的
`locarchive.c` 的发布顺序是：

```text
mkstemp(temp) → mmap(MAP_SHARED) → link(temp, /usr/lib/locale/locale-archive)
→ unlink(temp) → 继续用**原来那个 fd** 把各 locale 写进映射
```

copy 让 archive 变成另一个 inode，后续写入全落在已被 unlink 的原 inode 上，
于是**第一条 locale 永久丢失**，且不是报错而是「列表为空」：

```text
$ rm -f usr/lib/locale/locale-archive
$ localedef -i zh_CN -c -f UTF-8 zh_CN.UTF-8
$ localedef --list-archive
                                       # 空
$ od -An -tx1 -N64 usr/lib/locale/locale-archive
 09 01 02 de 00 00 00 00 38 00 00 00 00 00 00 00 …   # namehash_used=0：还是空壳
```

最小复现（同一进程，python）：

```text
linked, dst= b'first'
after close, dst= b'first'      # 写 SECOND 并 close 之后没跟过去
```

修法：copy 兜底后把「源 inode → 目标绝对路径」记进一张 8 槽表；
`close(fd)` 时若该 inode 命中，就用 `/proc/self/fd/<fd>` 重新读一遍源 inode
（写端 fd 通常是 O_WRONLY，直接 pread 会 EBADF），整份覆盖到目标。
目标路径在登记时就解析成绝对路径（`/proc/self/fd/<dirfd>` 或
`/proc/self/cwd`），因为调用方的 dirfd 到 close 时早就不在了。
只有先成功打开读端才截断目标——否则一次失败会把好副本清空。

修复后：

```text
linked, dst= b'first'
after close, dst= b'SECOND'

$ rm -f usr/lib/locale/locale-archive
$ localedef -i zh_CN -c -f UTF-8 zh_CN.UTF-8
$ localedef --list-archive
zh_CN.utf8
$ LC_ALL=zh_CN.UTF-8 locale charmap
UTF-8
```

整代 locale 通过 `locale-gen`（不是单跑 localedef）：

```text
Generating locales (this might take a while)...
  zh_CN.UTF-8... done
  zh_SG.UTF-8... done
Generation complete.
$ localedef --list-archive
zh_CN.utf8
zh_SG.utf8
```

## 4 · 本轮回归（FACT）

同一 loader、同一 rootfs：

```text
bash -c 'echo $BASH_VERSION'                → 5.2.21(1)-release
dpkg -C                                     → 无输出，rc=0
dpkg --configure -a                         → rc=0
grep -c sigsys tmp/nr-sigsys.log            → 0 次真实 trap（15 行都是 "handler installed"）
```

## 5 · 已知边界（不要外推）

- 硬链接仍是「copy + 关闭时同步」，不是真 inode 共享：两个名字之后各自写入不会同步，
  并发读者可能看到旧副本。真 hardlink 在本域没有实现路径，这是有意的降级。
- 8 个槽位：同一时刻最多跟踪 8 个「刚发布、尚未关闭」的模拟链接。
- 只在本机（4 KiB / Android 13 / `untrusted_app`）实测；其它设备 UNKNOWN。
- `tmp/nr-sigsys.log` 里早期运行出现的 `nr=264`（388 次）本轮未复现，未定性，UNKNOWN。
