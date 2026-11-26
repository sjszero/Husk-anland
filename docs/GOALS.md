# Husk 发布目标（v55 · 2026-11-24）

> 唯一目标口径。v54 把「真机全方位对比」放回中心；v55 补上最后一环：**仓库的呈现形式**。
> 大厂公开仓库的样子是可以逐项复制的：目录、文件清单、README 结构、徽章、排版、版本与提交
> 规范、CI。写不出这个「味」，是因为缺规范，不是因为内容不够。

## 0 · 一句话

在同一台 Android 手机上，用同一份 Ubuntu 24.04 rootfs，把 **Husk / proot 容器 /
proot-distro** 放在一起跑完八维度基准，用实测差距说明「零 ptrace 的原生用户态」与
「syscall 翻译层」的本质不同；然后按大厂公开仓库的格式把这一切呈现出来。

## 1 · 不变量

- 不 root、不解 Bootloader、不开硬件虚拟化、零 ptrace；shim 只 interpose libc 符号。
- 能 Rust 就 Rust；第三方代码不入库；绝不读写 `com.linuxhub.myth`。
- 结论只认实测：FACT / INFERENCE / UNKNOWN 分开标注，未测不写 PASS，**实测数据全部照实记录**。
- 每次正式改动提交并推送 GitHub。
- 唯一目标机：RMX3357（MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64），
  运行域 `untrusted_app`。单机结论不外推成平台结论。

## 2 · 交付顺序

A 前置（修问题）→ B 核心（真机全方位对比）→ C 呈现（仓库格式 + 不装 App 也能用）。

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

### B · 真机全方位对比（内容核心）

#### B1 · 被测对象与共同条件

三个运行器，**同机、同内核、同一份 rootfs 内容（逐文件校验和一致）、同一起始 cwd、
同一 shell**，只换运行器：Husk / LinuxHub 2.0 的 proot 容器 / proot-distro 式 proot。

公开：机型、SoC、内核、页大小、Android 版本、三者精确版本与启动命令行、温度与电量区间。
基准前 `sync`，期间不充电、关省电、屏幕常亮，每轮冷却 60 秒。

#### B2 · 八个维度（每维度必须有数字）

1. **启动**：冷启动到 shell；冷启动到桌面首帧；应用网格打开；`apt update` 冷/热。
2. **进程与 syscall 密度**：`/bin/true` × 1000；`fork+exec` 1000 次；`getpid` /
   `open+close` / `stat` 三种纯 syscall 循环；`make -j4`。核心维度：proot 的税按 syscall
   密度收，Husk 不按。
3. **文件 I/O**：`find /usr`；源码树 `tar` 解包；`grep -r`；1 GiB 顺序写/读；1 万小文件
   创建+删除；`git clone` 本地仓库 + `git status`。
4. **包管理与开发工作流**：`dpkg -l`；`apt install` 中等包（下载/解包/配置分别计时）；
   `gcc` 编译 hello（`-H` 记录头文件搜索）；`make -j4` 编译 zstd；`python3` 导入 numpy。
5. **桌面与图形**：打开「设置」到首帧；文件管理器到首帧；任意 5 个 GTK 应用到可见；
   窗口拖动与滚动帧率（录屏逐帧）；终端 `top` 刷新节奏；1080p 播放丢帧数。
   两者都是软件渲染时必须写明「此维比较运行器开销，不是 GPU」。
6. **交互延迟**：触摸到响应；按键到回显；点击图标到应用出现（录屏对齐帧，±1 帧）。
7. **资源与稳定**：空载 30 分钟 RSS / 线程 / fd 增长；僵尸数；静止 5 分钟平均 CPU；
   同批任务 10 分钟耗电；温度曲线。
8. **兼容性覆盖**：固定清单（bash、coreutils、git、python3、gcc、vim、node 或等价、
   文件管理器、终端、浏览器或明示不可用）逐项「可用 / 不可用 + 阻塞原因」。

#### B3 · 方法

每项 ≥5 次报中位数 + 极差，冷/热分列；脚本入库 `tools/bench/`，一条命令重跑
`tools/bench/run-all.sh <runner>`；原始输出入 `docs/design/evidence/`，表格入
`docs/BENCHMARKS.md`；任何一项 proot 更快，原样写出并解释原因。

#### B4 · 结论必须解释机制

`docs/BENCHMARKS.md` 结尾固定三段：**总表**（八维度 × 三运行器，"Husk 相对 proot" 倍率）、
**为什么**（零 ptrace；路径翻译只在 loader 做一次 GOT 改写，运行期无 syscall 拦截；
fork/exec 不经翻译层；I/O 直落内核）、**代价**（生态成熟度、发行版覆盖、GPU 仍软件渲染、
受限设备访问）。

### C · 仓库格式：按大厂公开仓库的样子做

#### C1 · 必须存在的文件（缺一个都不算正式仓库）

