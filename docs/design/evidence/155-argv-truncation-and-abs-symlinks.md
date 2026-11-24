# 155 · argv 超过 63 个被静默截断、552 个绝对符号链接逃出 rootfs（FACT）

日期 2026-11-24 15:30–16:05 · RMX3357（MT6893 / Android 13 / kernel 4.19.191 / 4 KiB / arm64）
包 `com.linuxhub.myth.husk`（debug），`untrusted_app`，loader rev 22 → **rev 23**，
rootfs 882 包（Ubuntu 24.04 + build-essential/git/vim/nodejs/python3-numpy）

这两个 bug 都是「基准跑不动」时才露出来的，而且报错信息完全不指向真正原因。基准本来要测
`make -j4`（八维度里的构建吞吐），结果 `make_j4` 一律 `NA` —— 追下去是本文的两条。

## 0 · 结论

| # | 现象 | 根因 | 修在哪 | 修完实测 |
|---|---|---|---|---|
| 1 | `make -j4` 的 200 目标链接失败，`ld` 报 `crtstuff.c:(.text+0x38): undefined reference to '__TMC_END__'` / `hidden symbol '__TMC_END__' isn't defined` / `final link failed: bad value` | shim 的 `execve` 用栈上 `[*const CChar; 64]` 拼 argv，**第 63 个参数之后被静默丢弃**。collect2 给 `ld` 的参数超过 200 个，`-lc`、`crtend.o` 掉在数组外 | `path-shim` 改成 `argv_len()` 量长度 + `argv_alloc()` 堆分配（失败才 `free`）；两条 exec 路径都改 | `make -j4`（200 个 `.o`）`MAKE_RC=0`，3 509 ms，产出 `app` 91 576 字节 |
| 2 | `ctypes.CDLL('libblas.so.3')` → `ENOENT`，同一个文件用绝对路径能打开；`numpy` 装得上但 import 失败 | guest rootfs 里有 **552 个绝对符号链接**（如 `/usr/lib/aarch64-linux-gnu/libblas.so.3 → /etc/alternatives/...`）。宿主机加载器解析 `<hostdir>/soname` 时，内核按 **host 根** 解析链接目标 → 逃出 rootfs | `tools/guest-config/relative-symlinks.py --apply` 原地改写成相对路径 | 552 → 改写 551（1 条悬空保留），`numpy 1.26.4` 可用：`arange(200000).sum() = 499500`，`libblas.so.3` / `liblapack.so.3` 按 soname 加载 OK |

另外两个本轮确认、但**不是** Husk 的问题（写进 `docs/LIMITATIONS.md` 的取样条件）：
基准夹具 `prep.sh` 生成的 200 文件工程没有 `main()`，三个运行器都会链接失败；
`adb shell` 的 guest stdout 是 socket，`make` 往它写之后同一会话的写入全部 `EACCES`
（proot 完全一样，不是 shim 的行为，因此基准一律重定向到 `/dev/null`）。

## 1 · bug 1：argv 第 64 个以后被丢掉（FACT）

复现（guest 内，`make` 的链接步骤，200 个目标文件）：

```
$ cd /root/bench-make && make -j4
/usr/bin/ld: crtstuff.c:(.text+0x38): undefined reference to `__TMC_END__'
/usr/bin/ld: app: hidden symbol `__TMC_END__' isn't defined
/usr/bin/ld: final link failed: bad value
collect2: error: ld returned 1 exit status
```

`__TMC_END__` 是 `crtend.o` 里的符号，`crtend.o` 由 `gcc` 在参数末尾自己追加。也就是说
collect2 收到的参数尾巴整段不见了。缩小范围：

| 用例 | 参数个数 | 结果 |
|---|---|---|
| `gcc -o h h.c` | 4 | OK |
| `gcc -o app f0.o f1.o` | 5 | OK |
| `make`（3 文件工程） | 5 | OK |
| `make -j4`（200 文件工程） | ≈ 205 | 失败，尾巴丢失 |

单文件、两目标、3 文件 `make` 全过，只有 200 参数失败 —— 与「64 个槽位的栈数组」吻合。

修法（`runtime/crates/path-shim/src/lib.rs`）：

