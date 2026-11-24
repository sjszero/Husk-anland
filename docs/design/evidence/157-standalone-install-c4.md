# evidence 157 — 不装 APK 的独立安装跑通（C4）：装、起 bash、跑 apt、GL 落 GPU

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64。
时间：2026-11-24 10:31–10:40（UTC，设备时钟）。执行域是 `adb shell`（`shell` uid），
**不在应用私有目录里，也没有安装任何一个 APK**：目录是 `/data/local/tmp/husk-c4`。

## 1. 装（FACT）

```sh
adb push dist/husk-arm64.tar.gz /data/local/tmp/husk-c4/
adb push linux-rootfs.tar     /data/local/tmp/husk-c4/      # 3.5 GB，来自目标机上验证过的树
cd /data/local/tmp/husk-c4 && tar xzf husk-arm64.tar.gz
sh install-husk.sh --rootfs linux-rootfs.tar --dest ./root --loader ./libld.so
```

产物摘录：

```
== pass 1: extract (hard-link errors are expected) ==
== pass 2: re-extract through the shim (fixes hard links) ==
== symlinks: absolute -> relative ==
   rewrote: etc/alternatives/editor : /bin/nano -> ../../bin/nano
   skip (dangling): etc/pulse/client.conf.d/01-enable-autospawn.conf -> /run/pulseaudio-enable-autospawn
   links=29476 absolute=5 rewritten=4 dangling=1 left=0
   links=29476 absolute=1 rewritten=0 dangling=1 left=0      # --verify
== libc ==
   libc 69ceef3759f4 matches the bundle
== verify ==
   PRETTY_NAME="Ubuntu 24.04.4 LTS"
   bins=1035
== ok: hard links materialised (perl5.38.2 + uncompress are real files) ==
   sonames ok                       # ctypes.CDLL('libblas.so.3') + liblapack.so.3
```

`.husk-install-state`：

```
version=0.01
libc_sha256=69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c
loader_sha256=d5dd7e635a2a36798a792b1e4ae30d7af748e25518b4a9c6d6bfe1744a9991b8
installed=2026-11-24T10:31:10Z
```

## 2. 起 shell（FACT）

```
$ ./husk --version
husk 0.01
$ ./husk run ubuntu -c 'id; head -1 /etc/os-release; uname -m'
uid=0(root) gid=0(root) groups=0(root)
PRETTY_NAME="Ubuntu 24.04.4 LTS"
aarch64
```

## 3. 包管理器（FACT）

```
$ ./husk run ubuntu -c 'apt-get update'
Fetched 38.1 MB in 23s (1680 kB/s)
$ ./husk run ubuntu -c 'apt-get install -y --no-install-recommends cowsay; echo rc=$?'
rc=0
$ ./husk run ubuntu -c 'cowsay -t moo'
 _____
< moo >
```

## 4. GL 在这套独立安装里落在 GPU（FACT）

会话脚本默认把树里带的 kbase 后端 PanVK 的 ICD 指给 Vulkan，并让 GL 走 Zink：

```
$ ./husk run ubuntu -c 'eglinfo -B | grep -i renderer'
OpenGL compatibility profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

（`HUSK_GPU=llvmpipe` 可以退回软件路径。）

## 5. 这次跑出来、并且修掉的六个问题

| # | 现象 | 根因 | 修法 |
|---|---|---|---|
| 1 | `install-husk: loader not executable` | `make dist` 把 `libld.so` 复制成 644，tar 保留模式位 | dist 补 `chmod 755` |
| 2 | `gzip: gzclose: Inappropriate ioctl for device`，但文件是完整的 | 本机 toybox `gzip -dc > file` 写完仍以 1 退出 | `.gz` 直接交给 toybox tar，不再暂存 |
| 3 | 传纯 `.tar` 时一行都没解出来 | 按 magic 分派后没有 `*)` 分支 | 补 plain tar 分支 |
| 4 | `./husk run ubuntu -c 'id'` → `usr/bin/env: '/-c': No such file` | 启动器把 `-c` 当成程序名传下去 | 首参以 `-` 开头时前置 `usr/bin/bash` |
| 5 | `apt-get update` → `Temporary failure resolving` | release 包按设计不含 `etc/resolv.conf`，独立安装没人补 | 安装器写 `resolv.conf`（先取 `net.dns1/2`，再兜底 1.1.1.1、8.8.8.8） |
| 6 | `E: Directory '/var/log/apt/' missing`，装完仍返回非 0 | release 包按设计不含 `var/log` | 安装器建 `var/log/apt`、`var/log/dpkg.log` |

前五个是**必须修**才装得上/用得了；第 6 个只影响退出码和日志。

## 6. 边界（UNKNOWN）

- 兜底 DNS 用的是 1.1.1.1 / 8.8.8.8：在网络限制这些地址的环境里，`apt` 仍会失败。
  更稳的做法是让应用侧把系统的 DNS 写进 `resolv.conf`（APK 路径已经是这么做的）。
- `tail -f`、`gsettings` 一类依赖会话总线的东西没在这次独立安装里重跑，只在应用路径下验过。
- 图形会话（真实的 X/Wayland 桌面窗口）没有在这套独立安装里起过；这一项仍未验收。

## 7. 发布前复验（2026-11-24 后续，同一台设备）

把上面的修改打成 0.02 的包后，用新的包重跑了两遍，避免「改了没验」：

| 跑的什么 | 根包 | 结果 |
|---|---|---|
| `b02/` | 3.5 GB 的 `.tar` | 安装全绿（符号链接 4 改 1 悬空 0 残留、libc 匹配、硬链接成立、sonames ok） |
| `c02/` | 1.5 GB 的 `.tar.gz`（**发布里那个产物**） | 同上，全绿 |

两次都验到：

```
$ ./husk --version
husk 0.02
$ ./husk run ubuntu -c 'id; uname -m'
uid=0(root) gid=0(root) groups=0(root)
aarch64
$ head -2 root/etc/resolv.conf
nameserver 1.1.1.1
nameserver 8.8.8.8
$ ./husk run ubuntu -c 'apt-get update'
Fetched 38.1 MB in 47s (806 kB/s)
```

结论：**发布里的两个产物（启动器包 + 1.5 GB 的树）都在目标机上装得上、起得来、能装包。**
这条同时说明前面那次 `.gz` 失败是**我们导出的包被截断**（toybox gzip 的 gzclose 行为），
不是 toybox tar 不会解 gz。