| 文件 | 要求 |
|---|---|
| `README.md` | **中文为主**（产品主文档），结构见 C2；首屏 30 秒可读 |
| `README_EN.md` | 英文镜像，结构与数字与中文版一致；两版顶部互相链接 |
| `LICENSE` | Apache-2.0（已落地，2026-11-24）。第三方 glibc 二进制只剩一个构建输入（`tools/loader/ld-noble2404-ld.so`，LGPL-2.1+，声明见 `NOTICE`）；其余 `binaries/` 构件已从仓库移出，改为 `make loader` 本地生成 |
| `NOTICE` | 第三方组件、商标、字体与图标来源声明 |
| `CHANGELOG.md` | Keep a Changelog 1.1 格式 + SemVer 2.0；每个 release 一节 |
| `CONTRIBUTING.md` | 起环境、跑门禁、提交规范、PR 要求、代码风格 |
| `CODE_OF_CONDUCT.md` | Contributor Covenant 2.1 原文 + 联系方式 |
| `SECURITY.md` | 支持版本、漏洞披露渠道、响应时限、不接受的范围 |
| `SUPPORT.md` | 去哪问问题（Discussions / Issues / 文档） |
| `ROADMAP.md` | 对外承诺的下一步（与 `docs/GOALS.md` 分开：GOALS 是口径，ROADMAP 是对外） |
| `ARCHITECTURE.md` | 架构图 + 模块职责 + 关键取舍，重排为规范结构 |
| `docs/BENCHMARKS.md` | B 的产出，见 C2 表格规范 |
| `docs/RUNNING.md` | 不装 App 的使用方式（见 C4） |
| `docs/COMPATIBILITY.md` | 能跑什么、不能跑什么、glibc / 桌面栈版本 |
| `docs/LIMITATIONS.md` | 诚实边界清单 |
| `.github/ISSUE_TEMPLATE/bug_report.yml` | 结构化表单：设备、系统、版本、复现、日志 |
| `.github/ISSUE_TEMPLATE/feature_request.yml` | 结构化表单 |
| `.github/ISSUE_TEMPLATE/config.yml` | 引导到 Discussions / 文档 |
| `.github/PULL_REQUEST_TEMPLATE.md` | 变更说明、影响面、验证方式、证据链接 |
| `.github/workflows/ci.yml` | `fmt` + `clippy -D warnings` + arm64 构建 + 单元测试 + 文档链接检查（不上设备） |
| `.github/CODEOWNERS` | 责任范围 |
| `.editorconfig` | 缩进、行尾、编码统一 |
| `rustfmt.toml` | 格式固定 |
| `Makefile` 或 `justfile` | 标准目标：`build` `test` `lint` `bench` `gate-a` `dist` `docs` |
| `SHA256SUMS`（随 release） | 发布产物校验清单 |

#### C2 · README 结构（顺序固定）

1. 居中 logo（`docs/assets/brand/husk-mark-hexagon-h-512.png`）+ 项目名 + 一句话定位 + 徽章行
2. 定位段：≤60 词，说清「是什么 / 跑在哪 / 凭什么不同」
3. 目录（Table of Contents，锚点链接）
4. **Why Husk**：3 行对比数字（链到 BENCHMARKS 锚点），不放形容词
5. Features：≤8 条 bullet，每条一句话，必要时括号给出证据编号
6. Quick Start：可直接复制的代码块（安装 → 进 shell → 进桌面）
7. How it works：一张架构图（SVG）+ 3–5 句机制说明 + 链 `ARCHITECTURE.md`
8. Performance：总表摘要 + 链 `docs/BENCHMARKS.md`
9. Compatibility：表格 + 链
10. Limitations：诚实清单（允许 `⚠️` 标记）
11. Build from source：前提、命令、产物、校验
12. Repository layout：`tree` 输出
13. Documentation：文档索引表（文件名 + 一句话说明）
14. FAQ：5–8 条真实问题
15. Contributing / Code of Conduct / Security：各一段 + 文件链接
16. License：一句 + 链接
17. Acknowledgements：致谢与第三方声明

#### C3 · 徽章（只允许真徽章）

首屏徽章行至少包含：**CI 状态**（指向 `.github/workflows/ci.yml`）、**最新 release**、
**License**、**平台**（Android arm64）、**Rust 工具链 / MSRV**、**PRs welcome**。
规则：端点必须真实存在（先建 workflow 再放徽章）；禁止静态假状态、禁止已失效的第三方 CI 徽章、
禁止"star 数""下载量"这类装饰性徽章。徽章之间用空格分隔，不换行堆叠。

#### C4 · 不打包 App 也能用（对外可用性）

交付 `husk` 独立分发：arm64 启动器（无 JVM / 无 Python / 无 Termux 依赖）+ rootfs 包 +
`SHA256SUMS`。一条命令起步：`./husk run ubuntu`。验收：测试机**不安装 APK**，仅用
`adb shell` 或普通终端起 `bash`、跑 `apt`、起图形桌面（图形阻塞则写明原因）。
分发产物随 GitHub Release 发布，体积与校验和写进 release notes。

#### C5 · 写作与排版规范

