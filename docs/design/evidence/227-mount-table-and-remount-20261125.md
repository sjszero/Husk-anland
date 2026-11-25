# 227 · 挂载表跨线程：锁是对的，缓冲和落盘不是；顺带 `mount -o remount,ro` 一直走不通

日期：2026-11-25 23:50（CST）。设备 UKHMMB8PYTGECMNV（RMX3357 / MT6893 / A13 / 4.19.191 / 4 KiB）。
结论分三段：**猜想被否掉的那一段**（表本身没有跨线程漏洞）、**真有两个洞**（读缓冲、落盘方式）、
**以及一条用户能直接撞上的 bug**（util-linux 的 remount）。

---

## 1 · 先查它怀疑什么：`mnt_read_file()` 会不会把表清零给别的线程看

怀疑来源：`mnt_read_file()` 里是 `MNT_N = 0` 再逐条重填，发布过程在 `MNT_LOCK` 里；
如果别的线程读表时**不持锁**，那它就能看到一段"表很空"的时间窗——表现为
「刚 mount 的东西在这一瞬间不存在」，路径掉回 rootfs 里的原目录。

**逐个访问点查完（FACT，代码逐行）**：全部在同一次持锁区间内。

| 位置 | 函数 | 持锁 |
|---|---|---|
| 2114 | `mnt_read_file()`（读 + 发布） | 起锁在 `MNT_LOCK`，`MNT_N = 0` → 解析 → 解锁 |
| 2191 | `mnt_purge_dead()` | 全程持锁（含 `mnt_save()`） |
| 2280 | `mnt_save()` | 无锁，但 4 个调用点（2239/2576/2674/2685/2723）都在锁内 |
| 2375 | `mnt_apply()`（主查找路径） | `mnt_ensure()` 之后 `mnt_lock()`，遍历 + 替换都在锁内 |
| 2429 | `mnt_is_ro()` | 同上 |
| 2455 | `mnt_unmap()` | 起手 `mnt_lock()` |
| 2579 | `mount()`（REMOUNT / 去重 / 追加） | 每段各自持锁 |
| 2700 | `umount2()` | 持锁遍历 + 移位 + `mnt_save()` |
| 3295 | `virt_mount_text()`（合成 `/proc/mounts`、`mountinfo`） | 持锁 |

**实测（FACT）**：探针 `mnt_race` —— 一个线程 200 000 次 `stat("/mnt/q/f")`（文件内容校验，
表撕裂会解析到别处），另一个线程在同一进程里反复 `mount tmpfs /mnt/u` + `umount`：

```
RESULT ok=200000 stat_fail=0 wrong=0 churns=46367      # 修前（pre-227 构建）
RESULT ok=200000 stat_fail=0 wrong=0 churns=46493      # 修后（最终构建）
```

46 000 次 mount/umount（≈9 万次 `mnt_save`）里零次异常。**这条怀疑不成立，表本身没有跨线程漏洞。**
写进 §3 的「核过」表，省得下一轮再查。

---

## 2 · 真有两个洞：读缓冲是共享的，落盘不是原子的

### 2.1 `static mut BUF`：两个线程往同一块内存里读（UB）

`mnt_read_file()` 原来是：

```rust
static mut BUF: [u8; 4096] = [0; 4096];
let n = syscall4(SYS_READ, fd, addr_of_mut!(BUF), BUF.len(), 0);   // 锁外
...
mnt_lock(); MNT_N = 0; /* 解析 BUF[..n] */
```

读在锁外，缓冲是静态的。两个线程同时进来会往同一块内存里写同一份内容——多数时候
**看不出来**。真出事的是「表文件在两次读之间被改小」（另一个线程刚 `umount` 并 `mnt_save`）：
先读的那次 `n` 更大，`n` 之外的尾部还是**上一版**内容，于是解析出一条已经不存在的挂载，
或者反过来漏掉一条。窗口小，但后果是"路径解析到错的地方"，不是变慢。

**修法：缓冲搬到栈上**，只把「解析 + 发布」留在锁里。

**为什么不是"读也放进锁里"**（试过，被实测否掉）：那种写法 4 线程路径解析从
9.08 µs/op 掉到 21.59 µs/op，**2.4 倍**——每次路径解析本来就 open+read+close 一次文件
（`NR_MNT_REFRESH` 默认开），把它串行化等于把整条热路径串起来。

```
# 同一轮、同一台设备、背靠背
读在锁外（pre-227）    th=4  400000 ops  3634 ms  9.08 us/op
读也在锁内            th=4  400000 ops  8636 ms 21.59 us/op     ← 2.4×，否决
栈缓冲（final）        th=4  400000 ops  3789 ms  9.47 us/op
```

栈缓冲第一版是 `[0u8; 4096]`，每次调用 memset 4 KiB，单线程比 pre-227 慢 2–4%
（25.49/26.62 对 24.58/25.18）。改成 `MaybeUninit` 之后消掉：

```
# 最终 A/B，两轮穿插（设备温度在这段时间里整体漂了约 15%，所以只看同轮对比）
round1  pre-227  th=1 25.23 / 24.77   th=4 10.64 / 10.78
round1  final    th=1 25.12 / 24.76   th=4 10.70 / 11.05
round2  pre-227  th=1 26.17 / 26.03   th=4 10.77 / 10.68
round2  final    th=1 26.74 / 26.66   th=4 11.35 / 11.36
```

单线程打平（±1%），4 线程在噪声内到 +5%（同一份二进制两轮之间也能差 1.7%）。
和 2.4 倍比起来可以接受。**单线程的绝对数字整体比本节开头高 15%，是热漂移，不是回归。**

### 2.2 `mnt_save()` 是 `O_TRUNC` 直写：读的一方能看到 0 字节或半张表

