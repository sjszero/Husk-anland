# 219 · 两个真缺口收掉：`dlmopen`、`execveat`

日期：2026-11-25 22:40–22:50（CST）。设备：RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB。

## 0 · 结果

| 项 | 修前（FACT） | 修后（FACT） |
|---|---|---|
| `dlmopen(LM_ID_BASE, "/usr/lib/…/libm.so.6", RTLD_NOW)` | `NULL`，`dlerror()` = `cannot open shared object file: No such file or directory` | 句柄非空，`dlerror()` = `(none)` |
| `dlmopen(LM_ID_NEWLM, 同一路径)` | 同上（失败） | 新命名空间句柄，`(none)` |
| `dlmopen(LM_ID_BASE, "libm.so.6")`（裸 soname） | 正常 | 正常（仍交给 ld.so 搜索） |
| `execveat(AT_FDCWD, "/usr/bin/true", …)`（按符号调用） | 子进程 `exit=97`（没跑起来，`ENOENT`） | 子进程 `exit=0`（真的 exec 成功） |
| `syscall(SYS_execveat, …)`（raw syscall） | `exit=97` | `exit=97`（绕过 shim，设计边界，不是 bug） |
| 覆盖度 | 96 / 111 | **98 / 111** |
| 门禁 A | 38 / 38 | **38 / 38**（无回归） |
| `python3 -c "import numpy"` | 1.26.4 | 1.26.4（dlopen 路径没被改坏） |
| `gcc` 编译 hello | ok | ok（collect2 用 dlopen 装 LTO 插件） |

## 1 · `dlmopen`：钩子会撞自己

`dlopen` 一直是被钩的（先翻译路径再交给 glibc），`dlmopen` 不是，于是 glibc 的
`ld.so` 自己按宿主路径 open。补钩子时碰到一个自指问题：

**一旦 shim 导出 `dlmopen`，`dlopen` 内部那句 `dlmopen(LM_ID_BASE, …)` 就会解析到自己身上，
无限递归。** 解决办法不是特判，而是把「libc 那一份」的地址显式取出来：

- `boot.rs` 新增 `nr_resolve_name_skip(main_map, name, len, skip)`：走 link map 链找符号，
  **跳过等于 `skip` 的命中**；
- shim 里 `real_dlmopen_addr()` 惰性解析一次，`skip = dlmopen as usize`（我们自己的地址）；
- 解析失败就返回 `ENOSYS`，不假装成功——宁可让调用方看到错误，也不给它一个假句柄。

顺带把 `dlopen` 和 `dlmopen` 合成一条路径：`dl_translate()` 负责「裸 soname 交给 ld.so、
带 `/` 的路径翻成 rootfs 里的真路径」，两个入口只差一个命名空间号。

## 2 · `execveat`：折成一次 `execve`

`execveat` 不经过 `execve` 钩子，路径原样交给宿主内核。它的关键不是翻译路径，
而是**exec 本身必须走 loader 重入**：guest 的动态 ELF 的 `PT_INTERP` 是宿主上没有的
`/lib/ld-linux-aarch64.so.1`，光把路径翻对也起不来。

所以实现是「折成一次 `execve`」：先按 `dirfd` + 相对路径解析成 guest 绝对路径
（`resolve_dirfd_rel`），再调自己的 `execve` 钩子——loader 重入、`#!` 解释器改写、
库路径修正、`/proc/self/exe` 发布一次全齐。

`AT_EMPTY_PATH`（执行一个已打开的 fd）表达不成路径，直接 `ENOSYS`，让调用方回退。

## 3 · 探针本身的两个坑（都踩过）

1. **用 `syscall(SYS_execveat, …)` 测钩子**：raw syscall 本来就绕过整个 shim，
   测出来永远是 `ENOENT`。必须按符号调用（走 GOT）。这条同时是**反证**：
   raw syscall 的边界是真实存在的，不是文档措辞。
2. **成功的 exec 不会返回**：探针里直接调会把探针自己替换掉，看起来像"程序挂了"。
   改成 `fork` + `waitpid` + 读退出码。

## 4 · 顺带核掉的两个「疑似 bug」

审计清单里挂着两条，实测/读码后**不成立**，记在这里免得下次再查：

| 项 | 结论 |
|---|---|
| `MNT[]` 写入前没查容量 | **不成立**：`mount()` 里 `if MNT_N >= MNT_MAX { mnt_unlock(); return set_errno(28) }` 就在 `MNT[MNT_N] = e` 前面；读文件那条路径的循环条件本身是 `MNT_N < MNT_MAX`（FACT，代码在 `lib.rs` `mount()` / `mnt_read_file()`） |
| `elf_interp()` 里 4 个 `try_into().unwrap()` 会 panic | **不成立**：四个转换的源都是固定长度常量区间的切片（`eh[32..40]`、`eh[54..56]`、`eh[56..58]`、`ph[0..4]`，底层是 `[u8; 64]` / `[u8; 56]`），`try_into` 到定长数组不可能失败，编译期就定死了（FACT，读码） |
