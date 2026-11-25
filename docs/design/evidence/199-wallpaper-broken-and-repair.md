# 199 · 桌面背景黑了 / 老是变样：`picture-uri` 是空串，而且"拟合"这一步从来没装上过

日期：2026-11-25 16:08–16:14（CST）。设备 RMX3357，`:0`，`sess-plain.sh` 会话
（发布版加载器 + `/etc/husk/gpu-env` zink）。这一份回答用户提过两次的那个现象：
"桌面背景黑了" / "桌面壁纸老是变样"。

## 1 · 现场值（FACT）

会话刚起来、没做任何操作时的 `org.gnome.desktop.background`：

```
picture-uri      = ''
picture-uri-dark = ''
primary-color    = '#00ff00'
picture-options  = 'zoom'
~/.cache/husk/   = 空（没有预缩放出来的图）
/usr/share/backgrounds/ = 一堆 3840x2160 的原图在
```

shell 日志里还有一条更早的残留形态：

```
(gnome-shell:21440): libmutter-WARNING: Failed to load background 'file:///root':
                     Error opening file /root: Is a directory
```

⇒ 三种坏法都出现过：空 URI、指向目录、指向不存在的图。三种在屏幕上都是"没有壁纸"，
底色由 `primary-color` 决定，而它当时正好被留成了 `#00ff00`（荧光绿）。

## 2 · 为什么修不好：两个独立的洞（FACT）

1. **看护脚本从没被装进 rootfs。** `husk-session.sh` 只在
   `$BASE/husk-wallpaper-fit.sh` 存在且可执行时调用它：

   ```sh
   [ -x "$BASE/husk-wallpaper-fit.sh" ] && { "$BASE/husk-wallpaper-fit.sh" ... & }
   ```

   而 `install-husk.sh` 只装了 `husk-wallpaper-scale.js`，**没有装 `husk-wallpaper-fit.sh`**。
   实测设备上 `usr/local/share/linuxhub/` 里只有 `husk-wallpaper-scale.js`。所以"把壁纸缩到
   屏幕尺寸"这件事在真实安装里一次都没发生过。
2. **脚本本身也不会修。** 即使装上了，遇到空 URI 它直接退出：

   ```sh
   case "${URI:-}" in
       file://*) ;;
       *) exit 0 ;;          # 空串、纯色、任何非 file:// 都不管
   esac
   ```

   于是"壁纸坏了"这个状态没有任何自动恢复路径，只能靠人手改 `gsettings`。

## 3 · 改法（都在运行时层脚本里，不动应用、不动合成器）

`tools/guest/husk-wallpaper-fit.sh`：

- 空 URI、非 `file://`、或 `file://` 指向的文件不存在，一律判为"壁纸坏了"，兜底到
  随发行版装进来的第一张：`usr/share/backgrounds/husk-default.png` →
  `Clouds_by_Tibor_Mokanszki.jpg` → 任意 `*.png` → 任意 `*.jpg`；
- 兜底时同时把 `primary-color` 复位成 `#023c88`（避免再出现荧光绿底）；
- 顺手修掉 `[ -s OUT ] && [ OUT -nt SRC ] 2>/dev/null` 里 `2>/dev/null` 只作用在后半句的问题。

`tools/guest/install-husk.sh`：装上 `husk-wallpaper-fit.sh` 并 `chmod 755`，缺文件时打警告。

## 4 · 验收（FACT，逐像素）

修之前：`picture-uri=''`、底色 `#00ff00`，屏幕上没有壁纸。
跑一次修好的脚本：

```
husk-wallpaper-fit: 壁纸缺失或失效，兜底 -> /usr/share/backgrounds/Clouds_by_Tibor_Mokanszki.jpg
husk-wallpaper-fit: picture-uri -> /root/.cache/husk/wallpaper-2400x1080.png (2400x1080)
picture-uri      = 'file:///root/.cache/husk/wallpaper-2400x1080.png'
primary-color    = '#023c88'
```

`xcap` 取整屏 2400x1080，六个采样点（修完 / 整会话重启后再取一次）：

| 点 | 第一次 | 重启后 |
|---|---|---|
| 100,100 | 159,172,183 | 159,172,183 |
| 1200,100 | 250,250,250 | 250,250,250 |
| 2300,100 | 142,154,170 | 142,154,170 |
| 100,1000 | 21,73,90 | 21,73,90 |
| 1200,540 | 62,111,135 | 62,111,135 |
| 2300,1000 | 10,32,53 | 10,32,53 |

整屏均值 74.5，`min 0 / max 255` 都在（不是全黑、也不是纯色底）。**六个点跨重启逐字节一致**
⇒ 壁纸既不再丢，也不会每次开桌面都变样；输出图恰好 2400x1080，走 1:1 采样，
天然绕开 evidence 195 的右侧楔形。

## 5 · 还没覆盖的

- 装完的默认图是 `Clouds_by_Tibor_Mokanszki.jpg`（rootfs 里现成的一张）。要换成 Husk 自己的
  默认壁纸，把 `husk-default.png` 放进 `usr/share/backgrounds/` 即可，脚本优先取它。
- DING（桌面图标扩展）自己也会画一次壁纸；它拿的是同一个 gsettings 值，故不需要单独改。
