# 125 · 16 KiB 真内核端到端全绿；"exec 之后 shim 失效"其实是测试脚本没设 NR_LD

日期：2026-11-23　宿主：macOS x86_64 + QEMU 11.0.3（TCG）
被测物：`tools/loader/ld-embedded.so`（sha256 `be32d7f1…`, 673280 B，64 KiB over-align）
环境：RPi5 `kernel_2712.img`（`6.18.64-v8-16k+`，`AT_PAGESZ=16384`），initramfs = Ubuntu 24.04 base rootfs

## 1. 结论

上一份 evidence（124）留下的"exec 之后 shim 不再改路径"**不是 16K 的 Bug，也不是运行时 Bug**。
真因是测试脚本少设了环境变量，导致 exec 出来的子进程换了 loader：

| 缺失的变量 | 后果 |
|---|---|
| `NR_LD` | `execve()` 回落到 `<root>/usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1`（rootfs 里原厂 glibc），**我们的 shim 不在里面**，子进程自然不被 interpose |
| `NR_ROOTFS` | `guest_root()` 为空 → 连父进程都不改写（本次排障中先踩到这个，症状是 `command not found` 而不是 `No such file`）|

两者都设上之后，16K 内核上**没有任何遗留缺陷**。

判据：`NR_LD` 与 `NR_ROOTFS` 是启动契约的一部分，正式启动器
（APK 侧 `gexec.sh` / native lib dir）一直都会设 `NR_LD`；只是 QEMU 里手搓的验证脚本漏了。
→ **运行时不需要改代码修这个**；要改的是验证脚本，以及把这两个变量的契约写进文档。

## 2. 16K 上的实测矩阵（FACT，全部通过）

命令（initramfs 内，`R=/rootfs`）：

```sh
export NR_ROOTFS=$R NR_LD=$R/libld.so
LP=$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu
./libld.so --library-path $LP usr/bin/bash -c '...'
```

| 检查 | 结果 |
|---|---|
| `cat /etc/os-release` | `PRETTY_NAME="Ubuntu 24.04.4 LTS"`，rc=0 |
| `ls /usr/bin \| wc -l` | `296`；`command -v ls cat bash` → `/bin/ls` `/bin/cat` `/bin/bash` |
| 嵌套 exec（bash → sh → cat） | `cat /etc/hostname` → `localhost.localdomain`，inner=0 outer=0 |
| 写读 `/tmp` | `echo hi16 > /tmp/nt16.txt` → `cat` 回 `hi16`；`ls -l` 显示 `-rw-r--r-- 1 root root 5` |
| `cd /usr/bin && pwd` | `/usr/bin`（相对路径与前缀匹配都对） |
| `nr-tools check-elf . --page 4096/16384/65536` | `checked 727 ELF file(s)`，三档 `all congruent` |

配合 124 已记录的：`uname -r` = `6.18.64-v8-16k+`、`AT_PAGESZ`/`getconf PAGESIZE` = 16384、
以及 78a04f8 修的两个真 Bug（shim 段与 loader 末段共用 16K 页、hook 的 mprotect 按 4K 对齐）。

⇒ **loader + path-shim 在真 16K 内核上功能等价于 4K。** 16K **真机**仍是 UNKNOWN（本机只有 4K）。

## 3. 顺带发现：`ls -l` 在我们的 loader 下多打一行 stderr（P2，未定性）

```
# 原厂 loader，无 shim
./usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1 --library-path $LP usr/bin/ls -l /tmp
total 0                                    ← 只有这一行

# 我们的 loader（shim 生效）
./libld.so --library-path $LP usr/bin/ls -l /tmp
usr/bin/ls: /tmp                           ← 多出来的一行（stderr，无原因文本）
total 0
```

特征（FACT）：

- 只在 **stat 成功**时出现（`ls -l /tmp/nope123` 只有标准的 `cannot access ...` 一行）。
- 文本形状是 `error(0, 0, "%s", name)` 的样子：`<argv0>: <path>`。
- `cat` / `stat` 之外的常规读操作没有这个现象。
- 不影响功能：ls 的正常输出与退出码都正确（rc=0）。

推断（INFERENCE，未验证）：shim 在某个 stat/readdir 前置调用上让 ls 拿到一个它不预期的
errno（被 ls 用 `error(0,0,...)` 记了下来），然后 ls 重试成功。下一步用
`strace -f -e trace=%file` 对比两条路径的 syscall 序列即可定性。

## 4. 复现命令（不要重建环境）

```sh
cd /tmp/k16                     # Image.rpi5 + root/(initramfs 源)
# 改完 root/ 里的东西后重打 initramfs：
(cd root && find . | cpio -o -H newc | gzip -1 > ../initramfs16.cpio.gz)
qemu-system-aarch64 -M virt -cpu max -smp 4 -m 3072 -nographic \
  -kernel Image.rpi5 -initrd initramfs16.cpio.gz \
  -append "console=ttyAMA0 rdinit=/init loglevel=4 panic=1" -no-reboot
```

一次启动约 8–15 秒（TCG，`/init` 跑完 autorun 就 `poweroff -f`）。
`NR_DEBUG=1 NR_DEBUG2=1` 可以打开 shim 自己的 verbose（输出去 stderr 与 `<root>/tmp/nr-exec.log`）。
