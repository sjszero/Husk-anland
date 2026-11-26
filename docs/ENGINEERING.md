# 工程设计

面向需要改动、复现或审计这份代码的读者。产品层面的介绍见 [README](../README.md)，
机制说明见 [ARCHITECTURE](ARCHITECTURE.md)。

## 1. 组件划分

| 组件 | 位置 | 作用 |
|---|---|---|
| 运行期路径层 | `runtime/crates/path-shim`、`runtime/crates/runtime-core` | `no_std` Rust，只解析路径；构建期嵌进动态链接器 |
| 构建期改写 | `tools/loader/`、`runtime/crates/nr-tools` | 入口桩、调用点 `nop`、把路径层嵌进 loader |
| 产品壳 | `app/` | Android 应用：会话、PTY、终端渲染 |
| 设备测试 | `tools/device-tests/` | `run-as` 下跑功能矩阵、越界测试、gcc/make 验收 |

## 2. 语言构成

产品代码只有 Rust，加 318 行汇编。仓库工具与探针也全部是 Rust crate：`husk-tools`（链接门禁、
基准表格）、`husk-window-wait`（首个可见窗口计时）、`gl-probe`（ES3 真 workload / 窗口化 EGL）、
`ipc-probe`（POSIX mqueue、SysV shm/sem/msg）。

仓库里不再有 Java、C、Python 实现：设备测试原先要装一个只有 96 行 Activity 的探针 APK，
用来把 `libld.so` 拉起来，现在这一步直接由 `run-as` 完成（guest 就是 App 同一个 uid 下的进程），
`tools/device-tests/husk-exec.sh` 就是那个启动器。

| 语言 | 规模 | 负责什么 | 为什么不是别的语言 |
|---|---|---|---|
| Rust | 68,279 行 / 73 个文件 | 运行时本体、全部构建与仓库工具、客舱探针、壳应用（PTY、GL 渲染、字体、输入、生命周期） | 单一实现，`no_std` 路径层零依赖，无 GC |
| 汇编 | 318 行 / 2 个文件 | loader 入口桩与 `_dl_init` 钩子 | 这段代码必须在任何 Rust 运行时存在之前执行；稳定版 Rust 无法定义 naked function |
| Shell | 2,906 行 / 55 个脚本 | 安装、会话启动、设备测试编排 | 被测对象本身就是 shell 与 coreutils，换掉等于换掉被测对象 |
| Gradle kts | 30 行 / 3 个文件 | APK 打包 | Android 官方打包工具，不承载产品逻辑 |

壳应用没有 Activity 的 Java 类：APK 里声明的就是 `android.app.NativeActivity`，
平台直接调用我们 `.so` 里的 `ANativeActivity_onCreate`，从这里往下全是 Rust。

## 3. 三处改动与字节账

| 改动 | 工具 | 结果 |
|---|---|---|
| 追加一个可读可执行的 `PT_LOAD`，容纳 shim | `nr-tools embed` | 203,968 B → 673,280 B（+469,312） |
| `_dl_init` 挂接钩子（176 B，无重定位） | `init-hook.S` + `nr-tools embed` | 位于同一 `PT_LOAD` |
| 两处 `svc` → `nop` | `nr-tools scan-syscalls patch` | 字节级替换，无位移 |

嵌入段布局（2026-11-24 实测的 `embed` 输出）：文件偏移 `0x40000`、虚拟地址 `0x90000`；
shim 跨度 `0x60000`；钩子地址 983,040；服务表地址 987,136；介入符号 167，外部符号 50。

`scan-syscalls` 不携带偏移表：它读取镜像的 `.gnu.version_d` 校验 C 运行库版本，
按指令模式定位调用点，并断言后继指令只写返回值寄存器、不读取它。
版本不符、数量不符、调用点缺失、后继指令读取返回值，四种情况均拒绝写出产物。

## 4. 符号介入的范围

`nr-tools embed` 报告的介入符号数是 167（`interposed`），其中约 45 个是 POSIX 入口；其余来自以下三类：