落盘原来是 `open(O_WRONLY|O_CREAT|O_TRUNC)` 再 `write`。而读的一方**每次路径解析都读这个文件**，
且读在锁外（见上）：正好读到 truncate 之后、write 之前的时刻，就拿到 0 字节或半张表。
0 字节会被 `n <= 0` 当成"没读到"而保留旧表（无害）；**半张表就是"刚挂上的东西在这次解析里不存在"**，
路径掉回 rootfs 里的原目录——正是 user 报的那类"东西不见了"。

**修法：写 `mounts.tmp` 再 `rename()` 覆盖**。同一目录内 rename 是原子的，读的一方
只会看到完整的新表或完整的旧表。临时文件建不出来（只读 rootfs / 配额满）时退回老写法，
不让"落不了盘"变成"表丢了"。

实测（FACT，`umount`/`mount`/表内容/残留文件一起查）：

```
mount -t tmpfs none /mnt/a → rc=0
/run/husk/mounts 内容： t	0	/mnt/a	/run/husk/t27644657664
/proc/mounts：          tmpfs /mnt/a tmpfs rw,relatime 0 0
bind /mnt/src → /mnt/b → rc=0，cat /mnt/b/s → srcfile
ls /run/husk/            → 只有 mounts 与 ipc，没有 mounts.tmp 残留
umount /mnt/b、/mnt/a    → rc=0，表清空
```

---

## 3 · 用户能直接用上的那条：`mount -o remount,ro <tgt>` 一直报 EINVAL

`mount -o remount,ro /mnt/b`（util-linux）：`rc=32`，
`mount: /mnt/src: mount point not mounted or bad option.`
而**同一组参数**从 C 里直接调 `mount("none","/mnt/b",NULL,MS_REMOUNT|MS_RDONLY,NULL)` 却成功。
说明差的不是 shim 的 remount 分支本身，是 util-linux 在进 syscall 之前自己改了参数。

加载 `NR_MNT_DEBUG=1`（本轮新加的调试口，打进 stderr）后一次就看清了：

```
mount src=none tgt=/mnt/src type=none flags=33      ← 33 = MS_REMOUNT(32) | MS_RDONLY(1)
mount: /mnt/src: mount point not mounted or bad option.
```

**根因（FACT + INFERENCE）**：util-linux 拿到 `/mnt/b` 之后会自己规范化——打开目录、
再 `readlink(/proc/self/fd/N)` 取回"这个名字"。模拟出来的 bind 在路径层是**同一个 inode 的两个名字**
（实测 `stat`：`/mnt/b` 与 `/mnt/src` 都是 dev=65024 ino=1454250），于是 readlink 回来的是**源**目录，
util-linux 就把 `/mnt/src` 当 target 交给了 mount(2)。真内核上这也是同一个 inode 的两个名字，
但真内核里 bind 是 VFS 层的挂载点，remount 走的是挂载点身份；我们这边只有路径匹配，所以匹配不上。

**顺带发现的第二个小口子（FACT）**：目标带尾部斜杠时也匹配不上——
`mount("none","/mnt/b/",NULL,MS_REMOUNT|MS_RDONLY,NULL)` 返回 `EINVAL`，而内核是接受 `remount /mnt/b/` 的。

**修法**（两处，都在 remount 分支里）：
1. 先把尾部 `/` 去掉再比（`kn` > 1 时才去掉，根 `/` 不动）。
2. 匹配时**同时认 `src`**：路径等于某条挂载项的源目录时，当成那条挂载点的 remount。
   这正是 util-linux 交过来的形状。

修复后（FACT，全部在设备上跑过）：

```
mount --bind /mnt/src /mnt/b                 rc=0
mount -o remount,ro /mnt/b                   rc=0
echo x >> /mnt/b/s                           rc=1  /mnt/b/s: Read-only file system
/proc/mounts                                 none /mnt/b none ro,relatime 0 0
mount -o remount,rw /mnt/b                   rc=0
echo y >> /mnt/b/s                           rc=0
mount -o remount,ro /mnt/b/   (尾部斜杠)      rc=0
mount -t tmpfs none /mnt/a; mount -o remount,ro /mnt/a   rc=0
echo z > /mnt/a/zz                           rc=1  Read-only file system
```

---

## 4 · 回归

- 门禁 A：`PASS=38 FAIL=0`（最终构建，装着 227 的全部改动）。
- `coverage-probe`：PASS（unlink/remove/inotify/freopen/pathconf/scandirat/truncate64/utime 全绿）。
- `coverage-sweep`：PASS，1379 条候选路径，没有未翻译的 guest 绝对路径。
- 挂载表并发回放 200 000 次 ×46 493 次 mount/umount：0 异常。
- `find /usr -name '*.so*'`：787–855 ms（与 221 记录的 948/952 ms 同一量级，本轮设备更热）。

---

## 5 · 记录在案、这轮不修的

| 项 | 说明 |
|---|---|
| 两个进程同时 mount | `MNT_LOCK` 是进程内的，跨进程是"后写覆盖先写"的丢更新。rename 之后每份快照至少是自洽的，但并发 mount 仍可能丢一条。要修得给表文件加锁（`flock`）并把读-改-写合成一次，属于行为改动，单列。 |
| `MNT_MAX = 8` | 容量还是 8 条，满了 `ENOSPC`（LIMITATIONS §13 已写）。 |
| bind 的两个名字无法从 inode 区分 | 真内核也一样；区别在于真内核 remount 认的是挂载点身份，我们只能认路径。已在 §3 用「同时认 src」绕开，不是通用解。 |
| `NR_MNT_DEBUG=1` 的日志走 stderr | 只为排障存在，不进正常路径（不设变量时只多一次 `env_lookup`）。 |
