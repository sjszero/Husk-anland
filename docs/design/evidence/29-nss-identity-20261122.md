# evidence 29 — 身份解析：libc 内部调用绕过 interpose，NSS 单独接管

日期：2026-11-22 21:10 CST
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191

## 症状

`bash -i` 提示符 `I have no name!@localhost:/$`，`groups` 与 `id` 对**每一个** gid
都打印 `cannot find name for group ID N`——包括 `/etc/group` 里明明存在的 gid。
而同一个进程里 `tail -2 /etc/passwd` 能正常读出文件内容，文件里也确实有该 uid 的行。

## 根因（FACT）

两条独立的机制叠在一起：

1. **glibc 的 NSS 读不到 rootfs。** `/etc/passwd`、`/etc/group` 的 `files` 后端
   编在 libc 里，它用**直接调用**打开 `/etc/passwd`，不经过任何 GOT 槽；而 shim 的
   路径重写只改 GOT 槽。于是 libc 内部看到的是宿主路径 `/etc/passwd`（不存在），
   `getpwuid()` 返回 NULL。程序自己调用 `open()`（走 PLT → GOT）一切正常，
   这就是「`cat` 能读、`getpwuid` 读不到」的分裂。

   证据：`LD_DEBUG=libs` 全程没有任何 `libnss_files` 的 dlopen（说明它是内建的），
   而 `groups root` 报 `'root': no such user`——连最早就在文件里的条目也解析不了。

2. **修补 libc 文本的运行时方案在本机不可行。** 直接改 libc 函数入口（16 字节
   `ldr x16,#8 ; br x16 ; .quad target`）在 `getcwd` 上验证成功，但在
   `open/open64/openat/openat64` 上立刻 SEGV。加 SIGSEGV 现场记录后定位：

   ```
   segv|sig=11|...|x1=<libc+0xe04a0>|x2=4|...|ld=...|lc=...
   body-rx <libc+...> r=-13        # -EACCES
   ```

   即：`mprotect(rw)` 成功，补完 `mprotect(r-x)` 被**拒绝（EACCES）**——app 域
   不允许把「已改过的文件映射」重新变成可执行（W^X / execmod）。于是整页失去执行权限，
   下一次取指就崩。进程随后在 `_IO_file_seekoff` 的 `blr`（跳到同页的 `lseek64`）
   上死在 `libc+0xe04a0`。

   结论：**文本修补只能在构建期做**，运行时只允许写数据段。

## 本轮的修复（FACT，已上机）

身份函数由 shim 自己接管，直接读 rootfs：

- `getpwuid` / `getpwuid_r` / `getpwnam` / `getpwnam_r` → `/etc/passwd`
- `getgrgid` / `getgrgid_r` / `getgrnam` / `getgrnam_r` → `/etc/group`
- 文件里没有的 gid（Android 给每个 app 塞了一堆内核组：`input`、`log`、`adb`、
  `sdcard_rw`…）返回**合成名** `husk<gid>`，与 app 启动时写进 `/etc/group` 的名字一致。

app 侧同时把本进程的 uid/gid/supplementary groups 镜像进 rootfs 的
`/etc/passwd`、`/etc/group`（每次启动幂等追加），并把提示符固定成
`PS1='\u@husk:\w\$ '`——`\h` 读的是 `gethostname()`，Android 上恒为 `localhost`，
改主机名要 CAP_SYS_ADMIN。

实测：

```
$ id
uid=10302(husk) gid=10302(husk10302) groups=10302(husk10302),1004(husk1004),…,50302(husk50302)
$ groups
husk10302 husk1004 … husk50302
终端提示符：husk@husk:/$
```

## 仍然存在的缺口（下一步，已记录设计）

libc 内部对**其它**路径的访问仍然落在宿主文件系统上：locale 文件、`/etc/hosts`、
`/etc/ld.so.cache`、apt/dpkg 自己的状态文件。可行方案（见
[libc-interpose.md](../libc-interpose.md)）：构建期在 libc 里把每个路径入口改成

```
adrp x16, <slot page> ; ldr x16, [x16, #off] ; br x16 ; nop
```

`<slot>` 落在构建期新增的**可写** PT_LOAD 里，运行时由 shim 在 `_dl_init` 填成自己的
实现地址——只写数据段，不碰代码页，因此不受 EACCES 影响。首 16 字节内用 adrp+ldr
而不是字面量，是因为新增段离 `.text` 超过 LDR 字面量 ±1MB 的范围。