- 标题只用 `#` / `##` / `###`；正文行宽 ≤100 列；一个段落不超过 5 行。
- 所有代码块带语言标记；所有命令可直接复制（不带 `$` 提示符）。
- 表格：数字带单位；比较表必须有表注「被测对象 + 设备 + 时间 + 样本数」；
  百分比必须给基数（如 `3/5 应用`）。
- 每个性能数字后面给可点击的 evidence 或 BENCHMARKS 锚点链接。
- 术语表统一：loader / path-shim / guest / rootfs / runner 的使用方式写进 `CONTRIBUTING.md`。
- 禁用词：极致、革命性、颠覆、完美、零成本（除非有实测支撑）；
  禁用句式：「我们正在努力」「即将支持」。
- 双语：英文为主，中文镜像同结构、同数字；两版 `git diff` 结构应一致。

#### C6 · 版本与提交

- Conventional Commits：`feat|fix|docs|perf|refactor|test|build|ci|chore`，正文写事实与
  evidence 编号。
- 版本号形如 SemVer 2.0，但发布序号按 **100 进位**：`0.0.99` 的下一版是 `0.1.0`，
  `0.99.99` 的下一版是 `1.0.0`，不存在 `0.0.100`。用 `make version-bump` 推进，不要手改。
- annotated tag `vX.Y.Z`；`husk --version`、APK `versionName`、徽章与 CHANGELOG 保持一致。
- GitHub Release 标题只写 `Husk X.Y.Z`；主题和说明只放正文，不拼进标题。
- 每个 release：CHANGELOG 一节 + GitHub Release（产物 + SHA256SUMS + 已知问题）。

## 3 · 品牌

- 图标 `husk-mark-hexagon-h`（橙→红渐变开口六边形 + 中央 H），`docs/assets/brand/`，
  命名 `husk-mark-hexagon-h{,-512,-1024}.{jpg,png}`。
- 配色只用渐变橙红（#F26522 → #E01F26 区间）与中性灰黑；文档不放大图、不用 emoji 装饰。

## 4 · 发布门禁（缺一不发）

- [x] 门禁 A 36 项全绿 + 桌面 30 分钟长稳 —— 主树 36/36（23:16），从发布包新装的树
      34/34（少 2 项是包内无 apt 缓存、重装检查被跳过，不是失败）；长稳见 evidence 169
- [x] A1–A6 闭环且有实测记录 —— evidence 151/152
- [ ] `tools/bench/run-all.sh` 三运行器跑完，八维度无空缺 —— 八字维度是**分轮单独测**的
      （1/2/3/4/5/6/8 走 `bench`，7 走 soak，见 `docs/BENCHMARKS.md`）；一条命令跑完仍未做，
      表里「不可比/未测」的格子已在文档里逐格写明原因
- [x] `docs/BENCHMARKS.md` 有总表 / 机制 / 代价三段，数字可复现 —— evidence 168/169
- [x] `docs/RUNNING.md` 在干净环境（不装 APK）走通 —— evidence 130，产物重验 172
- [ ] C1 文件清单齐全；C2 README 十七节齐全；C3 徽章全部为真徽章 —— C3 还差 CI 徽章
      （它依赖下面那条 CI）
- [x] LICENSE 已换成 OSI 认可许可证，`NOTICE` 就位
- [ ] CI 在 GitHub 上真实跑绿一次（fmt / clippy / build / test / docs 链接）——
      **卡在 token 权限**：推送 `.github/workflows/ci.yml` 被拒
      （`without 'workflow' scope`）。六个作业的命令本机逐条跑过、全绿；解开只需
      `gh auth refresh -h github.com -s workflow` 后再推一次
- [x] 仓库无第三方代码、无密钥、无真实用户数据、无内部路径与私有仓库名 ——
      2026-11-24 扫过：无 `gh[po]_`/私钥、无 `/Users/...`（4 个构建脚本里写死的 NDK 路径已改成
      从 `ANDROID_NDK_HOME`/`ANDROID_HOME` 解析）、无 vendored 第三方源、只出现本项目自己的
      包名 `com.linuxhub.myth.husk`
- [x] 每条结论可指回 `docs/design/evidence/` —— 193 个 markdown 链接 0 断

## 5 · 不做（Non-goals）

- 跨机型适配；root / 解 BL / 虚拟化方案。
- Termux / 任何第三方终端作为交付或交互形态（guest 树依赖不全，在 Termux 里跑不起来，不承诺）；
  传感器 / NFC / USB 二次暴露。
- 用软件渲染、软解、假设备、静态单帧冒充硬件路径。
- 装饰性徽章、营销形容词、开发日记式文档。

## 6 · 完成判定

门禁全绿打 `v1.0.0`。判定只有一条：一个没听过这个项目的工程师，打开仓库 30 秒内
（首屏 logo + 一句话 + 徽章 + 对比表）能说出「Husk 是什么、在本机比 proot 快多少、为什么快、
在哪儿还不如它、我该怎么用」。
