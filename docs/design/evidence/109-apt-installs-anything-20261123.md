# 109 · apt 现在能装任何东西：三个内核域缺陷 + 一个账号约定

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`

起点是 evidence 106：`apt-get install tree` 能过。但那只是"没有符号链接的包"能过。
升级/重装任何带链接的包时 dpkg 在**第一个符号链接成员**上就死：

```
dpkg: error processing archive libdbus-1-3_...deb (--unpack):
 error setting timestamps of '/usr/lib/aarch64-linux-gnu/libdbus-1.so.3.dpkg-new':
 No such file or directory
```

不是内核对时间戳有意见，是这个域里有三件事按常规做法做不到。

## 1. `lutimes` 没被接管 → 所有带符号链接的包都装不上

`dpkg --unpack` 的失败信息有两句，来源不同（`src/main/archives.c:527`）：

```c
#ifdef HAVE_UTIMENSAT
	rc = utimensat (AT_FDCWD, path, ts, flags);      /* 失败 → "cannot set timestamps for" */
#endif
	if (te->type == TAR_FILETYPE_SYMLINK)
		lutimes (path, tv);                      /* 失败 → "error setting timestamps of" */
	else
		utimes (path, tv);
```

**Ubuntu 的 dpkg 是用 `--without-utimensat` 的语义编的**：它的动态符号表里只有
`utimes` 和 `lutimes`，根本没有 `utimensat`（FACT：`readelf --dyn-syms /usr/bin/dpkg`）。
所以它每一个符号链接都走 `lutimes`，而 shim 只接管了 `utimes`。

结果：`utimes`（普通文件）被翻译，`lutimes`（符号链接）落到宿主命名空间 →
`/usr/lib/.../X.dpkg-new` 在宿主上不存在 → ENOENT。`tree` 没有符号链接，所以一直没暴露。

修法：shim 补 `lutimes`，并把它加进 `tools/loader/libc-interpose.txt`（libc 内部调用者
也要走同一条路）。`utimensat` 的 `AT_SYMLINK_NOFOLLOW` 分支同时改成"只解析父目录 +
`AT_SYMLINK_NOFOLLOW`"，不再用 `/proc/self/fd`（那条路会跟随链接，悬空链接就 ENOENT）。

## 2. glibc 的 `ldconfig` 是 static-pie → libc-bin 的 postinst 必然失败

`/sbin/ldconfig` 是个脚本，最后 `exec /sbin/ldconfig.real`。那个 .real 是 **static-pie**
（ET_DYN 但没有 PT_INTERP，`llvm-readelf -h` 可见），所以：

- 它不经过我们的 loader，shim 对它无效；
- 它看到的是**宿主**的 `/lib`、`/etc`：`Can't stat /lib/aarch64-linux-gnu`、
  `Can't create temporary cache file /etc/ld.so.cache~: Read-only file system`；
- 它退出码 1，`dpkg --configure libc-bin` 失败，而 apt 在有半配置包时**拒绝做任何事**。

修法：新 crate `runtime/crates/ldconfig`（纯 Rust、guest 二进制、跑在 loader 下），
自己扫 `/etc/ld.so.conf` + `/etc/ld.so.conf.d/*.conf` 之外的标准 multiarch 目录，
读每个 ELF 的 `DT_SONAME`，按 `glibc-ld.so.cache` **1.1** 格式（glibc 2.39 的
`sysdeps/generic/dl-cache.h`：48 字节头 + 24 字节条目 + 字符串表，条目按 soname 排序）
写 `/etc/ld.so.cache`。原件保留为 `ldconfig.real.glbc`。

**独立校验**（不是自己读自己）：glibc 自己的静态阅读器接受我们写的文件——

```
$ /usr/sbin/ldconfig.real.glbc -C <root>/etc/ld.so.cache -p | head -2
734 libs found in cache `/data/data/com.linuxhub.husk/files/rootfs-full/etc/ld.so.cache'
	libEGL.so.1 (libc6,AArch64) => /lib/aarch64-linux-gnu/libEGL.so.1
```

（不递归子目录：`gconv/`、`dri/`、`glibc-hwcaps/` 里的模块是按路径加载的，
进 soname 表只会遮蔽正常库。`-p | head` 触发的 SIGPIPE 也不再 panic。）

## 3. `audit_open` 被 seccomp 拒绝 → shadow 直接退出

任何 `useradd`/`groupadd` 的第一句是：

```
Cannot open audit interface - aborting.
```

libaudit 的 `audit_open()` 要开 `NETLINK_AUDIT` 套接字，本 ROM 的 seccomp 直接拒。
shadow 把 `audit_open() < 0` 当致命错误（`libmisc/audit_help.c`）。

一个有 `CAP_AUDIT_WRITE` 的容器拿到的是"能写、写了就丢"的套接字——端状态一样。
所以 shim 导出 `audit_open()`，返回 `/dev/null` 的 fd：写入成功、审计没人听，
`audit_log_user_message` 不会报错。

## 4. shadow 的锁协议要 `nlink == 2`，而这个域做不出硬链接

修完上一条，还剩：

```
groupadd: /etc/group.18193: lock file already used (nlink: 1)
```

shadow 的 `do_lock_file()`（`lib/commonio.c`）先建 `<file>.<pid>`，再
`link(file, lock)`，然后 **`check_link_count()` 要求 `st_nlink == 2`**（证明锁是自己刚建的）。
本域 `linkat` 对 app 私有目录返回 **EACCES**（FACT：裸 syscall 实测 errno 13），
shim 只能退化成"复制"，复制品的 `nlink` 永远是 1 → shadow 判定锁被别人占着。

硬链接做不到，锁协议就没法伪造（伪造 `nlink` 是编数据，不做）。可行的路是镜像里
先带好账号 —— 和发行版镜像做的事一样。`tools/guest-config/guest-accounts.txt` 加两条：

```
rtkit:106:106:1
pipewire::107:1
```

两个 postinst 都是 `getent` 守卫（`if ! getent group pipewire`），账号在就整段跳过。

## 结果（FACT，同一台机）

```
$ dpkg --configure -a            rc=0     # rtkit / pipewire / gstreamer1.0-pipewire 都配好了
$ apt-get -f install             rc=0
$ apt-get install -y --reinstall libpipewire-0.3-dev   rc=0   → /usr/include/pipewire-0.3 落地
$ apt-get install -y cowsay fortunes-min               rc=0   → /usr/games/cowsay 能跑
$ dpkg -l | awk '状态不是 i 的行'                        空     # 数据库里没有任何半配置包
```

`rt-server-parity.sh` 新增两条真实测量：

| 检查 | 结果 |
|---|---|
| `pkg db_consistent` | PASS（0 个未配置完的包；`hi` 是 held+installed，不算坏） |
| `pkg ld_cache` | PASS（glibc 自己的阅读器报 `734 libs found`） |

**73 PASS / 5 PARTIAL / 6 FAIL**（71/5/6 → 73/5/6）。

## 仍然做不到的（如实记账）

- **`useradd` / `groupadd` / `adduser` 本身**依旧不能用（第 3、4 条）。装新包时如果它的
  postinst 要新建账号，得先把账号加进 `guest-accounts.txt`。真正的根治是自带一个
  Rust 的 `useradd`（直接改 `/etc/passwd`…，不玩锁），尚未做。
- **真正的硬链接**不可用（内核对 app 私有目录拒 `linkat`），现在一律退化为复制，
  所以 `nlink` 语义不对；依赖它的程序（shadow 的锁、某些备份工具）会做出错误判断。
