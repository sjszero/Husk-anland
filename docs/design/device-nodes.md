# 设备节点面：哪些能做、哪些是平台墙

> 目标：guest 里的程序按名字打开 `/dev/*` 时得到和普通 Linux 一样的行为。
> 前提：不 root、不解 BL、不开虚拟化、零 ptrace、不扩大拦截面。设备 RMX3357 / 4.19.191 /
> `untrusted_app`。结论全部按实测，推断标注。

## 1. 现在能用的（FACT）

| 节点 | 行为 | 来源 |
|---|---|---|
| `/dev/null` | 读空、写吞 | 宿主节点直通 |
| `/dev/zero` | 读全 0 | 宿主节点直通 |
| `/dev/urandom` `/dev/random` | 出随机字节 | 宿主节点直通 |
| `/dev/shm` | 可读写目录（POSIX shm、`/dev/shm` 语义） | 宿主节点直通 |
| `/dev/pts/*` | PTY 从端（交互会话、`script`、`expect`） | 宿主 devpts 直通 |
| `/dev/mali0` | rw 打开，kbase UAPI 11.38 | 宿主节点直通（GPU 路线见 gpu-drm-shim.md） |
| `/dev/videoN` | V4L2 取流、拍照、录像 | 相机桥（evidence 91–93） |
| `mkfifo` | 可见的 FIFO 节点 | shim 的 `mknodat` 翻译 |

## 2. 三条平台墙（FACT / JUDGMENT 分开写）

### 2.1 `/dev` 不能列目录 —— 平台限制

```
$ run-as com.linuxhub.myth.husk sh -c 'ls /dev'      # 不经过 loader，纯 app 身份
ls: /dev: Permission denied
$ ls -ld /dev
drwxr-xr-x 25 root root 5320 /dev
```

目录权限是 `o+rx`，但 SELinux 仍然拒。**这不是 shim 的过滤**：不经过 loader 的普通 shell
一样被拒。FACT：本域列不了宿主 `/dev`。

后果：`ls /dev`、扫描 `/dev` 找设备的程序（部分 udev 遗留逻辑、`lsusb` 变体）拿不到列表。
按名字 `open()` 完全正常，所以真正干活的程序不受影响。

要绕开只能在 guest 里materialise 一个自己的 `/dev` 目录再用 bind 把节点填进去（youki/crun
就是这么做的）。**没做**：现在靠直通的节点全部可用，重建 `/dev` 收益小于"把能用的弄坏"的风险。
等 2.0 集成、需要给用户一个可浏览的 `/dev` 时再做，形状写在这里。

### 2.2 `/dev/full` 写要 `ENOSPC` —— 不做，理由明确

目标里写过"`/dev/full` 返回 `ENOSPC`"。做不到且不该做的原因：

* 节点建不了（没有 `CAP_MKNOD`，也不该有）。
* 要造 `ENOSPC` 只能拦 `write(2)`。这是**全系统最热的调用**，而且
  glibc 的 stdio 走内部 `__write_nocancel`（`fwrite`/`fprintf` 的目标就在这里），
  只拦导出的 `write` 会漏掉所有带缓冲的写 —— 结果比不做更糟：一半程序看到 `ENOSPC`、
  一半看到成功。
* 与硬约束"零 ptrace、不扩大拦截面"直接冲突。

结论（JUDGMENT）：记为**不支持**，不是待办。真的需要它的程序极少，而且 `write` 到
`/dev/full` 得到的 `ENOSPC` 在应用里基本只用于"磁盘满"测试。

### 2.3 `/dev/tty` 打开是 `ENXIO` —— 缺"控制终端"，不是缺节点

```
$ ls -l /dev/tty        # 宿主节点，直通得到
crw-rw-rw- 1 root root 5, 0
$ echo hi > /dev/tty
/dev/tty: No such device or address      # ENXIO
```

`ENXIO` 的准确含义是"这个进程没有控制终端"，不是节点不存在。要修必须在**起会话的人**那边
做 `setsid()` + `ioctl(TIOCSCTTY)`，那是终端载体（app-rs 的会话建立）的职责；proot 在父进程
没有控制终端时同样是这个结果。

读 `sudo` 密码、`less`、`ssh` 的 askpass 这类程序会摸 `/dev/tty`；在本项目里它们应当走
stdin/stdout（PTY 已经工作）。等 app-rs 建立会话时补 `TIOCSCTTY` 即可，属于终端载体项，
不是运行时内核项。

## 3. 验收

`tools/device-tests/10-device-nodes.sh`：按名字打开每个节点并验行为；
`/dev` 列目录与 `/dev/full` 作为**已知不支持**断言（EACCES / 不是 ENOSPC），
这样它们一旦变化（平台放宽或我们改设计）测试会立刻响。
