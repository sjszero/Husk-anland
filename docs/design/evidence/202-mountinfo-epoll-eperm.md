# 202 · `/proc/self/mountinfo` 的 `Operation not permitted` 定根因：虚拟 proc 文件是 memfd，不可 epoll

日期：2026-11-25 16:30–16:50（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`。
这一份替代 0.0.6 目标里的第 ③ 项（"给 Nautilus 46 内容区空白定根因"），并给出一个
**否定结论**：这条报错不是内容区空白的原因。

## 0 · 一句话

`GLib-GIO-WARNING: Error creating IO channel for /proc/self/mountinfo: Operation not permitted`
的唯一来源是：Husk 把虚拟 `/proc` 文件实现成 **memfd**，memfd 是普通文件，
而 libmount 的内核挂载监视要做 `epoll_ctl(ADD)`——**内核对普通文件一律回 EPERM**。
⇒ 挂载变化通知失效（不是内容区空白的原因）。

## 1 · 报错来源定位到行（FACT）

1. 字符串在 guest 里只存在于一个库：
   ```
   distributions/.../usr/lib/aarch64-linux-gnu/libgio-2.0.so.0
   ```
2. glib 2.80 `gio/gunixmounts.c:2049-2103`（`mount_monitor_start()`）：
   `HAVE_LIBMOUNT` 分支先 `mnt_monitor_enable_kernel()`，再 `mnt_monitor_get_fd()`；
   返回负数时 `g_set_error_literal(errno)`，随后打出
   `Error creating IO channel for %s: %s (%s, %d)`（第 2079 行）。
   ⇒ 报错里的 errno 就是 libmount 的那个 `-rc`。
3. util-linux `libmount/src/monitor.c`：内核监视的全部动作是
   ```
   fd  = open("/proc/self/mountinfo")     /* monitor_kernel_init */
   epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &ev)  /* monitor_modify_epoll，第 576 行 */
   ```
   源码注释（第 527 行）自己写着这条路依赖 `/proc` 文件的 poll 语义。
   ⇒ **唯一可能返回 EPERM 的系统调用就是 `epoll_ctl`**。

## 2 · 实测：所有虚拟 proc 文件都不可 epoll（FACT）

`rootfs/tmp/ep.c`（一次性探针，不入库）：open → fstat → `epoll_ctl(ADD)` → `poll` → `read`。

```
/proc/uptime           reg=1 mode=0777 dev=0:5 size=15    epoll_ctl=-1(Operation not permitted) poll=1 rev=1 read=15  target=/memfd:husk-virt (deleted)
/proc/self/mountinfo   reg=1 mode=0777 dev=0:5 size=428   epoll_ctl=-1(Operation not permitted) poll=1 rev=1 read=256 target=/memfd:husk-virt (deleted)
/proc/self/mounts      reg=1 mode=0777 dev=0:5 size=350   epoll_ctl=-1(Operation not permitted) poll=1 rev=1 read=256 target=/memfd:husk-virt (deleted)
/proc/version          open: No such file or directory
/etc/mtab              open: No such file or directory
```

三点：

1. `S_IFREG`、`dev 0:5`、`target=/memfd:husk-virt (deleted)` ⇒ **是 memfd，不是 procfs**。
2. `poll()` 对普通文件**永远返回就绪**（rev=1），所以"能 poll"是假象；`epoll_ctl` 才是硬门槛。
3. 不是 mountinfo 独有：`/proc/uptime` 同样 EPERM。**这是虚拟 proc 的整体属性**，不是某个文件的 bug。

代码侧对得上（`runtime/crates/path-shim/src/lib.rs:3693-3712`）：`virt_open()` 里
`SYS_MEMFD_CREATE` 建文件、写完内容、返回 fd。

## 3 · 后果（FACT + INFERENCE）

glib 的代码结构决定了后果：`mount_monitor_start()` 里 `/proc/` 分支拿不到 channel 时，
**既不挂 watch source，也不退回轮询**——那个 3 秒轮询（第 2113-2126 行
`g_timeout_source_new_seconds (3)`）只属于"根本找不到 mtab 路径"的另一条分支。
所以：

- `GUnixMountMonitor` 的 `mounts-changed` / `mountpoints-changed` **一次都不会发**（FACT，源码结构）。
- 受影响的进程在日志里都点名了：`udisksd`、`gvfsd`（每个实例）、`nautilus`、
  dbus 激活的 `libld.so` 进程——**凡是链接 libgio 又会看挂载表的进程都会中**（FACT）。
- 实际影响面：挂载表变化不再有通知，只能等下一个人主动读 `/proc/self/mountinfo`
  （INFERENCE）。在我们这个环境里挂载表本来就是静态合成的（见下），所以用户可见差异很小。
- **和内容区空白无关**（FACT）：空白那次会话的链条是 root 降级 + gvfs 回退（见 197 / 200），
  本次同一环境里 Nautilus 内容区正常列出。

## 4 · 我们合成出来的挂载表长什么样（FACT）

```
20 1 0:30 / / rw,relatime - f2fs .../rootfs rw
21 20 0:5 / /proc ...
22 20 0:6 / /sys ...
23 20 0:7 / /dev ...
24 23 0:8 / /dev/pts ...
25 23 0:9 / /dev/shm ...
```

6 行，静态，**不随假挂载变化**。所以即使把 epoll 这条路修通，也不会真的有事件可发——
真要发，得同时给 `virt_open` 加一份"内容变了就敲一下"的通知。

## 5 · 三条修法，以及为什么 0.0.6 都不做（JUDGMENT）

| 方案 | 做法 | 代价 / 风险 |
|---|---|---|
| A | 虚拟 proc 文件改 pipe 承载（open 时建 pipe、写满内容） | 可 epoll 了，但 `lseek`/`fstat.size` 全变 0；依赖 `stat` 大小预分配或回绕读的消费者（procps、部分 udisks 路径）会看到"空表"——**风险正好落在我们最怕的"空列表"上** |
| B | 在 shim 里 hook `epoll_ctl`，对我们的 fd 伪造成功 | 危险：GLib 的 `data.ptr` 是 GSource 指针，改写 `data.fd` 会破坏主循环；只伪造成功等于"永远不通知"，掩盖而不解决 |
| C | 不动，记录 | 代价是 4 个进程各一行 warning + 挂载变化无通知 |

0.0.6 选 C。理由：这条链路**不产生错误行为**，只产生日志噪音和"变化不通知"；
而 A 的风险恰好是"挂载表被看成空"，会把一个噪音问题换成一个内容区空白的风险。
真要修，正确顺序是先在 A 上把每个消费方（`mount`、`df`、`udisksd`、`gio`）
的读法逐个验过，再连同"内容变化敲通知"一起做——那是 0.0.7 的活，不是发版前的活。

## 6 · 复现

```sh
sh xe-plain.sh usr/bin/gcc -O2 -o /tmp/ep /tmp/ep.c
sh xe-plain.sh usr/bin/env /tmp/ep
```
