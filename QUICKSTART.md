# 从零跑通

从一台干净的开发机到 `bash` 在手机里跑起来。**这条路径是开发路径**；交付形态是
[`docs/RUNNING.md`](docs/RUNNING.md) §1 的「打进 APP」。

> [!WARNING]
> 先看 [`docs/RUNNING.md`](docs/RUNNING.md) §2 的三条硬前提。arm64 之外、或者沙箱不允许
> 执行的目录，后面每一步都会失败，而且报错看起来像 loader 坏了。

## 0 · 需要什么

| 项 | 要求 |
|---|---|
| 主机 | macOS 或 Linux，能跑 `adb` |
| Rust | stable，带 `aarch64-unknown-linux-gnu` 与 `aarch64-linux-android` 两个 target |
| NDK | 能提供 `clang` 与 `llvm-objcopy`，通过 `$NDK` 指到 `bin` |
| 设备 | 能 `adb` 的 arm64 Android；不需要 root、不需要解 Bootloader、不需要开虚拟化 |

```sh
rustup target list --installed | grep aarch64
adb devices
```

## 1 · 生成 rootfs

```sh
cargo build --release -p nr-tools --manifest-path runtime/Cargo.toml
./runtime/target/release/nr-tools rootfs \
  --out /tmp/husk-rootfs --tar /tmp/husk-rootfs.tar.gz
```

它解析镜像索引 → 下载包 → 解包成目录 → 补 usr-merge 链接、`/etc/passwd`、`dpkg` 状态库、
`apt` 源 → 把 guest libc 里两处会被 Android 拒掉的调用 `nop` 掉 → 打 tar。

## 2 · 产出配套 loader

```sh
export NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin
sh tools/loader/build-embedded-loader.sh
```

loader 必须和它加载的 libc 同源：`nr-tools scan-syscalls` 在构建期断言 glibc 版本与
`set_robust_list` / `rseq` / `clone3` 的 site 数量，对不上就直接报错。

## 3 · 打资产包

```sh
make dist LIBC=<pristine glibc 2.39 libc.so.6> NDK=<ndk>/toolchains/llvm/prebuilt/<host>
# 产出 dist/husk-arm64.tar.gz + dist/SHA256SUMS
```

包里有：`libld.so`、`libc.so.6`、`husk-ldconfig`、`husk-usermgr`、`husk-su`、
`install-husk.sh`、`husk-session.sh`、`husk-desktop.sh`、`husk-ding-fallback.sh`、
`husk-wallpaper-fit.sh`、`guest/wallpaper-scale`、`guest-config/`、`VERSION`。
少一个文件构建就会失败（`make dist` 里有逐条 `tar tzf | grep -qx` 断言）。
clean tree 必须提供 `LIBC`；如果设备树已经打过补丁，也可以先运行 `make libc-patch`，
再执行不带 `LIBC` 的 `make dist`。

## 4 · 装到设备

支持的目标是「能执行、可写、沙箱允许」的目录。开发机上是 `/data/local/tmp`，设备上是应用
私有目录（`run-as <包名> …`）：

```sh
adb push dist/husk-arm64.tar.gz /data/local/tmp/husk/
adb shell
cd /data/local/tmp/husk && tar xzf husk-arm64.tar.gz
sh install-husk.sh --rootfs linux-rootfs.tar --dest ./root --loader ./libld.so
```

`install-husk.sh` 会解两遍树：第一遍铺开、容忍硬链接失败；第二遍**经 shim** 解，靠 `link()`
回落成拷贝，把 `perl5.38.2`、`uncompress` 这类硬链接名变成真文件。装完它自己会验一遍
`/etc/os-release` 与硬链接，验不过就退出 70。

## 5 · 跑

```sh
./husk --version                      # 打印 VERSION 里的版本号
./husk run ubuntu -c 'id; head -1 /etc/os-release'
./husk                                # 交互式 bash

# 起一个可用的图形会话（唯一入口：清残留 → 双总线 → 起 shell → 自检 → 输入预热）
./husk-desktop.sh root
# 期望四行：CHECK wm=yes / CHECK ding=yes / CHECK gpu=zink / CHECK input=primed
```

验收清单（全部返回 0，且输出来自 rootfs 自己）：

```sh
dpkg -l | wc -l
apt-get update && apt-get install -y vim
gcc --version | head -1
python3 -c "import numpy; print(numpy.arange(200000).sum())"
getent passwd | head -3
```

## 6 · 常见问题

| 现象 | 原因 / 处理 |
|---|---|
| `cannot execute the loader from here` | 目录不可执行（`noexec`）或沙箱不允许，换到应用私有目录 |
| `Exec format error` | 设备不是 arm64 |
| `Bad system call` | libc 补丁没打：`make libc-patch` 重新打（evidence 170） |
| 库存在却 `ENOENT` | rootfs 里的绝对符号链接没改成相对：`husk-symlinks --root <rootfs> --apply` |
| `dpkg` 报 `Can't stat /lib` | 静态 `ldconfig` 没换成 `husk-ldconfig` |
| `su: System error` | `husk-su` 没装进 `usr/sbin/` |
| `make` 的输出消失 | 不是运行时问题：`adb shell` 给 guest 的 stdout 是 socket，重定向到文件再看 |
