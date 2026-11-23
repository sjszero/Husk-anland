# evidence 100：dlopen 进来的库也走 guest 路径

日期：2026-11-23
设备：RMX3357（MT6893 / Mali-G77，Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64）
域：`untrusted_app`（uid u0_a302）  rootfs：`files/rootfs-full`（Ubuntu 24.04.4）
测试程序：NDK clang 交叉编译（`--target=aarch64-linux-gnu --sysroot=/tmp/gccroot`），
通过 `run-as com.linuxhub.husk` 在 guest loader 下运行。

## 1 机制级：dlopen 进来的 .so 读 guest 文件

`libreadname.so` 里只有一个 `fopen("/etc/os-release")`；`dlmain` 用**guest 绝对路径**
`/tmp/husk-dlopen/libreadname.so` dlopen 它。

```
=== 旧 loader (libld39.so.bak-dlopen) ===
rc=-1 hostname=                      # fopen 失败：路径没被改写，落在宿主 /etc
=== 新 loader ===
rc=0 hostname=PRETTY_NAME="Ubuntu 24.04.4 LTS"
```

`NR_BOOT_DEBUG=1` 的计数（同一次运行）：

```
nr-boot: imports=0x21 objects=0x4 slots=0x1 bodies=0
nr-repatch: call=0x1 objects=0x5 new=0x5 slots=0x1 failed=0
```

`objects` 从 4 变 5 = 新加载的那个 DSO；`new=0x5` = 第一次 repatch 把整条链都当新对象处理
（缓存当时是空的），之后同一进程再 dlopen 只 patch 增量。

## 2 组件级：dlopen 进来的 libglib 读文件

`giofile2` dlopen `libglib-2.0.so.0`（RTLD_NOW），调 `g_file_get_contents()`：

```
guest path: r=1 errno=0 len=717 err=(none)
host  path: r=1 errno=0 len=717 err=(none)
guest dir : r=1 errno=0 len=400 err=(none)
```

`ls -l` 同一文件 717 字节 —— 长度逐字节对上，说明 libglib 内部的 `open64()` 已经落在
rootfs 上。旧 loader 上同一个程序直接 SIGSEGV（`segv|sig=11`），说明不修不是「慢」而是
「崩」。

## 3 对照实验：哪些**不是**这一层的问题

### 3.1 gdk-pixbuf 的文件加载失败与路径层无关

```
[0] /usr/share/icons/Yaru-blue/16x16/places/user-home.png -> (nil)
    Couldn't recognize the image file format for file "/usr/share/icons/..."
[1] /data/data/com.linuxhub.husk/files/rootfs-full/usr/share/icons/... -> (nil)
    Couldn't recognize the image file format for file "/data/data/com.linuxhub.husk/..."
```

**宿主绝对路径 [1] 也一样失败**（[1] 不需要任何改写），所以这不是路径改写的问题。

同一个库上，**喂字节**是成功的：

```
loader_write ok=1 err=(none)        # 把同一份 717 字节 PNG 直接写进 loader
```

即：PNG 识别本身没问题，问题在 gdk-pixbuf 的**文件**加载路径上。另有一条独立事实：

```
$R/usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders/   -> 14 个 .so
find $R/usr/lib -name loaders.cache                          -> 没有
formats=2 (png, jpeg)                                        # 只有内置的两个
```

没有 `loaders.cache` ⇒ 14 个模块一个都没注册。两条都要单独处理，不能算到
「dlopen 路径层」头上（这是 evidence 99 里那条归因的修正）。

### 3.2 仍然存在的边界

- dlopen 的对象**自己的构造函数**在 `dlopen` 返回前跑完，构造函数里的 `open()`
  这一次仍走宿主。本机实测 gdk-pixbuf / GIO / libspa 的构造函数都不碰文件。
- libc 内部直接调用（`__open_nocancel` 等，无 GOT 槽）继续由 `libc-interpose.md`
  的构建期槽位方案解决。

## 4 复现命令

```sh
F=/data/data/com.linuxhub.husk/files
R=$F/rootfs-full
export NR_ROOTFS=$R GLIBC_TUNABLES=glibc.pthread.rseq=0
LP="$R/usr/lib/aarch64-linux-gnu"
cd $R/tmp/husk-dlopen
$F/libld39.so --library-path "$LP" ./dlmain  /tmp/husk-dlopen/libreadname.so
$F/libld39.so --library-path "$LP" ./giofile2
$F/libld39.so --library-path "$LP" ./pixinfo3
```

回归门槛：`nr-tools embed` 的 sha256 断言照旧；bash/GNOME 的 10 分钟长稳脚本
（evidence 32）在新 loader 上重跑一遍，确认 repatch 没有引入第二条状态。
