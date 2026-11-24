# 166 · 假 DRM 节点的 fd 记账：dup 与 close 两个漏洞（自查，非抄）

日期：2026-11-24（CST）。设备：RMX3357（MT6933/Mali-G77 与本文无关，见 STATUS），
包 `com.linuxhub.myth.husk`，Android 13 / kernel 4.19.191 / 4 KiB。

## 0 · 结论

| 项 | 旧 loader | 新 loader | 类别 |
|---|---|---|---|
| `dup()` 出来的渲染节点 fd | `rdev=0:0 mode=100777`（就是 memfd） | `rdev=10:33 mode=020666` | FACT |
| `dup2()` / `fcntl(F_DUPFD)` 同上 | 同上，未被接管 | 同上，全部接管 | FACT |
| `close()` 是否注销 DRM 假 fd | 不注销（`FAKE_FDS` 8 槽只进不出） | 注销，`close_range` 也清区间 | FACT |
| 假 fd 槽位耗尽时的行为 | 静默不记账，ioctl/mmap 落到 memfd | 记一条 `nrshim drm: fake fd slots exhausted` | FACT |
| X11 窗口化 EGL（回归） | PASS | **PASS**（`stage 7`→`10`，60 swap，`zink Vulkan 1.3(Mali-G77 MC9)`） | FACT |

## 1 · 这两条是怎么被找出来的（方法）

看 Portal 的 `drmshim.c` 头注释时，它列了四条"用失败换来的"约束。其中一条写着
**"不要 interpose `close()`，裸 SVC 替 close 会让 cat/python 在一次成功 read 之后 EFAULT"**。
原文的前提是 **PRoot**：ptrace 接管每条 syscall，替掉 close 会打乱它的记账。

Husk 不是 PRoot（零 ptrace、运行期不改任何 syscall，只在装载时改一次 GOT），所以这条
不能直接照做，必须逐条换成"我们的架构要满足什么"再审计。审计结果：

- 我们**早就** interpose 了 `close()`（`path-shim/src/lib.rs`，保 rootfd、注销 v4l2/netlink 假 fd），
  并且在生产路径里跑过 882 包 apt、600 秒长稳、门禁 A 34/34 —— PRoot 那条现象在零 ptrace 下没有机理。
- 但顺着这条追下去，发现了**我们自己的**两个真漏洞：
  1. `close()` 注销了 v4l2 / netlink 的假 fd，**唯独没有注销 DRM 的**；
  2. 我们没有 interpose `dup`/`dup2`/`dup3`，`fcntl` 也不认 `F_DUPFD`。

## 2 · 漏洞 1：假 fd 只进不出

`FAKE_FDS` 是 8 槽，`register_fake_fd()` 只在 `drm_fake_open()` 里调用。
一个进程反复 open/close `/dev/dri/renderD128`，第 9 次开始的 open 就不被记账了，
它的 `ioctl`/`mmap` 会落到 memfd 上（客户端表现为 DRM 版本查询失败 → 软件渲染）。

修法：`close()` 里加 `drm_forget(fd)`，`close_range()` 里加 `drm_forget_range(first,last)`；
槽位满时打一条 `nrshim drm: fake fd slots exhausted`，以后再泄漏至少是**可见的**。

## 3 · 漏洞 2：dup 出来的 fd 不是假 fd

Mesa 拿渲染节点不是只用一次：它会 `fcntl(F_DUPFD)` 复制一份、在副本上问
`DRM_IOCTL_VERSION`，再按回复里的驱动名选 winsys。副本不在表里，就不被接管。

实测（同一台机器、同一 rootfs、只换 loader）：

```
# 旧 loader（libld-sysfs.so）
orig         rdev=10:33 mode=20666
dup          rdev=0:0   mode=100777      <- memfd 本身，不是设备
dup2:60      rdev=0:0
F_DUPFD      rdev=0:0

# 新 loader（libld-drmfd.so）
orig         rdev=10:33 mode=20666
dup          rdev=10:33 mode=20666
dup2:60      rdev=10:33 mode=20666
fd60         rdev=10:33
F_DUPFD      rdev=10:33 mode=20666
```

修法（全部 Rust，`path-shim/src/lib.rs`）：
`dup`（`SYS_DUP=23`）、`dup2`（转 `SYS_DUP3`，`dup2(fd,fd)` 按 POSIX 直接返回 fd）、
`dup3`，以及 `fcntl` 的 `F_DUPFD`/`F_DUPFD_CLOEXEC` 分支 —— 都调 `drm_inherit_fake(old,new)`。

### 3.1 顺带查出的第三处：`fcntl64`

只加 `dup` 之后**仍然不生效**（`dup` 那一行还是 `0:0`）。原因：CPython 的 `_Py_dup` 走的是
`fcntl(fd, F_DUPFD_CLOEXEC)`，而 glibc 在 LP64 上把 `fcntl` 与 `fcntl64` 导出成**两个符号**，
python 落到了 `fcntl64`，我们只 interpose 了 `fcntl`。

补上 `fcntl64` 之后三条路径才一起变成 `10:33`。loader 的 interpose 表从 177 涨到 178。

## 4 · 回归

```
$ sh xe-drmfd.sh usr/bin/env DISPLAY=:0 GALLIUM_DRIVER=zink \
    MESA_LOADER_DRIVER_OVERRIDE=zink VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json \
    /tmp/gl-probe window
stage 6: context OK
stage 7: makeCurrent OK
renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
stage 8: first draw OK
stage 9: first swap OK
stage 10: 60 swaps OK
readback px=0,255,0,255 err=0x3000
RESULT PASS
```

桌面会话（`husk-x11-gpu4.sh` = gpu3 换成新 loader）另测：`startup-complete`、
`gl-probe root` 抓到 `2400x1080`、非黑像素 100%（见 evidence 165 §4 的方法）。

## 5 · 未验证（UNKNOWN）

- 这两条对**真机上的浏览器/Electron**是否有可观测差别：本机 rootfs 里没有浏览器包，未测。
- Adreno 路径同样吃这两个修复（dup 记账与家族无关），但没有高通设备，**未验证**。
