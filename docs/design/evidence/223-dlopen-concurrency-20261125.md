# 223 · 并发 `dlopen` 会打死进程：链接映射一边遍历一边被 unmap

日期：2026-11-25 23:20–23:35（CST）。设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB。
审计面：loader 侧——新加载的对象什么时候被补 GOT，以及补的时候谁在动同一条 link_map。

## 0 · 结论

| 问题 | 性质 | 现状 |
|---|---|---|
| `REAL_DLMOPEN` 用「读一次 0 就解析」的裸静态懒解析 | 并发下 dlopen 返回 NULL 且 `dlerror()` 为空（解析失败被当成 ENOSYS） | 已修：CAS 解析一次，输的线程等赢家发布 |
| `REPATCH_LOCK` 是 check-then-set（`if != 0 return; = 1`） | 两个线程可以同时进 `nr_repatch`，同时改 `REPATCH_SEEN` / 同一批 GOT 槽 | 已修：CAS，输的线程跳过这一轮 |
| `nr_repatch` 遍历 link_map 时不持任何锁，而 `dlclose` 会把对象摘链、unmap | **SIGSEGV**（实测 10 次里 2 次） | 已修：dlopen/dlmopen/dlclose 用同一把进程内锁串起来 |

## 1 · 探针

`8 线程 × 60 轮 dlopen → dlsym → dlclose`，两个库交替（`libm.so.6` / `libz.so.1`），
全部走 guest 的 `dlopen` 符号（GOT 路径，不是 raw syscall）：

```c
const char *libs[2] = {"libm.so.6", "libz.so.1"};
const char *syms[2] = {"cos", "zlibVersion"};
for (r = 0; r < 60; r++)
  for (i = 0; i < 2; i++) { h = dlopen(libs[i], RTLD_NOW);
                            if (!h) fail(); else dlsym(h, syms[i]); dlclose(h); }
```

## 2 · 修前：三种失败，两种原因

连续 3 次（FACT）：

```
threads=8 rounds=60 fails=0
segv|sig=11|fa=536072421904|pc=536126722436|lr=536090312704|...   ← 第 2 次
threads=8 rounds=60 fails=0
```

再连 8 次，出现第二种（FACT）：

```
dlopen libm.so.6: (null)      ← 返回 NULL，而 dlerror() 是空的
dlopen libm.so.6: (null)
threads=8 rounds=60 fails=3
```

`dlopen` 返回 NULL 却不给错误串，只可能是我们自己的 `call_real_dlmopen` 里
`real_dlmopen_addr() == 0` 那条 ENOSYS 分支——也就是说**原版 dlmopen 的地址没解析出来**。
它用的是裸静态懒解析：

```rust
static mut REAL_DLMOPEN: usize = 0;
if REAL_DLMOPEN == 0 { REAL_DLMOPEN = nr_resolve_name_skip(…); }
```

8 个线程同时进来，8 个同时走 `nr_resolve_name_skip` 遍历 link_map；期间别的线程在
dlclose 里摘链、unmap，遍历读到垃圾 → 有时拿到 0，有时直接 SIGSEGV。

`REPATCH_LOCK` 是同一类错误（check-then-set 而不是原子操作），已经用 CAS 改掉；
但**只改这两个还是崩**（10 次里 2 次），所以下面那条才是主因。

## 3 · 主因：repatch 与 dlclose 之间没有锁

`_dl_init` 之后加载的对象要重新补 GOT（`nr_repatch`），它做的是：从 `MAIN_MAP` 起沿
`l_next` 走完整条链，对没见过的对象调 `patch_object`。这件事发生在 `dlopen` 返回**之后**，
此刻 glibc 的装载锁已经放开——别的线程的 `dlclose` 可以把引用归零的对象摘链并 unmap，
我们手上的 `l` 指针就成了悬空指针。

判据（FACT，bisect）：同一份 loader、同一份探针，只加 `NR_NO_REPATCH=1`：

```
NR_NO_REPATCH=0（默认）: 15 次里 2 次 segv
NR_NO_REPATCH=1        : 12 次 0 失败
```

**修法**：`dlopen` / `dlmopen` / `dlclose` 三个入口共用一把进程内锁（`DL_ACT`），
`nr_repatch` 只在持锁期间跑；`dlclose` 因此**第一次被钩进来**（它没有路径参数，钩它纯粹
是为了让「摘链接 / unmap」和别人的遍历互斥）。递归用 owner tid + 深度计数放行——
构造函数里再 `dlopen` 很常见，不放行就会等自己等到超时。等不到锁（40M 次自旋）时
退化成不持锁执行，宁可保留原来的小概率竞态，也不把进程挂死。

装载器数字：`interposed` 195 → **196**（多出来的就是 `dlclose`）。

## 4 · 修后

```
runs=15 failed=0   × 3          → 45 / 45 全过
门禁 A                          → PASS=38 FAIL=0
```

## 5 · 这一条对桌面的意义

`gnome-shell`、`gdk-pixbuf` 的 loader、GIO 模块、Mesa 的 ICD 都是运行时 `dlopen` 进来的，
而且不保证只在一个线程里做。修前这种崩溃在现场表现为「桌面用着用着整个进程没了」，
日志里只有我们 SIGSEGV 处理器打的那一行十六进制。现在这条路径有锁，
`NR_NO_REPATCH=1` 仍然保留，用于下次二分。

## 6 · 未做（UNKNOWN）

- glibc **内部**的 `dlopen`/`dlclose`（`__libc_dlopen_mode`、NSS 模块、iconv 模块）不经过
  我们的符号，也就不进这把锁。它们和我们 repatch 撞车的窗口理论上还在，没有观测到，
  也没有构造出复现。
- 锁是自旋锁，持锁期间会调 libc 的 dlopen（可能做 IO）。拿不到锁的那 40M 次自旋在
  慢设备上是一段可见的忙等；实测 15 轮 × 60 次 dlopen 的探针里没有触发过。
