# 188 · Nautilus 46 内容区空白：这一轮三种配置都复现不出来，178 的结论要更正

日期：2026-11-25 12:47–12:49（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4 KiB 页）。承接 evidence 178（"内容区整块空白，连列头都不出现"）。

## 1 · 做法

同一条命令、同一个目录、同一个会话总线，只换两个变量，各起一次、各抓一张整屏：

```
nautilus --new-window /etc        # 走发布版加载器 libld.so（xe-plain.sh）
```

窗口几何三次都一样：`1012x672`，标题 `etc`。

| # | GSK_RENDERER | GL 后端 | 内容区 | 图 |
|---|---|---|---|---|
| 1 | `cairo` | zink → PanVK | **完整**：面包屑 `Ubuntu / etc`、侧栏 Recent/Starred/Home/Trash/Other Locations、4×N 图标网格（NetworkManager/UPower/X11/adduser.conf/alsa/alternatives/apg.conf/apparmor.d/apport/apt/bash.bashrc/bash_completion.d…） | `188-nautilus-cairo-zink.png` |
| 2 | `ngl` | zink → PanVK | **完整**，但侧栏出现 187 那种三角缺口 | `188-nautilus-ngl-zink.png` |
| 3 | `cairo` | llvmpipe | **完整** | `188-nautilus-cairo-llvmpipe.png` |

三种配置下 `gvfs` 都在刷同一条：

```
GVFS-WARNING: The peer-to-peer connection failed: Unexpected lack of content trying to
read a line. Falling back to the session bus. Your application is probably missing
--filesystem=xdg-run/gvfsd privileges.
```

多刷三条，不影响出画——**FACT**：p2p 回退不是空白的成因（至少不是唯一成因）。

## 2 · 结论

**FACT**：本轮三种组合都画出了内容区，178 记的"内容区整块空白、连列头都没有"没法复现。

**JUDGMENT**：178 那次大概率是**会话态**问题，而不是渲染路径问题。可疑的三点当时都成立：
① 用 `xdotool ... windowkill` 杀掉再拉起（GTK 的 surface/尺寸状态不是正常退出）；
② 旧会话跑的是 `llvmpipe + GSK_RENDERER=cairo` 的组合，且 X 屏刚被从 1200x540 抬到
2400x1080（窗口几何是旧的，见 186 §5）；③ 当时会话里的加载器是实验构建，注入不生效。

**UNKNOWN**：那一次到底断在哪一环，没有留下可复算的现场。要坐实只能按原步骤一比一重放，
本轮时间没花在这上面——因为 187 已经解释了"看起来缺一块"的主因，而 178 的场景现在**不是**
用户能稳定见到的故障。

## 3 · 随之要改的口径

- `docs/LIMITATIONS.md` §7 里那句"nautilus 内容区整块空白、连列头都不出现……根因 UNKNOWN"
  要降级为：**未复现**，并注明 178 的现场条件。
- ③ 这一条在 0.0.6 的验收里改成"按 178 原步骤重放一次，若仍空白再当 bug；否则按会话态
  问题记录"。**不能拿 178 当已知 bug 的宣传口径。**
