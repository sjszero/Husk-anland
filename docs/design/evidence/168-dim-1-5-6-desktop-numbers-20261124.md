# 168 · 八维补测：第 1（启动）/ 5（桌面与图形）/ 6（交互延迟）维

日期：2026-11-24 22:20–23:15（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页），包 `com.linuxhub.myth.husk`，X server 是 App 进程里的 Lorie（`:0`）。
rootfs Ubuntu 24.04 一份，两个运行器只换运行器。

> 所有数字都是**同一台机器、同一份 rootfs、同一个 X server、同一套环境变量**下量的。
> 计时源：guest 的 `date +%s%N`（ms 级），轮询间隔 50 ms（首帧）、5 ms（按键）。
> 每项 3–5 次，报中位数与全部样本；对 Husk 不利的数字照写。

## 1 · 第 1 维：启动

| 项 | 值 | 说明 |
|---|---|---|
| 会话冷启动到桌面可用 | **8 / 8 / 9 s** | 杀掉落 `gnome-shell` 后从发起到日志出现 `startup-complete`（stage 可见 + 顶栏/面板可见）。计到整秒，±1 s。dbus 已是常驻，不含它 |
| 桌面首帧（同一次会话） | 2400×1080，**非黑像素 2 592 000 / 2 592 000 = 100.0%** | `gl-probe root` 抓 X 根窗口，PNG 见 `168-desktop-gpu-x11.png` |

> 与 0.0.2 时期（软件渲染）不可比：那时桌面是 llvmpipe，现在这条会话是 zink → PanVK。
> proot 侧**没有**对应的"桌面冷启动"数字：2.0 的 proot 桌面不在本仓库的测试链里，
> 本机也没有第二个可比的 X server。这一格标**不可比**，不拿别的数字顶替。

## 2 · 第 5 维：桌面与图形（首帧 + 帧率）

### 2.1 首个"真正的大窗口"出现的时间

匹配规则：`xwininfo -root -tree` 里标题/类匹配目标，且**宽 ≥ 400、高 ≥ 300**
（GTK 应用会先建一个 1×1 的 leader 窗口，只按类名匹配会测出几十毫秒的假数）。

| 应用 | Husk（ms） | 中位 | proot（ms） | 中位 | Husk 相对 proot |
|---|---|---:|---|---:|---:|
| 设置（`gnome-control-center`） | 4446 / 4462 / 4432 / 4813 / 4787 | **4462** | 5289 / 5417（另 2 次 >90 s 超时，未计入） | **5353** | 1.2× 快 |
| 终端（`xterm`） | 233 / 110 / 100 | **110** | 141 / 261 / 153 | **153** | 1.4× 快 |

- proot 的"设置"有 2 次 90 秒超时：单实例应用在 D-Bus 上被"激活"而不是新开进程，
  残留一个实例就测不出首帧；这两次**没算**进中位数，但如实记在这里。
- 文件管理器（Nautilus）**测不了**：它拒绝以 root 运行（`This app cannot work correctly if
  run as root`），而本会话的 guest 身份是 root。改到 uid 1000 再测需要另一套会话总线，未做 → **未测**。
- gnome-terminal 起不来（`org.gnome.Terminal exited with status 8`，缺服务端），故用 xterm。

### 2.2 帧率（640×480 clear + swap，60 帧，`gl-probe window`）

| 渲染路径 | 帧率（fps） | 中位 |
|---|---|---:|
| zink → PanVK → Mali（GPU） | 118.4 / 130.9 / 134.0 | **131** |
| llvmpipe（CPU，`HUSK_GPU=llvmpipe`） | 209.3 / 389.5 / 300.1 | **300** |

**这个负载上 CPU 更快，照写。** 原因不是 GPU 慢，是这个负载只有 `glClear` + swap：
真正的瓶颈是呈现路径（DRI3 未通，走 SHM 拷贝），zink 还多一层 Vulkan 翻译。
所以：

- 这一格说明的是**呈现链路**的天花板，不是 GPU 算力；
- 能证明"桌面由 GPU 出帧"的是渲染器字符串与逐帧出图（§1），不是这个 fps；
- **重负载（着色器/纹理密集）的 GPU vs CPU 对比：未测（UNKNOWN）**，要做就得给 `gl-probe`
  加一个逐帧绘制 + 计时的子命令。

## 3 · 第 6 维：交互延迟（按键到回显）

方法：`xterm` 里跑 `cat > /tmp/keylog`；`xdotool type` 把 `a` 送进 tty（规范模式下 cat 还看不到），
把计时起点压在 **Return** 之前，轮询 `/tmp/keylog` 变大（5 ms 粒度）。

| 样本 | ms |
|---|---:|
| 1 | 53 |
| 2 | 47 |
| 3 | 41 |
| 4 | 40 |
| 5 | 36 |
| **中位** | **41** |

`xdotool` 自身启动开销（`xdotool getactivewindow`，5 次）：17 / 19 / 19 / 23 / 22 ms，中位 **19 ms**。
扣除它之后，**"回车 → guest 进程读到并落盘"约 22 ms**。

- 这里量的是"注入 → 应用读到"，**不是**"到屏幕像素"；后者要录屏逐帧对齐，未做 → 那一半 **未测**。
- proot 侧的同口径数字**未测**：proot 下 xterm 首帧能测（上面 153 ms），但按键链路要另搭，
  本轮没做。这一格标**未测**，不拿 Husk 的数当对比。

## 4 · 复现

```sh
# 首帧（husk / proot 同一脚本，只换第一个参数）
sh bench-win2.sh husk  gnome-control-center /usr/bin/gnome-control-center
sh bench-win2.sh proot gnome-control-center /usr/bin/gnome-control-center
# 帧率（GPU / CPU）
sh xe-drmfd.sh usr/bin/bash -lc 'export DISPLAY=:0; /tmp/gl-probe window'
sh xe-drmfd.sh usr/bin/env HUSK_GPU=llvmpipe usr/bin/bash -lc 'export DISPLAY=:0; /tmp/gl-probe window'
# 按键到回显
sh xe-drmfd.sh usr/bin/sh /tmp/keylat.sh
# 会话冷启动
sh coldstart.sh
```
