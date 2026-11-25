# 211 · 中文界面：语言包单独打包，安装器解开

日期：2026-11-25 20:05–20:20（CST）。设备 RMX3357，`:0`。

## 1 · 为什么单独打包，不重打 rootfs

参考 rootfs 是 1.3 GB。中文界面缺的只是 413 个 `.mo`，压缩后 5 186 077 字节。为了 5 MB 重打
1.3 GB 并让每个用户重下，不划算；做成一个附加包，安装器在树包旁边看到就解开。

## 2 · 包里有什么（FACT）

从 Ubuntu 24.04（noble）的两个 `-base` 包解出来，只留 `.mo`：

| 包 | `.mo` 个数 | 大小 |
|---|---:|---:|
| `language-pack-zh-hans-base_24.04+20260905_all.deb` | 222 | 2 078 120 B |
| `language-pack-gnome-zh-hans-base_24.04+20260905_all.deb` | 191 | 1 714 548 B |
| 合计（tar.gz） | **413** | **5 186 077 B** |

落点是 `usr/share/locale-langpack/zh_CN/LC_MESSAGES/`。Ubuntu 的 glibc 会把
`/usr/share/locale-langpack` 一起搜，所以不用动 `/usr/share/locale` 里原有的 19 个 `.mo`。

## 3 · 实测（FACT）

同一个 `dgettext` 探针（`$R/tmp/t.c`），只换 `LANG`：

| 调用 | 无 LANG | `LANG=zh_CN.UTF-8` |
|---|---|---|
| `dgettext("gnome-shell","Activities")` | `Activities` | **活动** |
| `dgettext("nautilus","Files")` | `Files` | **文件** |
| `dgettext("gtk40","Cancel")` | `Cancel` | `Cancel` |
| `locale` | `C` | `zh_CN.UTF-8` |

`gtk40` 那条两种情况下都是英文：`gtk40.mo` 里本来就没有这个 msgid（这个域只覆盖一部分字符串），
不是查找失败。同一张 `zh_CN` 目录里 `gnome-shell.mo` 的 `Cancel` 是「取消」、`nautilus.mo` 的
`Open` 是「打开」。

**真实应用的端到端**：`gnome-calculator --help`

```
LANG=zh_CN.UTF-8 → 用法： / 帮助选项： / 显示帮助选项 / 显示全部帮助选项
LANG=C          → Usage: / Help Options: / Show help options / Show all help options
```

同一个二进制、同一棵树，只是环境变量不同。这说明应用自己的域（不是我们伪造的字符串）确实走了
系统语言包。

## 4 · 安装器

`install-husk.sh` 在树包旁边看到 `husk-lang-zh.tar.gz` 就解到 `$DEST`，然后数
`usr/share/locale-langpack/zh_CN/LC_MESSAGES/` 里的文件；少于 400 个直接 `exit 74`
（"do not ship this tree"）。没有这个文件就跳过并打印一行说明，界面回到英文，其它不受影响。

`make dist` 把 `tools/langpack/husk-lang-zh.tar.gz` 打进树包，并断言包里有 `gnome-shell.mo`——
少了就构建失败。

## 5 · 还没做

- 只做 `zh_CN`。`zh_TW` / `zh_HK`（繁體）没打。
- 翻译版本跟 `24.04+20260905` 绑死；换 Ubuntu 版本要重下（步骤写在
  `tools/langpack/README.md`）。
- 界面语言由 `LANG` / `LANGUAGE` 决定，会话里已经是 `zh_CN.UTF-8`（evidence 210）；
  如果用户自己设成别的语言，语言包还在，只是不会被用到。
