# Husk 发布目标（v54 · 2026-11-24）

> 唯一目标口径。v53 确立了「转向可发布」，v54 把重心放回应有的位置：
> **真机上的全方位对比**才是这个项目存在的理由——不是宣传语，是同一台手机、同一份 rootfs、
> 只换运行器跑出来的数字，以及数字背后的机制解释。

## 0 · 一句话

在同一台 Android 手机上，用同一份 Ubuntu 24.04 rootfs，把 **Husk / proot 容器 /
proot-distro** 三个运行器放在一起跑完整套基准，用实测差距说明「零 ptrace 的原生用户态」
与「syscall 翻译层」的本质不同；差距讲清楚之后，按公开发布标准整理仓库。

## 1 · 不变量

- 不 root、不解 Bootloader、不开硬件虚拟化、零 ptrace；shim 只 interpose libc 符号。
- 能 Rust 就 Rust；第三方代码不入库；绝不读写 `com.linuxhub.myth`。
- 结论只认实测：FACT / INFERENCE / UNKNOWN 分开标注，未测不写 PASS，**对 Husk 不利的数字照写**。
- 每次正式改动提交并推送 GitHub。
- 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64），
  运行域 `untrusted_app`。单机结论不外推成平台结论。

## 2 · 交付顺序（A 是前置，B 是核心，C 是包装）

### A · 先把已知问题修净（不修完，基准数据不可信）

| # | 项 | 完成判据 |
|---|---|---|
| A1 | polkit「调用者可信」层 | `pkcheck --action-id org.freedesktop.login1.power-off --process <uid 1000 进程>` 通过；桌面电源菜单能弹授权框 |
| A2 | at-spi 无障碍总线 | GTK 客户端能连 a11y 总线，`AT_SPI_BUS_ADDRESS` 可用 |
| A3 | gvfs 点对点总线回退 | 不再回退，日志无该告警 |
| A4 | xrandr scaling | `Impossible to set scaling` 消失，缩放可设 |
| A5 | 触控默认模式 | 真机「点哪落哪」；模式 2 / 3 给出实测结论 |
| A6 | `nr-sigsys.log` 噪声 | 单次会话不再产生 MB 级重复行 |

统一判据：`tools/device-tests/14-gate-a-acceptance.sh`（36 项）全绿；桌面连续 30 分钟
零僵尸、零崩溃；收尾后设备回睡眠态。

### B · 真机全方位对比（本项目最重要的产出）

#### B1 · 被测对象与共同条件

三个运行器，**同机、同内核、同一份 rootfs 内容（逐文件校验和一致）、同一起始 cwd、
同一 shell**，只换运行器：

1. **Husk**（本仓库 loader + path-shim）
2. **LinuxHub 2.0 的 proot 容器**
3. **proot-distro 式 proot**（Termux 生态常见形态）

记录并公开：机型、SoC、内核、页大小、Android 版本、三者的精确版本号与启动命令行、
设备温度与电量区间。基准前统一 `sync`，基准期间禁止充电、关闭省电模式、屏幕常亮，
每轮之间冷却 60 秒。

#### B2 · 八个维度，每维度都要有数字

1. **启动**：冷启动到 shell 提示符；冷启动到桌面首帧；应用网格打开；`apt update` 冷/热。
2. **进程与 syscall 密度**：`/bin/true` × 1000（spawn 到 exit 固定开销）；`fork+exec` 1000 次；
   `getpid` / `open+close` / `stat` 三种纯 syscall 循环；`make -j4` 并行编译。
   —— 这一维是核心：proot 的税按 syscall 密度收，Husk 不按。
3. **文件 I/O**：`find /usr`；源码树 `tar` 解包；`grep -r`；1 GiB 顺序写/读；
   10 000 个小文件创建+删除；`git clone` 本地仓库 + `git status`。
4. **包管理与开发工作流**：`dpkg -l`；`apt install` 一个中等包（下载+解包+配置分别计时）；
   `gcc` 编译 hello（`-H` 记录头文件搜索次数）；`make -j4` 编译 zstd；
   `python3 -c 'import json'` 与一次 numpy 导入。
5. **桌面与图形**：打开「设置」到首帧；文件管理器到首帧；任意 5 个 GTK 应用到可见；
   窗口拖动与滚动帧率（录屏逐帧）；终端里 `top` 的刷新节奏；1080p 视频播放丢帧数。
   —— Husk 与 2.0 都是软件渲染时，必须写明「此维比较的是运行器开销，不是 GPU」。
