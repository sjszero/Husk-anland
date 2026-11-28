# 359 · Overview 黑色三角：10 次复现未出现（定量）；adb 点击到不了桌面
> **口径更正**：本份测的缩略图是 **Super 键 Overview** 里的那一块，用户指的是
> **应用盘（左下角 Ubuntu 按钮）**里的工作区缩略图，两者不是同一个界面。
> 应用盘那块的实测见 evidence 361 / H49；本份结论只作 Overview 的对照保留。


日期：2026-11-27 21:38（CST）。设备 `UKHMMB8PYTGECMNV`，树
`files/linux/distributions/ubuntu-24.04/rootfs`（0.0.37 发行包装出，PanVK `f73600da…`）。

## 1 · 结论

- **NOT REPRODUCED**：按缺陷台账里的复现步骤（进桌面 → 开 Overview → 看工作区缩略图 →
  返回 → 循环 10 次），缩略图右上角**没有一个近黑像素**；
- 缩略图右上角与桌面同区域的亮度一致（均值 61.5 vs 58.4，最大值 100 vs 86），
  也就是说缩略图就是桌面的缩放，没有出现"黑色三角"；
- **FACT**：`adb shell input tap` 完全没有到达 GNOME 会话（同一坐标点两次、截图逐字节相同）；
  能从 guest 内驱动会话的是 `xdotool`（`DISPLAY=:0`）。

## 2 · 怎么驱动的

`input tap` 试过两次（`(60,1030)`、`(30,1040)`，都在左下角应用盘附近），截图与之前逐像素相同。
改用 guest 内的 `xdotool`：

```sh
# 在 guest 里
DISPLAY=:0 xdotool getdisplaygeometry   # 2400 1080
DISPLAY=:0 xdotool key super            # 打开 Overview
DISPLAY=:0 xdotool key Escape           # 关闭
DISPLAY=:0 xdotool key Right            # 切到第二个工作区
```

10 次开关循环，结束后再抓一张 2400×1080 的整屏。

## 3 · 定量：缩略图右上角没有黑像素

区域：`x 1700..2130, y 170..520`（Overview 里那个缩略图的右上角，黑色三角出现过的地方），
每 4 像素采样一次，共 9,504 个采样点：

| 图 | 近黑像素（max(R,G,B) < 16）占比 | 平均亮度 | 最大亮度 |
|---|---:|---:|---:|
| Overview 缩略图 | **0.0000** | 61.5 | 100 |
| 桌面同区域 | **0.0000** | 58.4 | 86 |

原图：`359-overview-10cycles-20261127.png`（Overview）、
`359-overview-desktop-20261127.png`（桌面）。

## 4 · 这不能算"修好了"

- 缺陷台账 H38 记的那次是 gnome-shell 因 MIT-SHM 的 `BadAccess` 以 133 退出、桌面根本不起来；
  本轮三次冷启都是正常起桌面，没复现。
- 这一轮也没有做那 10 组单变量对照（旧 ICD / llvmpipe / `COGL_DEBUG` / shader cache / 纯色壁纸 /
  关动画）。**没有对照组，就不能说"三角已修复"**，只能写"当前组合下未复现"。
- 触发条件仍然未知，H38 保持 OPEN。

## 5 · 顺带：输入这一层现在是外部阻塞

`adb input tap` 不到桌面这件事，不是"Husk 的坐标算错了"，而是整条注入链没有把事件送到宿主窗口
（Husk 侧只负责 guest 的 X 输入语义）。用 `xdotool` 能证明 **X 侧的键盘/指针路径是通的**
（Overview 打开、切换工作区、鼠标移动都生效），所以：
- 能验的：guest → X 的输入语义（本轮已验：Super/Escape/Right/鼠标移动）；
- 验不了的：Android 触摸 → 宿主窗口 → X 的那一段，属宿主责任（H48）。

## 6 · 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs LD=files/linux/husk/libld.so \
  sh tools/device-tests/run-guest.sh t-ov.sh
adb -s UKHMMB8PYTGECMNV exec-out screencap -p > /tmp/overview.png
```
