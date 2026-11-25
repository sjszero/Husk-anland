# 181 · 0.0.4 发布产物端到端：门禁 A → 打包 → 全新目录装一遍 → 再跑门禁 A → 发版

日期：2026-11-25 08:2x–08:4x（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4 KiB 页），包 `com.linuxhub.myth.husk`。

这一条是 0.0.4 的发布取证：**先证明主树是绿的，再从主树打产物，再用产物装一棵新树，再验新树**，
最后才发版。

## 1 · 主树门禁 A（发版前）

```
PASS=36 FAIL=0
```

分组与 0.0.3 时一致：identity / bash / locale / 用户管理 / dpkg / apt / 宿主路径泄漏 /
seccomp 陷阱。两处 NOTE 也一致（`apt-get -s` 离线无索引、对方视角 argv 能看到本应用目录）。
差别只有一项数字：argv 那条这次是 **82 个进程**（上次 72）——那是进程数，不是缺陷。

## 2 · 产物

| 文件 | 字节 | sha256 |
|---|---:|---|
| `husk-arm64.tar.gz` | 1 913 632 | `430921230e0f21d78a1e5e2370560f22697df8016d507cbb1714cab8d8a03523` |
| `husk-rootfs-ubuntu-24.04.tar.gz` | 1 297 284 273 | `ab949009870f01cf57da5cad3282e6bc710c8584d29550007705c40de28660a0` |
| `SHA256SUMS` | 182 | 上面两行的两份哈希 |

- 启动器包由 `make dist`（`NDK=<ndk>/bin`，rust-lld + NDK clang 驱动 + `/tmp/nr-toolchain/root`
  的 glibc sysroot）产出；`tar tzf` 里是 `libld.so` / `libc.so.6` / 四个 guest 工具 /
  `guest-config/husk-gpu-defaults.sh` / `install-husk.sh` / `husk-session.sh` / `VERSION`，
  `VERSION` 内容是 `0.0.4`。
- 树包由设备上的 `tools/guest/make-release.sh` 从**刚刚跑过门禁 A 的那棵树**打：
  **102 126 个条目**，`11m24s`，`gzip -t` 通过；包内含
  `etc/profile.d/husk-gpu.sh`、`usr/lib/aarch64-linux-gnu/libc.so.6`（`69ceef3759f4…`，
  与启动器包里那份逐字节一致）、`usr/bin/gl_probe`；**没有** `./data`、`./storage`、`./sdcard`
  这类 guest 里造出来的宿主路径镜像（`make-release.sh` 的前置拒绝项没触发）。
- 设备侧与主机侧 sha256 一致（`ab949009…`），1.30 GB 没有传坏。

## 3 · 用发布物装一棵新树

设备上 `files/linux/rel-test`，只放发布出来的两个包，然后跑包里的安装器：

```
sh install-husk.sh --rootfs <dist>/husk-rootfs-ubuntu-24.04.tar.gz --dest ./root
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
```

装完自检：`usr/bin/bash`、`usr/bin/apt-get`、`etc/profile.d/husk-gpu.sh` 都在，
`usr/lib/aarch64-linux-gnu/libc.so.6` = `69ceef3759f4…`。安装器还给出了启动器
`rel-test/husk`。

## 4 · 新树再跑门禁 A

```
PASS=34 FAIL=0
```

比主树少 2 项，原因写在输出里：`no .deb on this tree (offline cache already removed) -
reinstall checks skipped`——树包按设计排除了 `var/cache/apt/archives`，所以「用缓存里的 deb
强制重装一遍」这两项在这里没有素材。这与 0.0.3 的 evidence 172 结果一致（当时也是 34/34）。

## 5 · 发版

- `VERSION` = `0.0.4`、`CHANGELOG.md` 最新一节 `## [0.0.4] - 2026-11-25`、
  链接 `[0.0.4]: …/releases/tag/v0.0.4` 三处由 `make version-check` 强制对齐（实测通过）；
- annotated tag `v0.0.4`（不移动）；
- Release `Husk 0.0.4`，非 draft、非 prerelease：
  <https://github.com/xishanp/Husk/releases/tag/v0.0.4>；
- GitHub 侧对上传字节算出的 digest 与本地 sha256 **逐个相同**：
  `husk-arm64.tar.gz` `43092123…`、`husk-rootfs-ubuntu-24.04.tar.gz` `ab949009…`
  ——所以「上传后的资产 = 本地验过的字节」是平台侧的证据，不是我这边口头保证。

## 没做的事（照实记录）

- 没有从 GitHub 把 1.30 GB 的资产**下回来**再校验一遍（上传 digest 已由平台侧给出，
  但没有端到端重下一次）；
- 没有在第二台设备上装（只有这一台）；
- 16 KiB / 64 KiB 页仍是 QEMU 结论，真机没验。
