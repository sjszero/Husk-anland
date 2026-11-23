# 挂载：一个没有 CAP_SYS_ADMIN 的命名空间能给出什么

> 目标段落：B（挂载与文件系统）。设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB / `untrusted_app`。
> 状态：bind / tmpfs / 只读 bind 可用，上机验收见 evidence 112。

## 1. 先说清楚做不到什么

`mount(2)` 需要 `CAP_SYS_ADMIN`，本域没有（`CapEff=0`）。所以：

- 没有真正的挂载、没有 mount namespace、没有传播（shared/slave/private 都是空谈）；
- 没有 overlayfs、没有 loop 设备、没有 NFS；
- 不能给 tmpfs 真配额（内核不参与，没人执行）。

这些不是"以后补"，是平台边界，跟容器里非特权进程的处境相同。

## 2. 容器用户态真正观察到的三件事

前人的实现把它们分得很清楚，我们的做法就是按这三条对齐：

| 观察面 | 容器的表现 | 我们的做法 |
|---|---|---|
| 路径语义 | 绑定点之下的路径指向源 | 路径层做最长前缀替换（`abs_guest` 之后、翻译之前） |
| 目录内容 | tmpfs 初始为空，进程间共享 | 在 `/run/husk` 下分配一个 scratch 目录，挂载点指过去 |
| 写权限 | `-o ro` 的挂载点写入报 `EROFS` | `openat` 写标志 + 所有改树操作（mkdir/unlink/rmdir/mknod/symlink）前置检查 |
| 可见性 | `/proc/mounts`、`findmnt` 能看到 | 合成表追加条目（`virt_mount_text`） |

## 3. 读过什么（目标第 0 条）

| 来源 | 版本 | 拿走了什么 | 不采用的部分 |
|---|---|---|---|
| `containers/youki` | `f330e83`（2026-11-23） | `crates/libcontainer/src/rootfs/mount.rs` 的挂载清单与顺序：bind 用 `MS_BIND[|MS_REC]`，改标志必须 `MS_BIND|MS_REMOUNT`，cgroup 子系统的"emulated"分支说明别人也会在不能真挂时退到别的表示 | pivot_root、propagation、fsopen/fsmount：全都要特权 |
| `containers/bubblewrap` | `1fb0aa2`（2026-10-14） | `bind-mount.c`：递归 bind 后再 remount 改 flags 的两次调用模式；这就是 util-linux `mount --bind -o ro` 会发两次 `mount(2)` 的原因，我们的幂等判断按这个写 | 它自己也是靠 userns + mount ns，不是我们能走的路 |
| `containers/fuse-overlayfs` | — | 用户态 COW 的唯一现实路线（FUSE），留作以后做镜像分层时用 | 现在不需要 |
| `dex4er/fakechroot` | 315★ | 路径改写的边界用例清单 | 它是 LD_PRELOAD 全量拦截，我们是 loader 内一次改写 |
| `lxc/lxcfs` | `e1c5a7d` | 容器 `/proc` 该由谁合成、哪些条目需要"自己的视角" | FUSE 方案；我们已经有自己的合成层 |

## 4. 设计

- 表存在 `/run/husk/mounts`（每行 `b|t \t ro \t target \t source`），进程内有一份缓存；
- `mount()` 只接受两种形式：`MS_BIND`（源必须存在）与 `-t tmpfs`（分配 `/run/husk/tmp<N>`）；
  其它类型返回 `ENODEV`——不假装支持；
- `MS_REMOUNT` 改已有条目的只读位；同目标的第二次 bind（util-linux 的惯用两步）按幂等处理；
- `umount2()` 按目标精确匹配删除，目标用**未套用映射**的原始路径解析（否则会被改写成源）；
- 解析顺序：guest 绝对路径 → 应用挂载表（最多 4 跳，防环）→ 翻译成宿主路径。

## 5. 已知限制（都写进证据，不假装）

1. **同进程可见性**：挂载由进程 A 做，进程 B 启动后才看得到。已经在跑的进程（例如做重定向的 shell 自己）
   看不到新挂载，它 exec 出来的子进程能看到。容器里没有这个差别（内核全局）。
2. 最多 8 个挂载点，单条路径 ≤128 字节；超出返回 `ENOSPC`/`ENAMETOOLONG`。
3. tmpfs 的 `size=` 不执行；`df` 报的是底层文件系统。
4. 只读判定只覆盖 `openat` 写标志与改树系统调用；`chmod/chown/utimensat` 目前不做 ro 检查（待补）。
5. 没有传播语义，挂载不会被"继承"到子命名空间（我们本来也没有子命名空间）。

## 6. 验收（evidence 112）

- `mount --bind /tmp/src /mnt/data` → 目标下读到源内容
- `mount -t tmpfs tmpfs /mnt/ram` → 子进程写入并读回
- `mount --bind -o ro /tmp/src /mnt/ro` → 读成功，写 `Read-only file system`
- `/proc/mounts` 与 `findmnt` 列出三条
- `umount` 两条都成功，表项消失，目标目录恢复为空
