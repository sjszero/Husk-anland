# 308 · 安装器图像缓存路径与最终空目录安装（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`
安装树：`files/release-tree-final/root`

## 0 · 结论

- **FACT**：修前安装器把 gdk-pixbuf cache 写到
  `.../2.10.0/loaders/loaders.cache`，正确路径是
  `.../2.10.0/loaders.cache`。安装本身仍能返回成功，但 PNG/SVG loader 不注册，
  GTK 启动后报 `Unrecognized image file format` / `Could not load a pixbuf`。
- **FACT**：修复后从空目录安装 `release-tree-v2.tar.gz`，安装器返回 `INSTALL_RC=0`。
  输出包含 `mime.cache (159188 bytes)` 和 `loaders.cache (14 loaders)`。
- **FACT**：同一棵最终树通过 `TREE_INTEGRITY PASS=23 FAIL=0`，其中 gdk-pixbuf cache
  路径、缓存内容、dpkg 记录和 locale 均逐项检查。
- **FACT（2026-11-27 后续复验）**：这棵最终树已切换到 App 的正式入口
  `files/rootfs-full -> release-tree-final/root`。切换后 `A-hw PASS=11 FAIL=0`，
  Tree Integrity 仍为 `PASS=23 FAIL=0`，Gate A 仍为 `PASS=34 FAIL=0 SKIP=1`。
  独立安装树与 App bridge 路径曾经分离，但该差异已通过正式入口切换消除，见 evidence 310 §4。

## 1 · 修前失败

修复前安装日志中的路径：

```text
usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders/loaders.cache
```

正确路径应为：

```text
usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders.cache
```

GTK 实际错误：

```text
Unrecognized image file format
Could not load a pixbuf
```

安装器没有因此失败，所以只检查退出码会把坏树当成功。此问题由
`20-tree-integrity.sh` 在标准路径检查时暴露。

## 2 · 修复

`tools/guest/install-husk.sh` 现在用：

```sh
_gb_cache="${_gb_dir%/loaders}/loaders.cache"
```

修复后空目录安装的关键输出：

```text
wrote usr/share/mime/mime.cache (159188 bytes)
wrote usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders.cache (14 loaders)
== done ==
```

## 3 · 最终树复验

命令：

```sh
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
  ROOTFS=files/release-tree-final/root EXPECT=desktop \
  sh tools/device-tests/20-tree-integrity.sh
```

结果：

```text
PASS  usr/bin/bash present
PASS  etc/os-release: Ubuntu 24.04.5 LTS
PASS  shared MIME cache present (159188 bytes)
PASS  gdk-pixbuf loader cache registers 14 guest loaders
PASS  gdk-pixbuf loader paths are guest-relative
PASS  dpkg records: 807 .list for 807 installed package(s)
PASS  locale-archive present (6222864 bytes)
PASS  etc/default/locale selects zh_CN (LANG=zh_CN.UTF-8 LANGUAGE=zh_CN:zh)
PASS  zh_CN langpack: 413 .mo files
TREE_INTEGRITY PASS=23 FAIL=0
```

## 4 · 发布资产

`make dist` 重新点验后生成：

```text
e8cb69d066baec5c0a0a8ca1e0ee1d65159e7eb1aa665bed01b1582d2da8505f  dist/husk-arm64.tar.gz
```

这个包包含 `husk-window-wait`、安装器、桌面脚本、PanVK 驱动和 `SHA256SUMS`；
该哈希对应本轮最终重建的 dist；正式发布仍以 tag、Release 资产和 `release-audit` 为准。
