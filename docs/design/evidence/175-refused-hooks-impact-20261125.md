# 175 · 那 5 个「打不上」的 hook，到底漏没漏东西

日期：2026-11-25（CST）。对象是 `libc-interpose.expected` 里被钉住的 5 个拒绝项：
`__readlink_chk`、`__readlinkat_chk`、`__realpath_chk`、`__mq_open_2`、`__openat64_nocancel`。
之前只在 `nr-tools libc-patch` 的输出里写「something branches into the window」，没人验证过
这算不算真的漏。

## 做法

1. 取 CI 用的同一份 stock libc（`libc6 2.39-0ubuntu8.12` arm64，sha256
   `2578bc27…`），解包出 `usr/lib/aarch64-linux-gnu/libc.so.6`；
2. `llvm-readelf --dyn-syms` 取符号地址，`llvm-objdump -d` 反汇编整棵 `.text`（294 873 行）；
3. 对每个被拒函数，列出**落在 24 字节补丁窗口内**（entry+4 … entry+20）的所有分支目标；
4. 把这些函数的函数体读一遍，看它们真正的工作路径走到哪。

## 结果

| 符号 | 地址 | 窗口内的分支 | 函数体真正做的事 |
|---|---|---|---|
| `__readlink_chk` | `0xf8dc0` | `b.hi 0xf8dcc`（自己跳自己 +0xc） | `cmp`；超长 → `__chk_fail`（abort）；否则 `b readlink` |
| `__readlinkat_chk` | `0xf8de0` | `b.hi 0xf8dec` | 同上，尾部 `b readlinkat` |
| `__realpath_chk` | `0xf9040` | `b.ls 0xf904c` | `cmp x2,#0xfff`；超长 → `__chk_fail`；否则 `b realpath` |
| `__mq_open_2` | `0x91530` | `tbnz w1,#6 → 0x91538` | 带 `O_CREAT` 无 `mode` → `__fortify_fail`；否则 `b mq_open` |
| `__openat64_nocancel` | — | — | 这个构建**没有导出**（`.dynsym` 里没有），无法定位 |

关键点：前四个所谓「有分支跳进窗口」，跳的都是**它自己**（`0xc` 偏移处），没有任何外部函数
跳进来。而它们唯一会碰文件系统的路径，是尾调用到 `readlink` / `readlinkat` / `realpath` /
`mq_open`——这四个名字**都在 `libc-interpose.txt` 表里**，也就是已经被 slot 页接管的入口。

## 结论（FACT，前四条）

- 这四个拒绝项不构成路径逃逸：包装函数本身只做长度检查，真正干活的那一步走的是**已打补丁的
  入口**；补它们等于给一个只做 `cmp` + `b` 的 24 字节壳子再套一层，收益为零。
- 因此 `.expected` 里保留它们的意义变成「记录 glibc 这版长这样」，而不是「覆盖缺口」。

## 剩下一条（UNKNOWN）

`__openat64_nocancel` 在这个构建里没有导出符号，slot 页无从定位（工具报的是
「not exported by this libc (stripped .symtab)」）。它属于 glibc 内部「不可取消」的打开路径，
调用者只会是 libc 自己的函数——而那些外层入口（`open` / `open64` / `openat` / `openat64`）
都在表里、整条函数体被接管，内部路径根本不会被执行到。所以**推断**是同样不漏，但这一条没有
像前四条那样被反汇编证死，标 UNKNOWN。

## 影响

`docs/LIMITATIONS.md` 里原先没写这 5 个拒绝项；现在写成「四条已证不影响、一条未证」，
不再让读者以为有 5 个已知的路径逃逸。
