# 中文界面翻译包

`husk-lang-zh.tar.gz`（5 186 077 字节，sha256
`4f9ba4b75202af8dfcbadd802f5e8f7f02c38055aa9f527269ea9907b013a379`）

## 里面是什么

从 Ubuntu 24.04（noble）的两个包解出来的 `zh_CN` 翻译，只留 `.mo`：

- `language-pack-zh-hans-base_24.04+20260905_all.deb`（222 个 `.mo`）
- `language-pack-gnome-zh-hans-base_24.04+20260905_all.deb`（191 个 `.mo`）

合计 413 个，落到 `usr/share/locale-langpack/zh_CN/LC_MESSAGES/`。Ubuntu 的 glibc 会把
`/usr/share/locale-langpack` 一起搜，所以不需要动 `/usr/share/locale`（那里已有的 19 个 `.mo`
保持原样）。

## 为什么单独打包，不重打 rootfs

参考 rootfs 有 1.3 GB，只为了塞 5 MB 翻译重打一次不划算。安装器在树包旁边看到这个文件就解开，
没有就跳过并说明界面会保持英文。

## 怎么重建

```sh
curl -O http://ports.ubuntu.com/ubuntu-ports/pool/main/l/language-pack-zh-hans-base/language-pack-zh-hans-base_24.04+20260905_all.deb
curl -O http://ports.ubuntu.com/ubuntu-ports/pool/main/l/language-pack-gnome-zh-hans-base/language-pack-gnome-zh-hans-base_24.04+20260905_all.deb
mkdir -p stage && for d in *.deb; do bsdtar -xf "$d" --include data.tar.xz; tar xf data.tar.xz -C stage; done
tar czf husk-lang-zh.tar.gz -C stage usr/share/locale-langpack
```

版本号跟着 Ubuntu 24.04 的 language-pack 走；换版本时记得改这个文件里的 sha256。
