# 172 · 发布产物端到端：`make dist` 的包 + 树 tar → 全新目录装一遍 → 门禁 A

日期：2026-11-24 23:20–23:40（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13）。
中间产物都在设备上，装完就删，没有留在树里。

## 1 · 产物

| 文件 | 字节 | sha256 |
|---|---:|---|
| `dist/husk-arm64.tar.gz` | 1 913 623 | `f97410e66478ba13301c4258a4df401c58ade02703e1490e0cf74d67a5c56f07` |
| `dist/husk-rootfs-ubuntu-24.04.tar.gz` | 1 424 404 564 | `084d74f2f6ca2f436a3811ca9dee3282db2e39b44a3b09042e685c214a02dca0` |

- 启动器包是 `make dist`（NDK clang + `/tmp/nr-toolchain/root` 的 glibc sysroot）产出的；
  里面是 loader（`libld.so`）、补丁版 `libc.so.6`、4 个 guest 工具、`guest-config/`、
  安装器、会话脚本、`VERSION`。
- 树包是从**正在跑的那棵树**打的（`tools/guest/make-release.sh` 的排除表：`tmp proc sys dev
  run root var/cache/apt/archives var/lib/apt/lists var/log var/lib/husk/owners
  etc/ld.so.husk-path etc/{hostname,resolv.conf,machine-id}`），
  102 110 个条目、`gzip -t` 通过；包里 `usr/lib/aarch64-linux-gnu/libc.so.6` =
  `69ceef3759f4…`（与 `tools/loader/libc-nop.so.6` 逐字节一致，evidence 170），
  `usr/bin/gl_probe` = `e3f607348d90…`（23:20 的构建），`etc/profile.d/husk-gpu.sh` 在包里。
- 传回设备后设备侧 sha256 与主机侧一致（`084d74f2…`），1.42 GB 没有传坏。

## 2 · 顺手查出来的两个打包缺陷（都已修）

| # | 缺陷 | 症状 | 修法 |
|---|---|---|---|
| 1 | `make dist` 打的启动器包**没带 `guest-config/husk-gpu-defaults.sh`** | 从发布包装出来的树**没有** `/etc/profile.d/husk-gpu.sh`，也就是没有 GPU 默认值，而且安装器**一声不响**（`if [ -f … ]` 没有 else） | Makefile 复制该文件进包，并在**打完的 tar 上**断言 `guest-config/husk-gpu-defaults.sh` 存在；安装器缺这个文件时打 WARNING 到 stderr |
| 2 | `make-release.sh` 用 here-document 写排除表 | mksh 的临时目录在启动时按 `TMPDIR` 定死，`run-as` 下不可写 → `can't create temporary file /data/local/shXXXX.tmp`；脚本只有 `set -u`，于是**软失败**：排除表是空的，`tar -X` 拿到空表，`tmp/proc/dev/run/root` 会全部被打进发布包 | 改用 `printf` 写文件、写完检查非空、失败即 `exit 70`；另外加了一条前置拒绝（`data`/`storage`/`sdcard` 存在就拒绝打包——那是 guest 里造出来的宿主路径自镜像，见 evidence 169 §3 的同源问题） |

两条都实测过：修好前，从包里装出来的树 `ls /etc/profile.d/` 是空的；修好后安装器打印
`wrote etc/profile.d/husk-gpu.sh (GPU defaults, escape: HUSK_GPU=llvmpipe)`。
排除表那条用「先造一个 `rootfs/data/` 再跑」验证，退出码 67、提示明确、不留半个 tar。

## 3 · 全新目录装一遍（用发布包，不是开发树）

```
sh install-husk.sh --rootfs ./husk-rootfs.tar.gz --dest ./root
== pass 1: extract (hard-link errors are expected) ==
== pass 2: re-extract through the shim (fixes hard links) ==
   wrote etc/resolv.conf (2 nameservers)
   wrote etc/profile.d/husk-gpu.sh (GPU defaults, escape: HUSK_GPU=llvmpipe)
== symlinks: absolute -> relative ==
   links=29476 absolute=5 rewritten=4 dangling=1 left=0
   links=29476 absolute=1 rewritten=0 dangling=1 left=0
== libc ==
   libc 69ceef3759f4 matches the bundle
== verify ==
   PRETTY_NAME="Ubuntu 24.04.4 LTS"   bins=1035
   -rwxr-xr-x. 1 root root 3942240 /usr/bin/perl5.38.2
   -rwxr-xr-x. 1 root root    2346 /usr/bin/uncompress
== ok: hard links materialised (perl5.38.2 + uncompress are real files) ==
   sonames ok
install rc=0
```

`absolute=5` 而不是 552：树在打包前已经规范化过（evidence 155），发布包里剩下的 5 个是
`etc/pulse/client.conf.d/01-enable-autospawn.conf`（悬空）之类，安装器再扫一遍只改 4 个。

## 4 · 装着门禁 A

```
DEV=… ROOTFS=files/linux/fresh/root LD=files/linux/fresh/libld.so \
  sh tools/device-tests/14-gate-a-acceptance.sh
PASS=34 FAIL=0
```

与主树那次的 36 项差 2 项，原因写在输出里：发布包的排除表不带
`var/cache/apt/archives`，所以 `dpkg-deb -I` 与「force-overwrite 重装」这两项
`NOTE  no .deb on this tree (offline cache already removed) - reinstall checks skipped`。
**是跳过，不是失败**，也不拿它当通过。

## 5 · 清理

- 设备上 `files/linux/fresh/`（装出来的树 3.9 GB + 上传的 tar 1.4 GB）用完即删。
- 顺手删了 7 个早先失败安装留下的 `files/linux/distributions.attempt{3,4,5,6,6run,7,8}`
  （每个约 3.35 GB，合计约 23 GB）。删前确认没有进程 cwd 在里面，正在跑的树
  （`files/linux/distributions/ubuntu-24.04/rootfs`）没有动，`df` 从 8.2 GB 空闲变成 33 GB。

## 6 · 没做的

- **APK 这条交付路径没验证**：`make apk` 需要 Gradle + Android SDK 构建，本轮只验证了
  「一条命令装进目录」这条（`install-husk.sh`）。0.0.3 的产物里也没有 APK。
- 只有 arm64 + glibc 2.39 这一套；16K/64K 页设备、其他发行版都没有跑。
- 装完的门禁 A 没有覆盖桌面（门禁 A 是命令行面的 36 项）；桌面的证据是 165/168/169/171，
  跑在主树上，不是这个新装的树上。**新装的树上没跑桌面**，这一点别混。
