# 215 · `scandirat` 自己实现：导符号拦不住的那一层

日期：2026-11-25 22:00–22:20（CST）。设备 RMX3357，`:x11` 未起，纯 guest 命令。

## 1 · 问题（FACT，来自 214 §4.1）

`scandirat(AT_FDCWD, "/root", …)` 在 guest 里返回 `n=-1, errno=2`。原因不是我们没导符号，
而是 glibc 的 `scandirat` 内部走 `__openat64_nocancel` 的**直接调用**（隐藏别名，不经 GOT），
所以：

- 导出 `__openat64_nocancel` 拦不到（0.0.11 试过，见 214 §2 的改后列）；
- 唯一能拦的是 body patch，而 body patch 在这台 4.19 上 `sig=11`（214 §4.1，用改动前的
  95 条清单一样崩，不是我们加的四个名字造成的）。

## 2 · 改法（FACT）

`scandirat` 的参数本身就是"路径 + dirfd"，所以不需要任何内部钩子，自己实现即可：

```
openat(dirfd, path, O_RDONLY|O_DIRECTORY|O_CLOEXEC)   ← 我们的 openat：翻译 + 只读检查
  → fdopendir(fd)
    → scandir_common(dir, namelist, select, compar)    ← 与 scandir 共用的扫描体
```

`runtime/crates/path-shim/src/lib.rs`：把 `scandir` 的扫描体抽成 `scandir_common`，
新增 `scandirat` 走上面这条链；failing 路径都把 `errno` 显式写好（`openat` 失败不留旧 errno）。

## 3 · 实测（FACT）

`sh tools/probe/run-coverage-probe.sh UKHMMB8PYTGECMNV`

| 用例 | 0.0.11 | 0.0.12 |
|---|---|---|
| `scandirat(AT_FDCWD, "/root", …)` | `n=-1 errno=2` | **`n=36 errno=0`** |
| 其余 9 条 | 同 | 同（无回归） |

```
--- coverage-probe: PASS
```

## 4 · 探针并进门禁（FACT）

`make gate-a` 现在是两段：

```
PASS=36 FAIL=0
--- coverage-probe: PASS
```

为什么单列一步：36 项验收的判据是"命令返回 0 / 内容对"，探针的判据是"这个文件到底落在
guest 里还是宿主里"。前者全绿的时候后者仍可能红（0.0.5 时代 `inotify_add_watch` 就是这样
静默失效的），所以两个都要跑。

## 5 · 还没做

- `coverage-probe` 目前是 **10 条点测**，不是普查。要变成普查（把 nautilus / apt / python /
  浏览器实际调到的路径全捞出来、断言没有未翻译的绝对路径），需要 strace 或一层 seccomp
  记账——rootfs 里现在没有 strace，`apt-get install strace` 还没验（本机没网时这一步会静默失败）。
  这一条进 0.0.13 全链路审计。
- body patch 仍然坏（`sig=11`），`__xstat` 家族、`execveat`、`umount` 仍未实测。
