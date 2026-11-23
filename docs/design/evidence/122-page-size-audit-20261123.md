# 122 · 页大小体检：2987 个 ELF，4K/16K/64K 全绿（外加一个我们自己造的 16K 杀手）

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / **4 KiB**（本机没有 16K/64K 内核）
全部命令在 guest（app uid 10302）里跑，无 root、无 chroot、无 ptrace。

## 1. 判据（两条，缺一不可）

加载一个 ELF 有两道各自独立的检查，之前文档只写了第一条：

| # | 规则 | 谁检查 | 违反的后果 |
|---|---|---|---|
| 1 | 每个 `PT_LOAD` 满足 `p_offset ≡ p_vaddr (mod PAGE_SIZE)` | 内核 `execve`/`mmap` | `mmap` `EINVAL`，整个加载失败 |
| 2 | 每个 `PT_LOAD` 的 `p_align` 必须是页大小的整数倍 | **glibc `dl-load`**：`(ph->p_align & (dl_pagesize-1)) != 0` → `ELF load command alignment not page-aligned` | 对象直接拒绝加载（不是"大概率"，是立即失败） |

第 2 条是本轮实测才补上的：它在 4 KiB 内核上永远看不出问题（`p_align % 4096 == 0` 对
`0x1000`/`0x4000`/`0x10000` 都成立），只有把内核页大小换成 16K/64K 才会暴露。
`p_align` 不是"建议值"，是 glibc 的硬校验。

## 2. 我们自己造的失败项（FACT）

`libc.so.6` 被 `nr-tools libc-interpose` 打过槽位补丁后，PT_LOAD 变成三段：

```
LOAD off=0x0      vaddr=0x0      filesz=0x19b000  align=0x10000
LOAD off=0x19b000 vaddr=0x19b000 filesz=0x2000    align=0x1000   <-- 我们加的槽位段
LOAD off=0x19d100 vaddr=0x1ad100 filesz=0x4528    align=0x10000
```

同余（规则 1）是满足的（off == vaddr），但 `0x1000 % 0x4000 = 0x1000 ≠ 0`，
**16K 内核上 glibc 会拒绝加载 libc.so.6**，guest 里任何动态程序都起不来；
64K 内核同理。这是本项目自己造出来的兼容性缺陷，不是 Ubuntu 的问题
（Ubuntu noble arm64 侧的 4391 个 ELF 本来就是 64K 对齐）。

## 3. 修法（`runtime/crates/nr-tools/src/libc_slots.rs`）

槽位页从"塞进两个 LOAD 之间的空洞"改成"追加在文件末尾"：

* 空洞只有 `0x19d100 - 0x199ec1 = 0x323f` 字节，放得下 4K 对不齐的两个页，但永远放不下 64K 对齐的段；
* 放到文件末尾后，`p_offset` 和 `p_vaddr` 都能取 `SEG_ALIGN = 0x10000` 的整数倍：
  `slot_off = align(file_len, 0x10000) = 0x1b0000`，
  `slot_vaddr = align(max(vaddr+memsz), 0x10000) = 0x1c0000`，两者同余（都是 0）；
* 存根页仍留在空洞里（入口搬到存根页后，`b.cond`/`cbz`/`tbz` 的射程要求它必须靠近 `.text`），
  它被并进可执行 LOAD，不新增段落；
* 入口用 `adrp`（±4 GiB）取槽位页地址，所以槽位页离 `.text` 多远都无所谓；
* 新增两条断言（不满足就拒绝写文件）：每个 LOAD 的 `p_align` 必须是 2 的幂且 `p_align % 0x4000 == 0`；
  `(p_vaddr - p_offset) & (p_align-1) == 0`；LOAD 按 `p_vaddr` 保持升序。
* 槽位表里 `stub_delta` 现在是负值（存根在槽位页下方），运行期 `add x17, x16, x17` 按 u64 环绕，实测正常。

文件只长大 `0x1b2000 - 0x1a4a28 = 0x7bd8`（+31 KB），没有额外页。

## 4. 复测（FACT，同一台机）

```
guest$ nr-tools check-elf usr/lib lib usr/bin bin usr/sbin sbin usr/libexec \
            --page 4096 --page 16384 --page 65536
checked 2987 ELF file(s)
    4096: all congruent
   16384: all congruent
   65536: all congruent
```

`nr-tools check-elf` 这轮补了两件事：把规则 2 纳入判定；扫描目录时跳过
`.orig` / `.pre-*` / `.bak*` 备份（`libc.so.6.orig` 是重打补丁用的原料，不该参与结论）。

新 libc 上机后功能没有退化（同一台机、同一批脚本）：

| 检查 | 结果 |
|---|---|
| `getent hosts baidu.com` | 真实 IP，rc=0 |
| `TZ=Asia/Shanghai date` | `CST` |
| `locale -a` | `C C.utf8 POSIX` |
| `dpkg -l \| grep -c ^ii` / `apt-get -s install sl` | 1194 / 正常 |
| 05-mount-audit / 09 / 10 / 11 / 12 / 13 | 18 / 11 / 9 / 8 / 8 / 6，全 PASS |
| 06 / 07 / 08（POSIX mqueue、SysV shm、sem+msg） | 3 × PASS |

