# Husk 发布目标（v53 · 2026-11-24）

> 本文件是仓库的唯一目标口径。上一版（v52，全硬件后端清单）已完成的部分保留在
> `docs/RESULTS.md` 与 `docs/design/evidence/`，本阶段不再以「硬件清单」为主线，
> 而以**可发布**为主线：把已知问题修净、把仓库写成一份正式介绍、让第三方不装 App 也能跑。

## 0 · 一句话

让 Husk 成为一台普通 Android 手机上**可以直接用的 Linux 运行环境**，并用同一台机器、
同一份 rootfs 上的**可复现实测**说明它与 proot 类容器差在哪里——然后按公开发布的标准
整理仓库。

## 1 · 不变量（任何阶段都成立）

- 不 root、不解 Bootloader、不开硬件虚拟化、零 ptrace；shim 只 interpose libc 符号，
  不扩大系统调用拦截面。
- 能 Rust 就 Rust；第三方代码不入库；绝不读写 `com.linuxhub.myth` 的任何数据。
- 结论只认实测：FACT / INFERENCE / UNKNOWN 分开标注，未测不写 PASS。
- 每次正式改动提交并推送 GitHub。
- 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64），
  运行域 `untrusted_app`。单机失败不外推成平台结论。

## 2 · 本阶段交付三件事

### A · 收尾：把所有已知基础问题修净（发布阻塞项）

| # | 项 | 完成判据 |
|---|---|---|
| A1 | polkit「调用者可信」层 | `pkcheck --action-id org.freedesktop.login1.power-off --process <uid 1000 进程>` 通过；桌面「电源」菜单可弹授权框 |
| A2 | at-spi 无障碍总线 | GTK 客户端能连 a11y 总线，`AT_SPI_BUS_ADDRESS` 可用 |
| A3 | gvfs 点对点总线回退 | 不再回退，日志无该告警 |
| A4 | xrandr scaling | `Impossible to set scaling` 消失，显示器缩放可设 |
| A5 | 触控默认模式 | 默认模式在真机上「点哪落哪」，2/3 两种模式给出实测结论 |
| A6 | `nr-sigsys.log` 噪声 | 一次会话不再产生 MB 级重复行 |

统一判据：`tools/device-tests/14-gate-a-acceptance.sh`（36 项）全绿；桌面连续 30 分钟
零僵尸、零崩溃、屏幕常亮可控，收尾后设备回到睡眠态。

### B · 仓库是一份正式介绍，不是开发日记

- `README.md`：它是什么、能做什么、比容器快多少、怎么用、边界在哪。首屏 30 秒可读。
- `docs/BENCHMARKS.md`：新增，见 §3 的对比口径与结果。
- `docs/RUNNING.md`：新增，见 §2C，不装 App 的使用方式。
- `docs/COMPATIBILITY.md`：能跑什么（glibc 版本、桌面栈、常见工具），不能跑什么。
- `docs/LIMITATIONS.md`：软渲染、GPU、输入语义等已知边界，按实情写，不美化。
- 语气与用词规范：产品说明口吻，不写「我 / 我们 / 昨天 / 足迹 / 里程碑日记」；
  只描述已实测事实与可复现步骤；不对未来做承诺。
- 每张结果表都必须带：被测对象、命令、次数、统计量、证据文件链接。

### C · 不打包 App 也能用（第三方可用性）

交付 `husk` 独立分发形态，让任何 arm64 Android 设备的使用者不安装 LinuxHub APK 也能得到
同一套 Linux 环境：

- `husk` 启动器（arm64，无 JVM、无 Python、无 Termux 依赖）+ rootfs 包 + 校验和清单。
- 一条命令起步：`./husk run ubuntu`（首次运行解包、校验、落盘，二次运行直接进）。
- 验收：在测试机上**卸载/不安装 APK**，仅用 `adb shell` 或普通终端完成：起 `bash`、
  跑 `apt`、起图形桌面（或明确记录图形部分的阻塞原因）。

## 3 · 对比口径：Husk vs 两个容器

被测对象（同机、同 rootfs、同内核、只换运行器）：

1. **Husk**（本仓库 loader）；
2. **LinuxHub 2.0 proot 容器**（proot 运行时）；
3. **proot-distro 式 proot**（Termux 生态常见形态）。

指标与命令（每项 ≥5 次，取中位数，冷/热分列，记录方差）：

| 指标 | 命令/方法 | 单位 |
|---|---|---|
| 冷启动到 shell 提示符 | `time <runner> /bin/sh -c exit` | s |
| 打开「设置」到首帧 | 桌面场景，录屏逐帧对齐 | s |
| fork+exec 吞吐 | 1000 次 `execve` 循环 | s / 次均 |
| 目录遍历 | `find /usr -xdev \| wc -l` | s |
| 归档解包 | `tar -xf linux.tar -C /tmp` | s |
| 包管理 | `dpkg -l` | s |
| 纯 CPU 基线 | 定长整数循环（与 syscall 密度无关） | s |
| 常驻内存 / 线程数 | `smem` / `/proc/<pid>/status` | MiB / 个 |
| 交互延迟 | 触摸到界面响应（录屏对齐） | ms |

产出：`docs/BENCHMARKS.md` 三列表格 + `tools/bench/` 可复现脚本 + 证据文件。
结论表述只允许「在 RMX3357 上，以中位数计，Husk 相对 proot 的倍率为 X」。

## 4 · 品牌

- 图标：**`husk-mark-hexagon-h`**（橙→红渐变的开口六边形碗 + 中央 H）。
  文件命名 `husk-mark-hexagon-h{,-512,-1024}.{jpg,png}`，存放 `docs/assets/brand/`。
- 使用：README 首屏用 512 px；文档内不放大图；不使用 emoji 装饰。
- 配色：只允许渐变橙红（#F26522 → #E01F26 区间）与中性灰黑，不引入第三色。

## 5 · 发布门禁（缺一不发）

- [ ] 门禁 A 36 项全绿 + 桌面 30 分钟长稳
- [ ] A1–A6 全部闭环并有实测记录
- [ ] `docs/BENCHMARKS.md` 三运行器数字齐全且可复现
- [ ] `docs/RUNNING.md` 在干净环境（不装 APK）按文档走通
- [ ] `README / ARCHITECTURE / COMPATIBILITY / LIMITATIONS / SECURITY / CONTRIBUTING / CHANGELOG / LICENSE` 就位
- [ ] 仓库无第三方代码、无密钥、无真实用户数据、无内部主机路径与私有仓库名
- [ ] 每条结论可指回 `docs/design/evidence/`

## 6 · 不做（Non-goals）

- 跨机型适配；root / 解 BL / 虚拟化方案。
- Termux 作为交付形态；传感器 / NFC / USB 设备向 guest 的二次暴露（机主自身可穿透）。
- 用软件渲染、软解、假设备或静态单帧冒充硬件路径。
- 以开发日记、求助记录、内部试验流水账充作文档。

## 7 · 完成判定

门禁全绿后打 tag `v1.0.0`。判定标准只有一条：一个没听过这个项目的工程师，
在 README 首屏 + `docs/BENCHMARKS.md` 之后，能准确说出「Husk 是什么、它比 proot 快多少、
在我这台设备上怎么用」。
