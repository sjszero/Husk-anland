# 126 · 64 KiB 内核全绿；`ls -l` 多一行 stderr 的真因：xattr/readlink 返回了裸 -errno

日期：2026-11-23　宿主：macOS x86_64 + QEMU 11.0.3（TCG）
被测物：`tools/loader/ld-embedded.so`（`ssize_or_errno` 修复后，673280 B）

## 1. 64 KiB 内核是怎么拿到的（可复现，别自编内核）

Ubuntu 给 arm64 发布 64K 页变体，直接下二进制即可（不需要在 macOS 上编内核）：

```sh
curl -s -o k64.deb http://ports.ubuntu.com/ubuntu-ports/pool/main/l/linux-signed/\
linux-image-6.8.0-31-generic-64k_6.8.0-31.31_arm64.deb
mkdir k64x && cd k64x && ar x ../k64.deb && tar xf data.tar.zst
gzip -dc boot/vmlinuz-6.8.0-31-generic-64k > Image.ubuntu64k.raw
```

启动（`/tmp/k16` 里的 initramfs 不变，只换 kernel）：

```sh
qemu-system-aarch64 -M virt -cpu max -smp 4 -m 3072 -nographic \
  -kernel Image.ubuntu64k.raw -initrd initramfs16.cpio.gz \
  -append "console=ttyAMA0 rdinit=/init loglevel=4 panic=1" -no-reboot
```

## 2. 64K 内核实测（FACT，全通过）

| 检查 | 结果 |
|---|---|
| `uname -r` | `6.8.0-31-generic-64k` |
| guest `getconf PAGESIZE` | **65536** |
| `cat /etc/os-release` | `Ubuntu 24.04.4 LTS` |
| `ls /usr/bin \| wc -l` | `296` |
| 嵌套 exec（bash → sh → cat） | `localhost.localdomain`，inner=0 |
| `/tmp` 写读 + `ls -l` | `hi64` / `-rw-r--r--` |
| `check-elf . --page 4096/16384/65536` | 727 个 ELF，三档 all congruent |

⇒ 4K、16K、64K 三种内核页大小上，loader + shim 的功能一致。
**16K/64K 真机仍是 UNKNOWN**（本机只有 4 KiB）；这里证明的是"内核页大小无关"，不是"某台真机可用"。

## 3. `ls -l` 多一行 stderr：真因与修复

### 现象

只有 `ls -l`（长格式），每个文件一行，且没有原因文本：

```
$ ./libld.so --library-path $LP usr/bin/ls -l /tmp
usr/bin/ls: /tmp          ← stderr，原厂 loader 下没有
total 0
```

### 定位路径（FACT）

1. 关掉变量：`ls /tmp`（无 `-l`）无此行；`stat`/`du` 无此行；`ls -l /tmp/nope` 只有正常的
   `cannot access ...`。⇒ 是长格式里**逐文件**的动作。
2. 读 coreutils 9.4 `src/ls.c`：长格式路径上有
   `if (err == 0 && format == long_format) { n = file_has_acl_cache(...); err = (n < 0); }`
   以及 SELinux 的 `getfilecon()`；失败时执行 `error (0, errno, "%s", quotef (full_name))`
   —— 这条只在 `err` 为真时打印，且**如果 errno 是 0 就只剩 `<progname>: <path>`**，与现象完全一致。
3. 根因：本 shim 的 `getxattr/lgetxattr/listxattr/llistxattr`（以及 `readlink/readlinkat`）
   **把内核的裸 `-errno` 直接当返回值交了出去**（`-61` = ENODATA），没有做
   `-1 + errno=…` 的翻译。libselinux 看到 `getxattr() < 0` 但 `errno` 仍是 0，
   coreutils 于是打出 `<argv0>: <path>` 这一行没有原因的告警。

### 修复

新增 `ssize_or_errno(r: Ssize) -> Ssize`（与 `recvmsg` 里早已写下的规则一致：
"glibc 的封装只给调用方 -1 和 errno，从不给负的原始值"），在
`readlinkat`（三处返回点）、`lgetxattr`、`getxattr`、`listxattr`、`llistxattr` 上统一套用。

### 验证（FACT）

| 环境 | 检查 | 结果 |
|---|---|---|
| 16K QEMU | `ls -l /` 与 `ls -l /tmp` 的 stderr | **0 字节** |
| 64K QEMU | 同上 | **0 字节** |
| 64K QEMU | `readlink -v /etc/nope` | `/rootfs/usr/bin/readlink: /etc/nope: No such file or directory`（errno 文本正确，不是 "Success"） |
| 4K 真机 RMX3357 | `ls -l /` 的 stderr | **0 字节**；`readlink -v` 同样正确 |
| 4K 真机 | `04-path-reliability.sh` | `#checks=31 fail=0` + `#checks=15 fail=0` |
| 16K / 64K / 4K | 功能矩阵 | `cat /etc/os-release`、`/usr/bin` 计数（4K=877 / 16K·64K=296）、嵌套 exec、`/tmp` 写读、`check-elf` 全绿 |

注：这不只是"把告警藏掉"。修复前任何"检查 errno 再决定行为"的调用方在我们的
`getxattr`/`readlink` 上都会拿到 `-errno` 与过期 errno，属于正确性缺陷；`readlink -v`
在修复前后的差异（`Success` vs `No such file or directory`）就是它的直接证据。
