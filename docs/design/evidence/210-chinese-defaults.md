# 210 · 中文默认值：locale、字体族、输入法

日期：2026-11-25 19:35–19:45（CST）。设备 RMX3357，`:0`，0.0.8 的树加本节改动。

## 1 · 三个现场事实（FACT）

1. **LANG 是空的。** `locale` 输出全部 `POSIX`，`LANG=`、`LC_ALL=` 都为空。树里其实编译了
   `zh_CN.utf8`（`locale -a` 有）。
2. **字体族落错。** `fc-match "sans-serif:lang=zh"` → `DejaVu Sans`（没有中文字形）→ 逐字回落到
   `Noto Sans CJK JP`。换成 `lang=zh-cn` 才是 `Noto Sans CJK SC`。同一批汉字会有日文字形——
   「直」「骨」「次」这类字最明显。四个 Noto CJK（Sans/Serif × Regular/Bold）都在树里
   （`/usr/share/fonts/opentype/noto/`）。
3. **输入法装了但没人起。** `fcitx5`、`fcitx5-configtool`、gtk3/gtk4 immodule、拼音 addon 都在
   `/usr/bin` 与 `/usr/lib` 里；`/root/.config/fcitx5/profile` 不存在，也没有任何进程拉起它。

## 2 · 改了什么

| 文件 | 改动 |
|---|---|
| `tools/guest-config/husk-locale.sh` | 新增。`/etc/profile.d` 里补 `LANG=zh_CN.UTF-8`，规则只有一条：程序自己设了 LANG 就不动，空着、`POSIX`、`C`、`C.UTF-8` 才补 |
| `tools/guest/install-husk.sh` | 落 `/etc/profile.d/husk-locale.sh`；树里有 `zh_CN.utf8` 时另写 `/etc/default/locale`；只在没有时写 `root/.config/fcitx5/profile` |
| `tools/guest/husk-desktop.sh` | 会话环境注入 `LANG`/`LANGUAGE` 与 `GTK_IM_MODULE=fcitx`、`QT_IM_MODULE=fcitx`、`XMODIFIERS=@im=fcitx`；新增第 5c 步拉起 `fcitx5 -d --replace`，成功则打 `CHECK im=fcitx5` |
| `tools/guest-config/fcitx5-profile` | 新增。一组「键盘 + 拼音」，`DefaultIM=pinyin` |
| `Makefile` | `dist` 带上这两个新文件，并加 `tar tzf | grep -qx` 断言 |

为什么放 `/etc/profile.d` 而不是只写 `/etc/default/locale`：后者只对走 PAM 的登录 shell 生效，
run-as 起的进程、桌面 fork 出来的程序、`env -i` 都读不到——和 GPU 默认值是同一类问题（evidence 182）。

## 3 · 实测（FACT）

| 项 | 值 |
|---|---|
| `locale`（会话内） | `LANG=zh_CN.UTF-8`、`LANGUAGE=zh_CN:zh` |
| `nl_langinfo` | `locale=zh_CN.UTF-8 MON_1=一月 D_FMT=%Y年%m月%d日` |
| gnome-shell 进程 environ | `LANG=zh_CN.UTF-8`、`LANGUAGE=zh_CN:zh`（pid 10199） |
| 字体族 | `fc-match "sans-serif:lang=zh-cn"` → `NotoSansCJK-Regular.ttc: "Noto Sans CJK SC"` |
| 会话自检 | `CHECK wm=yes / ding=yes / gpu=zink / input=primed / im=fcitx5` |
| 输入法切换（有焦点窗口） | `ctrl+space` 前 `fcitx5-remote` = `0`，之后 = `2`；`fcitx5-remote -n` = `pinyin` |
| 端到端提交 | `gnome-text-editor /root/t.txt`，键入 `nihao` + 空格 → 文件字节 `e4 bd a0 e5 a5 bd`（`你好`），`ctrl+s` 落盘 |

## 4 · 一个会误导人的判据

**没有焦点窗口时，`fcitx5-remote -o` 与 `fcitx5-remote -n` 都返回空、状态一直是 0**（FACT）。
我一开始据此判断「输入法没起来」，是错的：`SetCurrentIM` 的 DBus 调用返回成功、`pinyin` addon
也确实加载了，只是没有输入上下文可切。加上 `xdotool windowactivate` 给一个焦点窗口，同一条命令
就变成 `0 → 2`、`-n` 输出 `pinyin`。

所以这条链路的判据只能是一次完整往返：**有焦点窗口 + 状态变化 + 提交的字符从应用里读回来**。

## 5 · 还没做的

- **界面翻译包不全。** `/usr/share/locale/zh_CN/LC_MESSAGES/` 只有 19 个 `.mo`
  （`cheese`、`dpkg`、`fcitx5`、`org.gnome.Weather` 等），没有 `gtk40`、`gnome-shell`、`mutter`、
  `nautilus`。所以 `LANG=zh_CN.UTF-8` 只改到日期格式、字体族、排序，**界面文字仍然是英文**。
  补语言包约 30 MB，且要重打 1.3 GB 的 rootfs，0.0.9 不做，写进 `docs/LIMITATIONS.md`。
- 输入法只在 **GTK4**（gnome-text-editor）上验过。XIM 前端、Qt 应用、候选窗的键盘导航（翻页、
  数字选词）都没测。
- 输入法第一次启动会打 `Failed to load pinyin history: io fail`（用户词库目录为空时的自愈日志），
  不影响输入；要不要预置空词库未定。

## 6 · 复现

```sh
sh $F/imtest.sh    # 起编辑器 → 给焦点 → ctrl+space → type nihao → 空格 → ctrl+s → od 文件
```

## 7 · 全新目录安装（FACT）

用 0.0.9 的树包在**空目录**装一遍（`install-husk.sh --rootfs <0.0.4 时代的 1 297 284 273 字节那份
rootfs> --dest fresh09/root`），`rc=0`，安装器自己那步
`husk -- usr/bin/bash -l -c 'id; head -1 /etc/os-release'` 也过了。三处新写入都在：

```
   wrote etc/profile.d/husk-gpu.sh (GPU defaults, escape: HUSK_GPU=llvmpipe)
   wrote etc/profile.d/husk-locale.sh + etc/default/locale (zh_CN.UTF-8)
   wrote root/.config/fcitx5/profile (keyboard + pinyin)
```

核对落盘内容：`etc/profile.d/husk-locale.sh` 724 B（与树包里那份逐字节相同）、
`etc/default/locale` = `LANG=zh_CN.UTF-8` / `LANGUAGE=zh_CN:zh`、
`root/.config/fcitx5/profile` 533 B（`DefaultIM=pinyin`，`keyboard-us` + `pinyin` 两项）、
`etc/profile.d/husk-gpu.sh` 3418 B 照旧写入——0.0.5 的那条没有被新加的步骤挤掉。

耗时可参考：这台机上这棵树两次解包（pass 1 直接解、pass 2 经 path-shim 重解）合计约 20 分钟，
不是卡住。