6. **交互延迟**：触摸到界面响应；键盘按下到字符回显；点击图标到应用出现（录屏对齐帧）。
7. **资源与稳定**：空载 30 分钟的 RSS / 线程数 / fd 数增长；僵尸进程数；静止 5 分钟平均 CPU；
   同一批任务 10 分钟的耗电百分比；温度曲线。
8. **兼容性覆盖**：固定软件清单（bash、coreutils、git、python3、gcc、vim、node 或等价、
   文件管理器、终端、浏览器或明示不可用）逐项「可用 / 不可用 + 阻塞原因」。

#### B3 · 方法（不做到这些，数字不认）

- 每项 **≥5 次，报中位数 + 极差**；冷/热分列；不挑好的一次。
- 脚本全部入库 `tools/bench/`，一条命令重跑整套：`tools/bench/run-all.sh <runner>`。
- 原始输出入库 `docs/design/evidence/`，表格入库 `docs/BENCHMARKS.md`。
- 图形项用录屏逐帧对齐，说明对齐方法与误差（±1 帧）。
- 任何一项 proot 更快，原样写出来，并解释原因。

#### B4 · 结论必须解释机制，不只是排名

`docs/BENCHMARKS.md` 结尾固定三段：

1. **总表**：八维度 × 三运行器，倍率一栏写「Husk 相对 proot」。
2. **为什么**：逐维说明成因（零 ptrace；路径翻译只在 loader 里做一次 GOT 改写，
   运行期无 syscall 拦截；fork/exec 不经翻译层；文件 I/O 直落内核）。
3. **代价**：Husk 目前不如容器的地方（生态成熟度、发行版覆盖、GPU 仍软件渲染、
   某些 SELinux 受限设备访问），照实写。

### C · 仓库成为正式介绍

- `README.md`：是什么、比 proot 快多少（引用 BENCHMARKS 总表）、怎么用、边界在哪；首屏 30 秒可读。
- `docs/BENCHMARKS.md`（核心）、`docs/RUNNING.md`、`docs/COMPATIBILITY.md`、`docs/LIMITATIONS.md`。
- 「不打包 App 也能用」：交付 `husk` 独立分发（arm64 启动器，无 JVM/Python/Termux 依赖
  + rootfs 包 + 校验清单），一条命令起步 `./husk run ubuntu`；
  验收：测试机不安装 APK，仅用 `adb shell` 或普通终端起 `bash`、跑 `apt`、起图形桌面
  （图形阻塞则写明原因）。
- 语气：产品说明口吻，禁止「我 / 我们 / 昨天 / 足迹 / 里程碑日记」；只写已实测事实与
  可复现步骤，不对未来做承诺。

## 3 · 品牌

- 图标 `husk-mark-hexagon-h`（橙→红渐变开口六边形 + 中央 H），`docs/assets/brand/`，
  命名 `husk-mark-hexagon-h{,-512,-1024}.{jpg,png}`。
- 配色只用渐变橙红与中性灰黑；文档不放大图、不用 emoji 装饰。

## 4 · 发布门禁（缺一不发）

- [ ] 门禁 A 36 项全绿 + 桌面 30 分钟长稳
- [ ] A1–A6 闭环且有实测记录
- [ ] `tools/bench/run-all.sh` 三运行器跑完，八个维度无空缺
- [ ] `docs/BENCHMARKS.md` 有总表、机制解释、代价三段，数字可复现
- [ ] `docs/RUNNING.md` 在干净环境（不装 APK）走通
- [ ] README / ARCHITECTURE / COMPATIBILITY / LIMITATIONS / SECURITY / CONTRIBUTING /
      CHANGELOG / LICENSE 就位
- [ ] 仓库无第三方代码、无密钥、无真实用户数据、无内部路径
- [ ] 每条结论可指回 `docs/design/evidence/`

## 5 · 不做（Non-goals）

- 跨机型适配；root / 解 BL / 虚拟化方案。
- Termux 作为交付形态；传感器 / NFC / USB 二次暴露。
- 用软件渲染、软解、假设备、静态单帧冒充硬件路径。
- 用开发日记、求助记录、流水账充作文档；用拔高措辞代替数字。

## 6 · 完成判定

门禁全绿打 `v1.0.0`。判定只有一条：一个没听过这个项目的工程师，看完 README 首屏 +
`docs/BENCHMARKS.md` 总表，能准确说出「Husk 是什么、在本机比 proot 快多少、为什么快、
在哪儿还不如它、我该怎么用」。
