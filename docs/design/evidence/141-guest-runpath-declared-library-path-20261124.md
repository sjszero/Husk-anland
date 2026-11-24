# 141 · guest 自己声明库目录：把 `DT_RUNPATH` 变成加载器的搜索路径

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
App 自建树（`distributions/ubuntu-24.04/rootfs`），资产 `husk-runtime-spawn-link-9`。

## 0 · 结论（FACT）

139 §3.1 记的两个「文件在、加载器找不到」被同一个改动修掉了：

```text
修之前
$ … usr/bin/sudo --version
usr/bin/sudo: error while loading shared libraries: libsudo_util.so.0
$ gsettings get …        → libpxbackend-1.0.so: cannot open shared object file

修之后（同一棵树、同一个 loader）
$ … usr/bin/sudo --version
sudo: /etc/sudo.conf is owned by uid 10265, should be 0     ← 加载成功，卡在下一层
$ gsettings get org.gnome.desktop.interface clock-format
'24h'                                                     ← 没有任何 libpxbackend 警告
```

修法不是打补丁到具体目录，而是**问 guest 自己**：新增
`husk-ldconfig --library-path-file /etc/ld.so.husk-path`，它走一遍 `/usr/bin`、`/usr/sbin`、
`/usr/libexec`、`/usr/lib`、`/usr/games`、`/usr/local`（递归），读出每个 64 位 little-endian
ELF 的 `DT_RUNPATH`(29) / `DT_RPATH`(15)，把其中的**绝对目录**去重排序写进文件；
App 侧 `HuskRuntime.libraryPath()` 把这份清单（加 rootfs 前缀）接到 `--library-path` 后面。

实测产出 21 个目录：

```text
/usr/lib/aarch64-linux-gnu/{NetworkManager/1.46.0,deja-dup,eog,gvfs,libmsgraph,libproxy,
                            libunity,mutter-14,pipewire-0.3,pulseaudio,samba,systemd,
                            tracker-3.0,tracker-miners-3.0}
/usr/lib/{gnome-settings-daemon-46,gnome-shell,lp_solve,man-db}
/usr/lib/pulse-16.1+dfsg1/modules
/usr/libexec/sudo
```

其中 `mutter-14` / `gnome-shell` / `gvfs` / `systemd` 正是 2.0 当年一个个手加的
（`libraryDirs`），现在由 guest 自己报出来 —— 那四个条目降级为「文件还没生成时的兜底」。

## 1 · 为什么不能靠截获

glibc 在**加载器内部**拼 `RUNPATH 目录/文件名` 再 `openat`，那里没有 GOT 可换；我们的 shim
只 interpose 符号。`/etc/ld.so.cache` 也一样：加载器按 `/etc/ld.so.cache` 这个绝对路径读，
没被改写，于是缓存根本没被用上（这也是为什么 husk-ldconfig 写了 938 条缓存却没人读）。
真正的系统级修法是改加载器里 `open_path` 的打开点（与 `clone3`/`rseq` 同一类改写），
本轮没做；先把「声明」这条路走通，代价小、可验证、不动加载器。

`$ORIGIN` 形式的 RUNPATH 被**故意跳过**：目标文件本身就是从 rootfs 内的真实路径打开的，
`$ORIGIN` 已经解析正确，改写反而会坏。

## 2 · 实现要点

- `elf_runpath()` 只读 ehdr → phdrs → PT_DYNAMIC → dynstr 这几段，不整file读；
  `/usr/lib` 下几千个对象，整读在手机上不可接受。
- `RUNPATH_ROOTS` 递归深度上限 8，`/bin`、`/sbin`、`/lib` 与 `/usr/...` 是同一目录
  （merged-usr），按 `canonicalize` 去重，避免重复遍历。
- 生成的文件首行是注释，其余每行一个 guest 绝对目录；App 只认以 `/` 开头的行。
- 生成时机（4 处）：初始化开始、`CREATE_CONTAINER` 装完包之后、`prepareContainerRuntime`
  （App 每次启动）、`startDesktop`（每次开会话）。**耗时实测 3.96 s**（user 0.5 / sys 2.1），
  只在上述时机跑，不进每个命令。

## 3 · 回归

- `14-gate-a-acceptance.sh` 改成用 `husk-exec.sh` 启动（见 §4），并且**先读
  `/etc/ld.so.husk-path`** 追加到搜索路径 —— 与 App 的行为一致，这样验收脚本量的就是
  真实启动方式。同一棵 App 树：`PASS=34 FAIL=0`（`EXPECT_PKGS=814`）。
- App 侧日志（`initialization.log`）里新一轮 `EXEC husk-library-path` 的 `library-path`
  已经包含全部 21 个目录，且去掉了上一轮 `rootfs//usr` 的双斜杠。

## 4 · 顺手修掉的验收脚本问题

`14-gate-a-acceptance.sh` 原来假设「rootfs 旁边有个 `husk` 启动器」、且不设 `PATH`/`HOME`，
换成 App 的树就跑不起来。现在它自带 `husk-exec.sh`（cwd 在 guest root、`--library-path`
显式给全并追加 guest 声明的目录、`HOME=/root`、`unset XDG_*`、`EXPECT_PKGS` 从 `env` 传入），
脚本本身只关心 `DEV/PKG/ROOTFS/LD` + `EXPECT_PKGS`。

## 5 · 仍未做（UNKNOWN / 下一步）

- **跨进程虚构所有权**（139 §3.2）：sudo 现在能加载，但立刻拒绝
  `/etc/sudo.conf is owned by uid 10265, should be 0`。要让它成立需要一张「路径 → 虚构
  uid/gid」的表（`chown` 写入、`O_CREAT`/`mkdir` 按当时身份写入、`stat`/`fstatat`/`statx`
  读出，默认 rootfs 内为 root），这是下一件事。
- 加载器里 `open_path` 的改写（让 `RUNPATH` 与 `ld.so.cache` 原生生效）仍是最干净的终态，
  本轮用「声明清单」绕开；两条路不冲突，前者落地后后者自动退化成冗余。