```rust
unsafe fn argv_len(argv: *const *const CChar) -> usize { /* 数到 NULL */ }
unsafe fn argv_alloc(n: usize) -> *mut *const CChar { /* malloc((n+1)*8)，清零，ENOMEM 返回 null */ }
```

`execve` 的两个分支（静态/shebang 直执、动态经 loader 重入）都改成堆数组，写入前预留
`+4` / `+8` 个空位给 `--library-path`、`--argv0`、shebang 参数；只有 `execve` 失败才 `free`。

修完实测（rev 23、882 包 rootfs）：

```
$ cd /root/bench-make && make -s clean && time make -s -j4
MAKE_RC=0 MS=3509
-rwxrwxrwx. 1 root root 91576 Nov 24 16:02 app
```

中间态也验证过一次：只修 argv、不修夹具时，报错从
`hidden symbol '__TMC_END__' isn't defined` 变成 `(.text+0x20): undefined reference to 'main'`
—— 说明 `crt1.o` / `-lc` 已经完整到达 `ld`，截断确实消失了。

## 2 · bug 2：绝对符号链接按 host 根解析（FACT）

现象（guest 内 python）：

```
$ python3 -c "import ctypes; ctypes.CDLL('libblas.so.3')"
OSError: libblas.so.3: cannot open shared object file
$ python3 -c "import ctypes; ctypes.CDLL('/usr/lib/aarch64-linux-gnu/libblas.so.3')"
<CDLL ... ok>          # 同一个文件，绝对路径能开
```

`/usr/lib/aarch64-linux-gnu/libblas.so.3` 是指向 `/etc/alternatives/libblas.so.3-aarch64-linux-gnu`
的**绝对**符号链接，后者再绝对链接到 `blas/libblas.so.3`。加载器把
`<hostroot>/usr/lib/aarch64-linux-gnu/libblas.so.3` 交给内核打开，内核沿链接读到 `/etc/...`
—— 那是 **Android 的** `/etc`，不是 guest 的。

统计与修法：

```
$ python3 tools/guest-config/relative-symlinks.py /path/to/rootfs
552 absolute symlinks
$ python3 tools/guest-config/relative-symlinks.py --apply /path/to/rootfs
rewritten 551, dangling 1 (/etc/pulse/client.conf.d/01-enable-autospawn.conf -> /run/pulseaudio-enable-autospawn)
```

脚本只改「目标在 rootfs 内且以 `/` 开头」的链接，`/proc /sys /dev /run /tmp` 跳过，
悬空链接列出来不动。修完：

```
$ python3 -c "import numpy; print(numpy.__version__, int(numpy.arange(200000).sum()))"
1.26.4 499500
```

rootfs 里只要还有绝对符号链接，任何按 soname 的 `dlopen` 都可能踩到，因此这一步写进
`docs/RUNNING.md` 的 rootfs 准备步骤，不是一次性手工修补。

## 3 · 两个不是 Husk 的问题（同样影响基准取样）

**夹具缺 `main()`**：`tools/bench/guest/prep.sh` 原来生成的 200 个 `.c` 只有
`int fN(void)`，没有 `main`，三个运行器都会得到 `undefined reference to 'main'`，
测到的是坏 Makefile 而不是构建吞吐。已改为 `f0.c` 带 `main()`，夹具重生成后
Husk 侧 `MAKE_RC=0`。

**adb shell 的 stdout 是 socket**：`ls -l /proc/self/fd/1` 在 guest 里是
`socket:[...]`，`make` 往它写完（哪怕只 309 字节）之后，同一会话后续写入返回
`Permission denied`：

```
$ sh -x -c "make -v" 2>/root/x.log; echo F_RC=$?
bin/sh: 1: echo: echo: I/O error
```

同一命令在 **proot** 下行为完全一致（同样只剩空输出），所以这不是 shim 的 bug。
把 make 的 stdout 换成本进程自己 `pty.openpty()` 出来的终端就正常
（`GNU Make 4.3` 309 字节，`rc=0`）。基准因此统一
`make -s -j4 >/dev/null 2>&1`，只测造物时间，不测 adb 回传。
