# 355 · 桌面冷启动同机对照：Husk 14.7 s vs PRoot 9.8 s（1.5×）

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
同一判据：点“打开桌面” → `dumpsys SurfaceFlinger --list` 出现该 App 的 `SurfaceView[…]`
（单次查询 24 ms，不抓屏、不抢设备；废弃了早期“每 0.5 s 抓 10 MB 原始帧”的做法）

## 0 · 结果

| 宿主 | 样本 | tap → 首帧 | 中位数 |
|---|---|---:|---:|
| Husk（`com.linuxhub.myth.husk`） | 5 | 14647 / 14730 / 15370 / 14523 / 14901 ms | **14730 ms** |
| PRoot（`com.linuxhub.myth`） | 3 | 10366 / 9736 / 9785 ms | **9785 ms** |

**Husk 比同机 PRoot 冷启动慢 1.51×。** 用户此前“同一个手机慢 1~2 倍”的观察在这一指标上成立。

## 1 · 这 14.7 s 的构成（evidence 353/354）

```text
点按钮
 +0.0s   宿主起 X server
 +4.8s   gnome-shell 进程创建
 +4.9s~  utime/stime 增量为 0（wchan=do_sys_poll）≈4.1 s 纯等待
 +9.0s   满核运行 ≈3.4 s
+14.7s   首帧
```

其中 4.2 s 与 `glxinfo -B` 的固定开销（4275 ms）量级一致：
**每开一个走 GL 的进程都要付 4.2 s（zink→PanVK 初始化），换 llvmpipe 只要 0.36 s。**

## 2 · 能确定的和不能确定的

FACT：
- Husk 冷启动 14.7 s，PRoot 9.8 s，比值 1.51。
- 文件路径密集负载 Husk 明显更快（evidence 349：stat 2.83×、open+close 5.85×）。
- GL 进程初始化固定 4.2 s，与 PRoot 无关地存在于本机 zink 路径上。

UNVERIFIED：
- PRoot 的 9.8 s 里有多少是同一项 GL 初始化。本轮没有在 PRoot 里跑 `glxinfo -B`，
  所以不能把 14.7−4.2 与 9.8−4.2 直接相减来归因剩下的 5 s。
- 剩下 5 s 的归属（loader 逐库翻译、会话服务串行、mutter 内部阶段）尚无逐项证据。

## 3 · 待做的优化与预期

| 项 | 机制 | 预期 |
|---|---|---|
| 会话保活（退后台不杀会话） | 冷启动只在第一次付 | 第二次起降到秒级 |
| 清 `$XDG_RUNTIME_DIR` 遗留 socket（H42，已积 146 个） | gvfsd 扫描目录变小 | 未知，需实测 |
| 砍非必需会话服务（evolution 三件套 / tracker / gvfs mtp-afc-gphoto2-goa） | 少拉起 8–10 个进程 | 未知，需实测 |
| PanVK exec-BO 回退 | 4.2 s 的主要嫌疑 | 需 root/kernel 侧确认 |

## 4 · 结论口径

发布说明里只能写：
“同机冷启动：PRoot 9.8 s / Husk 14.7 s（1.51×）；文件路径负载 Husk 快 2.8–5.9×；
GL 进程初始化 4.2 s 为已知边界。”
不能写“Husk 整体比 PRoot 快”。
