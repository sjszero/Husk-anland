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

1. 追加两个页：

   > 位置有硬约束（2026-11-23 修正，evidence 122）：**存根页必须在 `.text` 之后的零填充里**
   > （被搬走的窗口里有 `b.cond`/`cbz`/`tbz`，射程只有 ±1 MiB / ±32 KiB），
   > **槽位页必须追加到文件末尾**。原因是 glibc 的 `dl-load` 会拒绝 `p_align` 不是页大小整数倍的
   > `PT_LOAD`（`ELF load command alignment not page-aligned`）；这个 libc 的零填充只有 0x323f 字节，
   > 塞不进 64 KiB 对齐的段，而放进洞里的段只能声明 4 KiB，到了 16 KiB 内核就整个加载失败。
   > 放到文件末尾后 `p_offset` 和 `p_vaddr` 都能取 `0x10000` 的整数倍，4K/16K/64K 都合法。
   > 入口用 `adrp`（±4 GiB）取槽位页，所以槽位页离 `.text` 多远都行。

   * **slot 页**（`RW`，复用一个 PT_NOTE，**追加在文件末尾**，`p_align = 0x10000`）：
     头 + 表，`target` 初值 0。

     ```
     struct husk_slots {
         u8   magic[12];   // "HUSK-SLOT-1\0"
         u32  count;
         u32  stride;      // 48，表项定长，运行时按它走
         struct { char name[32]; u64 target; u64 stub_delta; } e[count];
     };
     ```

     `name` 与 shim 导出符号一字不差，`stub_delta` 是该函数 stub 在 stub 页内的偏移。

   * **stub 页**（`R|X`，扩进 EXEC LOAD）：每个函数一段，内容是它被覆盖掉的那 6 条指令，
     末尾再加一条 `b 入口+24` 跳回去。

2. 对每个要接管的函数（列表见下），把入口前 24 字节改成：

   ```
   adrp x16, <slot page>            ; 4 字节
   ldr  x17, [x16, #<target>]       ; 4 字节，运行时填的 shim 地址
   cbnz x17, #12                    ; 4 字节，填了就跳最后一条
   ldr  x17, [x16, #<stub_delta>]   ; 4 字节，stub 是相对槽页的偏移
   add  x17, x16, x17               ; 4 字节
   br   x17                         ; 4 字节
   ```

   两条路径都是必须的：guest loader 在 `_dl_init` 之前就会调 libc（重定位、`open`、`mmap`），
   那时槽还是 0，只能走 stub 里的原指令。用 `adrp` 而不是 `ldr #imm` 字面量：字面量 ±1MB，
   而新页可能在文件末尾，和 `.text` 的距离可能超过 1MB；`adrp` 是 ±4GB。

   搬走的 6 条指令要**重新编码**所有 PC 相对指令（`adrp/adr`、`ldr` 字面量、`b/bl`、
   `b.cond`、`cbz/cbnz`、`tbz/tbnz`）。搬不过去的就按名字拒绝，不硬改。

3. 够不着的条件分支要**展开**，不能拒绝：glibc 里不少函数开头就是 `b.ne`/`cbz` 去 ±1MB 外的
   错误分支，搬到 stub 页后 19 位 immediate 装不下（`mq_unlink` 就是 `-1085120`）。做法是把它
   换成两条：**反条件跳过下一条 + 全宽 `b`**（±128 MiB）。stub 因此变长，页内按实际长度紧凑排布，
   不再用固定 stride。这一改动把 `mq_unlink`、`__open_2`、`__openat_2`、`__openat64_2`、
   `renameat2`、`mkstemps`、`mkostemps` 一并救了回来（74/87 → 82/87，evidence 114）。

4. 校验：入口必须是 4 字节对齐的函数起点（`st_value` 来自 `.dynsym`）、这块窗口里不能有别的符号
   起头、也不能是任何分支的目标；stub 页必须落在零填充里（写完前逐字节检查）。

### 运行时（shim，`_dl_init` 钩子里）

1. 从 link map 找到 `libc.so.6`，读它自己的 ELF header（`l_addr` 处 `e_phoff/e_phnum`）。
2. 遍历 PT_LOAD，找到以 `HUSK-SLOT-1` 开头的段（内容已经映射，直接读）。
3. 对每个条目按名字在 **shim 自己的 interpose 表**里查地址，写进 `target`。
   只写 RW 数据段，不需要任何 `mprotect`。
4. 任何名字在 shim 侧找不到 → `target` 留 0。留 0 不是错误：入口会掉进 stub 分支，
   原样执行那 6 条指令，行为与没打补丁完全一致（这也是 loader 自举期间需要的）。

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
