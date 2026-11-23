# 105 · libc 内部调用的槽位 interpose：DNS / 时区 / locale 一次性打通

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`
（全部命令都在 guest（app uid 10302）里跑，无 root、无 chroot、无 ptrace）

## 现象（改之前，FACT）

| 命令 | 结果 |
|---|---|
| `cat /etc/resolv.conf` | 正常打印 guest 文件 |
| `getent hosts baidu.com` | 空，rc=2 |
| `python3 -c 'getaddrinfo("example.com",80)'` | `[Errno -3] Temporary failure in name resolution` |
| `TZ=Asia/Shanghai date` | `Mon Nov 23 09:25:48 **Asia** 2026`（把 `Asia` 当 POSIX 时区串，时区文件没读到） |
| `locale -a` | 只有 `C` / `POSIX` |
| 出站网络 | TCP 223.5.5.5:443、UDP 53 均通（FACT）：不是网络问题 |

原因：GOT 重写只覆盖走 PLT 的调用。libc 调自己（`_IO_file_open` → `open`、
resolver → `fopen("/etc/resolv.conf")`、locale/tzfile 加载器 → `open`）是直接 `bl`，
没有 GOT 槽，于是落到**宿主**路径上。这与 evidence 29 记录的是同一个根因。

运行时改写代码页不可行（evidence 29：改过的文件页再 `mprotect(PROT_EXEC)` 回 EACCES），
所以重定向必须**编译期写进 libc.so.6**。

## 实现

`nr-tools libc-interpose <libc> <out> <names.txt>`（纯 Rust，无依赖）：

1. 在 libc 的可执行 LOAD 之后的**零填充区**（本机 0x19a000..0x19d100）放两页：
   `R|X` 存根页、`R|W` 槽位页；可执行 LOAD 的 `p_filesz` 覆盖存根页（不碰 .bss：
   把 `p_filesz` 扩到数据段 .bss 上会把 section header 表变成"变量初值"，程序直接挂）。
2. 每个被接管的函数入口 24 字节改成：

   ```
   adrp x16, slot_page        ; 槽位页基址
   ldr  x17, [x16, #target]   ; 0 = 还没填空
   cbnz x17, #12              ; 填了就跳到 br x17
   ldr  x17, [x16, #delta]    ; 编译期写入：存根地址 - 槽位页基址
   add  x17, x16, x17
   br   x17
   ```

   没填空时走存根页，执行被覆盖的 6 条原指令（**PC 相对指令做精确重定位**，
   越界的按名字拒绝，绝不硬patch），然后 `b entry+24`。
3. 运行时 `_dl_init` 钩子（`boot.rs::fill_slot_page`）扫每个对象的 PT_LOAD，
   找到 `HUSK-SLOT-1` 页，按名字在自己的 interpose 表里查地址写进 target。
   页本来就可写，**不需要任何运行期 mprotect**。

## 结果（FACT，同一台机、同一条命令）

| 命令 | 改之后 |
|---|---|
| `getent hosts baidu.com` | `110.242.74.102 baidu.com` rc=0 |
| `getent hosts example.com` | 真实 IPv4/IPv6 |
| `python3 getaddrinfo("example.com",80)` | `('172.66.147.243', 80)` |
| `TZ=Asia/Shanghai date` | `Mon Nov 23 17:40:13 **CST** 2026` |
| `locale -a` | `C` `C.utf8` `POSIX` |
| `df -h /` | `108G 35G 74G 33% /`（之前报宿主 system 分区 760M 100%） |
| `apt-get update` | **rc=0**，`/var/lib/apt/lists` 37 个文件，来自 ports.ubuntu.com |

`apt-get update` 通过是这一项最硬的证据：DNS → TLS → 证书 → dpkg 元数据全链路。

## 覆盖与拒绝（可复现）

`tools/loader/libc-interpose.txt` 67 个名字，56 个落地。跳过 11 个，原因写死：

```
__readlink_chk / __readlinkat_chk / __realpath_chk   分支跳进被覆盖的 24 字节窗口
__open_2 / __open64_2 / __openat_2 / __openat64_2    tbz(±32 KiB) 无法从存根页够到
mkstemps / mkostemps                                 同上
renameat2                                            cbz 超出 ±1 MiB
__openat64_nocancel                                  这个 libc 没导出
```

`__open_2` 这一族是 _FORTIFY 包装，**程序侧**调用已经由 GOT 重写覆盖；libc 内部走的是
`__open64_nocancel`（已接管），所以缺它们不影响目标场景。

## 复现

```sh
runtime/target/release/nr-tools libc-interpose \
    rootfs/usr/lib/aarch64-linux-gnu/libc.so.6.orig /tmp/libc.slots.so.6 \
    tools/loader/libc-interpose.txt          # 打印 "56 of 67"，不匹配就拒绝写文件
# 推上去替换 libc.so.6，配 tools/loader/build-embedded-loader.sh 产出的 libld.so
```

## 已知残留（未解决，不算完成）

- `/proc/stat`、`/proc/loadavg`、`/proc/net/*`、`/sys/class/net`、netlink：SELinux 对
  `untrusted_app` 拒绝，需要由 app 侧合成或改 targetSdk 域（未测）。
- `link()`（硬链接）在 app 私有目录里被内核拒绝（EACCES），与 shim 无关；要模拟得改成复制。
- `/dev/full`、`/dev/log`、`/dev/mqueue` 不存在；SysV IPC（shmget/semget/msgget）被
  Android seccomp 回 ENOSYS。
- `/proc/mounts` 仍是宿主的（工具看到的挂载表不对）。
