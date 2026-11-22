# libc 内部调用的 interpose：构建期槽位方案

> 状态：设计已定，未实现。触发原因见 [evidence 29](evidence/29-nss-identity-20261122.md)。

## 问题

shim 的 interpose 是把**每个对象的 GOT 槽**改成自己的函数（`boot.rs` 第 2 步）。
这覆盖了：

- 程序与其它 DSO 通过 PLT 的调用 —— 也就是绝大多数情况；
- 不覆盖 **libc 内部**的直接调用（同一编译单元里的 `bl`，没有 GOT 槽）。

libc 内部的路径访问因此落在宿主文件系统上：NSS（`/etc/passwd`、`/etc/group`）、
locale（`/usr/lib/locale`）、`/etc/hosts`、`/etc/ld.so.cache`、apt/dpkg 的状态文件。
症状是「程序自己 `open()` 一切正常，libc 自己读同一个路径却 ENOENT」。

## 为什么不能在运行时改文本

实测（evidence 29）：`mprotect(text, RW)` 成功，写完再 `mprotect(RX)` 返回 **EACCES**——
app 域不允许把「已改过的文件映射」重新变成可执行（W^X / execmod）。整页失去执行权限，
下一次取指就崩。所以**运行时不写代码页**，这是硬约束，不是偏好。

## 方案：构建期放跳板，运行时填槽

### 构建期（`nr-tools libc-interpose`）

对 libc.so.6 做一次离线 ELF 手术：

1. 追加一个新的 PT_LOAD：`RW`，内容是一个头 + 一张表。

   ```
   struct husk_slots {
       u8   magic[12];   // "HUSK-SLOT-1\0"
       u32  count;
       u32  _pad;
       struct { char name[32]; u64 target; } e[count];
   };
   ```

   `name` 用和 shim 导出符号完全一致的字符串，`target` 初值 0。

2. 对每个要接管的函数（列表见下），把入口前 16 字节改成：

   ```
   adrp x16, <slots page>      ; 4 字节
   ldr  x16, [x16, #<off>]     ; 4 字节，off 是槽在页内的偏移（8 对齐）
   br   x16                    ; 4 字节
   nop                         ; 4 字节
   ```

   用 `adrp + ldr` 而不是 `ldr x16, #imm` 字面量：字面量寻址范围 ±1MB，而新段追加在
   文件末尾，和 `.text` 的 vaddr 距离远超 1MB；adrp 是 ±4GB。

3. 校验：入口必须是 16 字节对齐的函数起点（`st_value` 来自 `.dynsym`），
   并且**不能**落在 `.plt`、ifunc 解析器或 `__libc_start_main` 前的引导路径上。

### 运行时（shim，`_dl_init` 钩子里）

1. 从 link map 找到 `libc.so.6`，读它自己的 ELF header（`l_addr` 处 `e_phoff/e_phnum`）。
2. 遍历 PT_LOAD，找到以 `HUSK-SLOT-1` 开头的段（内容已经映射，直接读）。
3. 对每个条目按名字在 **shim 自己的 interpose 表**里查地址，写进 `target`。
   只写 RW 数据段，不需要任何 `mprotect`。
4. 任何名字在 shim 侧找不到 → 把 `target` 写成 libc 里一个「返回 -1 并置 errno=ENOSYS」
   的小桩（也放在新段里），而不是留 0；留 0 会让 `br x16` 跳到地址 0，现象是
   NULL 解引用，不好查。

顺序保证：`_dl_init` 钩子早于任何构造函数，也早于 `__libc_start_main`，
所以槽在使用前一定填好。

## 接管列表（第一批）

`open open64 openat openat64 __open_2 __open64_2 __openat_2 __openat64_2
stat stat64 lstat lstat64 fstatat fstatat64 statx statfs access faccessat
mkdir mkdirat rmdir unlink unlinkat rename renameat renameat2 symlink symlinkat
link linkat chmod fchmodat chown lchown fchownat truncate utimensat utimes
readlink readlinkat fopen fopen64 opendir creat creat64 realpath
canonicalize_file_name mkstemp mkstemps mkostemp mkostemps mkdtemp
getxattr lgetxattr statfs`

**不接管**：`exec*`、`fork`、`posix_spawn*`、信号函数（`sigaction`/`signal`/`kill`）、
`dlopen`、内存函数。这些一旦被 libc 内部重定向，语义和安全边界都会变。

## 与 NSS interpose 的关系

`getpwuid`/`getgrgid` 等 8 个身份函数已经在 shim 里实现（evidence 29），
它们不依赖本方案，先留着。等槽位方案落地、`files` NSS 能自己读 rootfs 之后，
可以再决定是否移除——保留没有坏处，二者读的是同一批文件。

## 验收

- `getent passwd`（若 rootfs 有）、`id`、`groups` 在**不**依赖身份 interpose 的情况下正确；
- `apt-get update` 能读到 `/etc/apt/sources.list`（libc 内部 fopen）；
- `locale -a` 能列出 rootfs 的 locale；
- `bash -c 'python3 -c "open(\"/etc/passwd\").read()"'`（若装了 python）——即 libc 内部 `_IO_file_open`；
- 全量 31 项功能矩阵与越界测试保持通过。
