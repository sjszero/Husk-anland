# 296 · nr-tools 暂存：把 dpkg 记录补全，把状态库换成包自己的 control

日期：2026-11-26（CST）。开发机 macOS arm64 + 真机 RMX3357 / MT6893 / Android 13 /
kernel 4.19.191。版本 `VERSION=0.0.36`（未发版）。

第 4 轮开工第一件事。上一轮在测试树上量到 P1：**862 个包标了 installed，只有 92 个
`.list`**（evidence 294 §7）。这一轮把根因修掉，顺手挖出三个同一处的缺陷。

## 0 · 结论

| 编号 | 缺陷 | 状态 | 判据 |
|---|---|---|---|
| P1 | 解包 deb 拼树时不写 `/var/lib/dpkg/info/<pkg>.list`：`dpkg -L` 空、`apt purge` 删不掉文件、`dpkg -C` 恒报 | **FIXED** | 真机 `dpkg --admindir` 验收：`dpkg -C` **rc=0 无输出**；`dpkg -L hello` 出 14 条；删掉 `.list` 立刻报 1 条，删掉 `.md5sums` 立刻报 1 条 |
| P2 | `nr-tools rootfs` 的**索引下载路径从来不调用 `finish()`**：包解包完就退出 0，没有 `var/lib/dpkg/status`、没有 libc 补丁、没有 loader、没有 tarball | **FIXED** | 修前 `/tmp/nr-check2` 无 `var/lib/dpkg/status` 且退出码 0；修后同一条命令产出 4 个包的完整状态库 |
| P3 | 状态库是**桩**（name + version + "staged by nr-tools rootfs"）：dpkg 每次解析都警告 `missing 'Maintainer' field`，apt 拿不到 `Depends`/`Provides`/`Conflicts`/`Breaks`/`Replaces` | **FIXED** | 现在直接用 deb 自带的 `control` 原文，只在 `Package:` 后面插一行 `Status: install ok installed`；hello 那段带 `Depends: libc6 (>= 2.38)`、`Conflicts`、`Breaks`、`Replaces` |
| P4 | usr-merge 链接的存在性用 `Path::exists()` 判断，它跟随符号链接：`sbin -> usr/sbin` 在最小包里是**悬空但正确**的链接，于是每次暂存都打印 `usr-merge links MISSING` | **FIXED** | 改用 `symlink_metadata`（`lexists`）；单测 `lexists_counts_a_dangling_symlink` |

- **FACT**：`cargo test -p nr-tools` → **4 passed**（P1 两条、P4 一条、libc patch 一条）。
- **FACT**：`make check` 全绿。

## 1 · P1：`.list` 从刚解出来的那份 data.tar 写

`provision::extract_tar` 现在返回"这次真正落盘的条目"，`unpack_deb` 把它带出来，
`write_dpkg_info` 据此写三样东西：

| 文件 | 来源 |
|---|---|
| `<pkg>.list` | `/.\n` + 每个落盘条目的绝对路径 |
| `<pkg>.md5sums` | deb 的 `control.tar` 里的 `md5sums`，原样拷贝 |
| `<pkg>.conffiles` | 同上，`conffiles` |

包不带 `md5sums`/`conffiles` 时不写空文件（单测 `dpkg_info_skips_control_files_the_package_does_not_ship`）。

**FACT（真数据）**：拉 `hello 2.10-3build1 arm64` 真包，生成的 `hello.list` 与该包
自带 `data.tar.zst` 的成员清单**逐条相同**（14 条，`diff` 为空）。

## 2 · 真机用 dpkg 自己验收（不是我看文件像不像）

把新代码产出的 `status` + `info/` 整份丢到真机 guest 里，用 `dpkg --admindir` 直接问 dpkg：

```
== 新代码产出的整份 dpkg 库：dpkg -C ==
rc=0                       ← 一个字都没说
== dpkg -L hello ==
/.
/usr
/usr/bin
/usr/bin/hello
...
== 负向：删掉 hello.list 后 ==
1                          ← missing the list control file 的条数
== 负向：删掉 hello.md5sums 后 ==
1                          ← missing the md5sums control file 的条数
```

这是 P1 的判据：正向静默、负向各报一条。上一轮那棵树上 `dpkg -C` 是 1722 行。

## 3 · P2：索引路径（网络装包）从来没跑完

`finish()`（usr-merge 链接、账户文件、状态库、libc 补丁、loader、tar）只被
`--base/--debs` 那条分支调用。走索引下载的那条路在 `println!("unpacked …")` 之后就结束了，
而且退出码 0 —— 树里有全部文件，apt 眼里一个包都没装。

```
# 修前
$ nr-tools rootfs --packages hello --out /tmp/nr-check2 --fresh --no-patch-libc
4 packages, 2.9 MB
unpacked 4 packages into /tmp/nr-check2
$ ls /tmp/nr-check2/var/lib/dpkg/      # 只有 info/，没有 status
info
# 修后
$ ls /tmp/nr-check3/var/lib/dpkg/
info  parts  status  updates
$ grep -c '^Status: install ok installed' /tmp/nr-check3/var/lib/dpkg/status
4
```

## 4 · P3：状态库要写包自己的说法

apt 解析**已安装包**的依赖关系读的就是这份 `status`。桩里没有 `Depends`，apt 就认为
这些包谁都不依赖谁；`dpkg` 还会在每次解析时警告 `missing 'Maintainer' field`。
现在 `control_stanza()` 从 deb 的 `control.tar` 里取 `control` 原文，写库时只在
`Package:` 行后插一行 `Status: install ok installed`（已有 `Status:` 行则丢弃重写）。

修后 hello 在库里的样子（真机 `dpkg -L`/`-C` 都接受）：

```
Package: hello
Status: install ok installed
Version: 2.10-3build1
Architecture: arm64
Maintainer: Ubuntu Developers <ubuntu-devel-discuss@lists.ubuntu.com>
Installed-Size: 144
Depends: libc6 (>= 2.38)
Conflicts: hello-traditional
Breaks: hello-debhelper (<< 2.9)
Replaces: hello-debhelper (<< 2.9), hello-traditional
```

## 5 · 没做的

- **UNVERIFIED**：没有重跑整棵桌面树（862 个包）。上面四条都用 `hello` + 依赖（4 个包）
  验证到底；全量重跑属于第 4 轮的"干净目录安装"，要么走 `--base/--debs`（本地 deb 集），
  要么付一次全量下载。**在整棵树重跑之前，不能声称门禁 A 已经 36/0。**
- `dpkg` 还要求 `<pkg>.shlibs`/`<pkg>.symbols`（有的包带），本工具没写；`dpkg -C` 不查这两样，
  但 `dpkg-shlibdeps` 会用到。**UNKNOWN**：这条在磁盘上会不会在真实使用中咬人，还没证据。
