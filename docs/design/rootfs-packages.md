# 2.0 rootfs 上补齐的包（可复现清单）

> 目的：别人照文档把 2.0 的 Ubuntu 24.04 链解出来之后，能复现我们跑通的同一棵树。
> **只解到 Husk 私有目录**（`files/rootfs-full`），不读写 `com.linuxhub.myth` 的任何数据。

## 1. 缺口与原因

| 包 | 缺了会怎样 | 原因 |
|---|---|---|
| `liblzo2-2` | `libgvfscommon.so` 加载失败 | base tar 里带了 gvfs，但压缩后端没带 |
| `gvfs-libs` / `gvfs-common` | nautilus / 文件选择器起不来 | 同上，且 `libgvfscommon.so` **没有版本后缀**，在 `usr/lib/aarch64-linux-gnu/gvfs/` 下 |
| `shared-mime-info` | **所有图片格式都报 `Unrecognized image file format`** | `libgdk_pixbuf` 编译期开了 `GDK_PIXBUF_USE_GIO_MIME`，靠 mime 数据库选 loader；`/usr/share/mime` 只有 `packages/`、没有 `mime.cache` |
| `libgdk-pixbuf2.0-common` | 同上，且缺 `loaders.cache` | loader 注册文件不在 base tar 里 |

## 2. 怎么补

包源只用 2.0 自带的离线 tar（`LinuxHub-2.0/app/src/main/assets/offline/`），不联网取包：

```sh
# 解出 deb（tar 里就是普通 deb 文件），再解包到 rootfs
dpkg-deb -x shared-mime-info_2.4-4_arm64.deb  <rootfs>
update-mime-database <rootfs>/usr/share/mime
```

`gdk-pixbuf` 的 loader 缓存：`gdk-pixbuf-query-loaders` 在
`usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/`（不在 `usr/bin`），生成后要把缓存里的宿主前缀
改成 guest 路径，否则 loader 路径全指到宿主机。

## 3. 验收（实测）

| 检查 | 修前 | 修后 |
|---|---|---|
| `Pixbuf.get_file_info("*.png")` | `Unrecognized image file format` | `png` |
| `Pixbuf.new_from_file(png)` | 失败 | 16×16 |
| SVG | 失败 | 300×300 |
| `loaders.cache` 里注册的格式数 | 2 | 16 |
| `imports.gi.Gtk` | 20 秒无输出被 timeout 杀 | 立刻返回 `GTK 4 14` |

## 4. 库路径

`--library-path` 是**替换**不是追加：必须显式包含
`$R/lib/aarch64-linux-gnu`（merged-/usr 下的真身）与 `$R/usr/lib/aarch64-linux-gnu/gvfs`，
否则 gvfs / lzo / 部分 GIO 模块找不到。