## 5. loader 自己也是 16K 不合格（规则 2）

`libld39.so`（glibc 的 `ld.so` + 我们嵌进去的 shim）三个 LOAD：

```
LOAD off=0x0     vaddr=0x0     align=0x10000
LOAD off=0x2e988 vaddr=0x3e988 align=0x10000
LOAD off=0x32000 vaddr=0x42000 align=0x1000    <-- 嵌入的 shim 段
```

同余（规则 1）三段都满足，`0x10000` 与 `0x20000` 的差也是 4K/16K/64K 的倍数；
只有规则 2 不满足（`0x1000 % 0x4000 ≠ 0`）。这个段只由内核 `execve` 映射，
不会被 glibc `dlopen`，所以**今天在 4K 机器上没有任何症状**；但静态体检会判 16K/64K FAIL，
而且下次谁把它 `dlopen` 进用户态就会当场失败。`nr-tools embed` 现在把该段的 `p_align`
写成 `SEG_ALIGN = 0x10000`（`V_BASE - F_BASE` 恰好等于它，所以仍和任意页大小同余），
重编后的 loader 三段全是 `0x10000`：

```
checked 1 ELF file(s)
    4096: all congruent
   16384: all congruent
   65536: all congruent
```

## 6. 我们自己的 6 个 guest 二进制

体检结果（修之前）：

| 产物 | 修前 | 修后 |
|---|---|---|
| `husk-init`（freestanding static） | `0x4000` → 64K ✗ | `0x10000` ✓ |
| `husk-logd`（no_std static） | `0x4000` → 64K ✗ | `0x10000` ✓ |
| `husk-usermgr` / `husk-ldconfig` / `husk-rd`（动态） | `0x4000` → 64K ✗ | `0x10000` ✓ |

构建脚本：`tools/guest-config/build-init.sh`、`build-logd.sh`、`build-guest-tools.sh`（新增）。
三个动态工具必须用 **NDK 的 clang 当链接驱动**：

```
-C linker=$NDK/bin/clang  -C link-arg=--target=aarch64-linux-gnu
-C link-arg=--sysroot=$SYSROOT  -C link-arg=-fuse-ld=$NDK/bin/ld.lld
-C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1
-C link-arg=-Wl,-z,max-page-size=65536
```

只用 `rust-lld` 直接链接（本仓库 `.cargo/config.toml` 的默认）会**静默产出 `e_entry = 0`**：
rustc 不传 `crt1.o/crti.o/crtn.o`，rust-lld 找不到 `_start` 就写 0。
实测症状是"程序在任何 loader 下（包括 Ubuntu 原版 `ld.so`）都在 `_dl_init` 之后、`main` 之前 SIGSEGV"，
`LD_DEBUG=init` 能看到最后一行是 `calling init: .../libgcc_s.so.1`。
所以这三个 crate 的构建必须走 C 驱动，不是偏好问题。

## 7. 仍未验证（UNKNOWN）

* 16K / 64K 内核真机：本机只有 4 KiB，以上全部是**静态判定 + 4K 真机运行**；
  16K/64K 上"能跑"仍是推断，不是实测。
* `usr/sbin/ldconfig.real`（就是我们的 `husk-ldconfig`）与 `husk-usermgr` 之前部署的是旧 16K 版本，
  本轮已替换；`/usr/sbin/useradd` 等 51 字节包装脚本 `exec /usr/sbin/husk-usermgr`，路径未变。
* Ubuntu 自带的 `usr/sbin/ldconfig`（387 字节 shell 包装）+ `ldconfig.real` 的替换关系保持原样：
  包装脚本调用 `ldconfig.real`，后者是我们的实现。

## 8. 复现

```sh
# 1. 重新打 libc 槽位补丁（空洞里只放存根页，槽位页追加到文件末尾）
runtime/target/release/nr-tools libc-interpose \
    .../libc.so.6.orig /tmp/libc.slots.so.6 tools/loader/libc-interpose.txt

# 2. 体检（主机侧，任意目录树）
runtime/target/release/nr-tools check-elf <dir>... --page 4096 --page 16384 --page 65536

# 3. guest 侧同一份工具（交叉编译后推进 rootfs）
sh tools/guest-config/build-guest-tools.sh          # 三个动态工具，64K 对齐
sh tools/guest-config/build-init.sh                 # husk-init
sh tools/guest-config/build-logd.sh                 # husk-logd
adb -s <serial> shell "run-as com.linuxhub.husk sh files/gexec.sh rootfs-full \
    usr/bin/nr-tools check-elf usr/lib lib usr/bin bin usr/sbin sbin usr/libexec \
    --page 4096 --page 16384 --page 65536"
```