- C 运行库的内部别名（`__open64_nocancel`、`__gen_tempname` 内部调用等），不经过程序链接表；
- 标准 I/O 入口（`fopen`、`creat`）与路径规范化（`realpath`、`canonicalize_file_name`）；
- 强制源码审计（`_FORTIFY_SOURCE=3`）生成的 `__*_chk` 与 64 位孪生函数。

每一项的触发用例记录在 `design/evidence/16`。

## 5. 构建工具

三个工具，均为 Rust、零外部依赖：

| 工具 | 作用 | 校验方式 |
|---|---|---|
| `nr-tools scan-syscalls` | 定位并改写启动期系统调用点 | 产物与回归测试的 sha256 一致 |
| `nr-tools embed` | 生成成品动态链接器 | 产物与回归测试的 sha256 一致 |
| `nr-tools rootfs` | 在主机上生成 guest rootfs（包集 → 目录 → 打补丁的 libc → tar），并可用同一镜像的 `ld.so` 产出配套 loader | 产物可结构校验：253 个 `usr/bin`、usr-merge 链接、`dpkg` 状态库 61 项、libc 两处 `nop` |
| `nr-tools provision` | 供应 gcc / make 工具链 | 阶段目录逐文件比对，2,866 项内容差异为零 |

三个工具都是唯一实现。迁移期间的 Python / sh 参考实现在逐字节对拍通过后已从工作树删除
（`git show 665ec31^:tools/loader/nr-embed.py` 可取回），构建路径上不存在第二种实现。

## 6. 产物指纹

| 文件 | 是什么 | sha256 |
|---|---|---|
| `tools/loader/ld-noble2404-ld.so` | 输入：Ubuntu 24.04 原始 loader，**入库** | `124e3f166bdc0be365eaa6dc59e415b876b0841a40463edd1b0e084b518bb7cd` |
| `ld-embedded.so` / `jniLibs/libld.so` | 产出：成品动态链接器 | `69d591b5baef4ba69a257bf59d0b53b75f1f00aee6ce5c8b705e69cb33ff951e` |
| `libc-nop.so.6` | 产出：guest 用（HUSK-SLOT-1 槽位页 + clone3→ENOSYS），输入固定为 `libc6 2.39-0ubuntu8.12`，需要 `LIBC=` 才生成 | `16d5ba2120c1a92c0c9cba62dd8ec244d29d5d30a733d4ac530be4fb1b92933f` |
| `libpath_shim.so` | 产出：嵌入前的路径层 | `a1ecb420c0f7784c9750f5c413f1796cfcc402fcf6ccb612911a5dee50f109b0` |

四行里只有 `ld-noble2404-ld.so` 是入库文件；另外三个都是本地构建产物（`.gitignore` 挡着）。
`ld-embedded.so` 的哈希是 `path-shim` 及其嵌入的 `drm-shim` 源码的函数：改一行路径层或
DRM 模拟代码它就会变，所以这一行
只对当前主干有意义，发布审计以 tag 里 `dist/SHA256SUMS` 的记录为准。clean tree 构建
`libc-nop.so.6` 时必须用固定的 glibc 2.39 stock 输入，结果应与上表哈希一致。

## 7. 环境

| 项 | 版本 |
|---|---|
| 设备 | RMX3357（MT6893 / Mali-G77），Android 13，内核 4.19.191，页大小 4096 |
| 安全域 | `untrusted_app`（uid 10512 / u0_a302），无 root、无 BL 解锁、无虚拟化 |
| Guest | Ubuntu 24.04，glibc 2.39 |
| 工具链 | Rust stable（`aarch64-unknown-linux-gnu`、`aarch64-linux-android`）、NDK r27+、JDK 17+ |

## 8. 维护约定

- 未实测不写通过；证据保存在 `design/evidence/`，编号被方案与文档引用。
- 每条结论标注为事实、推断或未知。
- 构建工具的任何改动必须与仓库里的 sha256 基线一致，否则视为回归。
