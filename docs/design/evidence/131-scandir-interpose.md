# 131 · `dpkg-deb` 的 `scandir` 路径漏接（已修复）

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），
包：`com.linuxhub.myth.husk`，loader：`tools/loader/ld-embedded.so`。

## 0 · 结论（FACT）

`scandir(3)` 不经过 `opendir(3)`，所以旧的 path-shim 只接管 `opendir/readdir`
时仍会在宿主 `/tmp` 扫描目录。`dpkg-deb` 正是用 `scandir` 枚举自己刚创建的
临时控制目录，导致：

```text
dpkg-deb: error: cannot scan directory '/tmp/dpkg-deb.IWVIAC': No such file or directory
```

现在 path-shim 导出并 interpose `scandir`，同一命令返回 0，随后 `dpkg -i`
也能完成 `acl` 的安装。

## 1 · 复现（修复前）

在 guest rootfs 内把包复制到 `/tmp/acl.deb`，用 loader 启动 `dpkg-deb -I`：

```sh
R=$PWD/files/linux/distributions/ubuntu-24.04/rootfs
L=$PWD/files/linux/husk/libld.so
cp "$R/var/cache/doro-offline/packages/acl_2.3.2-1build1.1_arm64.deb" "$R/tmp/acl.deb"
cd "$R"
env NR_ROOTFS="$R" NR_LD="$L" NR_SIGSYS=enosys \
  NR_SIGSYS_LOG="$R/tmp/nr-sigsys.log" \
  GLIBC_TUNABLES=glibc.pthread.rseq=0 \
  PATH="$R/usr/local/sbin:$R/usr/local/bin:$R/usr/sbin:$R/usr/bin:$R/sbin:$R/bin" \
  "$L" --library-path "$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu" \
  usr/bin/dpkg-deb -I /tmp/acl.deb
```

修复前：

```text
 new Debian package, version 2.0.
 size 39014 bytes: control archive=909 bytes.
dpkg-deb: error: cannot scan directory '/tmp/dpkg-deb.IWVIAC': No such file or directory
```

直接给 `dpkg-deb` 一个非临时路径时能过扫描，说明包和 `dpkg-deb` 本身没有问题；
差异只在临时目录的 `scandir` 路径翻译。`nm -D` 对该二进制明确显示：

```text
UND scandir@GLIBC_2.17
UND opendir@GLIBC_2.17
```

`opendir` 已由 shim 实现，`scandir` 没有，这就是根因。

## 2 · 实现

文件：`runtime/crates/path-shim/src/lib.rs`，在 `opendir` 后用同一个 guest
路径解析器实现 `scandir`：

- 用 shim 的 `opendir` 打开 guest 目录；
- 用 libc `readdir` 读取，按 aarch64 glibc `dirent` 布局复制记录
  （`d_reclen` 在 +16，`d_name` 在 +19）；
- 支持 nullable `select` 和 `compar` 回调，排序由自身完成，不额外引入 `qsort`；
- 数组用 libc `malloc/realloc/free`，调用方仍可用普通 `free()` 释放；
- 读目录、分配或格式异常时清理已分配项并保持 `-1 + errno` 语义；
- `scandir` 是导出函数，`nr-tools embed` 已将它加入 interpose 表。

构建验证（release、64 KiB over-align）：

```sh
NDK=$ANDROID_HOME/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin \
  sh tools/loader/build-embedded-loader.sh
```

结果：`interposed=166`（含新增 `scandir`），`externs=49`，loader 输出大小
673280 bytes，构建脚本的 glibc 版本和 syscall-site 断言全部通过。

## 3 · 修复后实测（FACT）

同一 `dpkg-deb -I /tmp/acl.deb` 命令：

```text
 new Debian package, version 2.0.
 size 39014 bytes: control archive=909 bytes.
     566 bytes,    15 lines      control
     468 bytes,     8 lines      md5sums
 Package: acl
 Version: 2.3.2-1build1.1
 Architecture: arm64
...
```

退出码：`0`。

继续跑真实安装：

```sh
usr/bin/dpkg --force-confdef --force-confold -i /tmp/acl.deb
```

输出：

```text
Unpacking acl (2.3.2-1build1.1) ...
Setting up acl (2.3.2-1build1.1) ...
RC=0
```

因此 `dpkg-deb` 的 `scandir` 漏接已收口；它不再阻塞 2.0 的第一批桌面包安装。
后续初始化若仍失败，需要按新的最小复现定位，不把 `dpkg-split` 的次生错误
当成独立根因。
