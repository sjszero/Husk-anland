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
| 探针应用 | `tools/probe-app/` | 最小验证载体，不面向用户 |

## 2. 语言构成

产品代码只有 Rust，加 271 行汇编。没有 Java，没有 Python，没有 C（`bench.c` 是测试用的
参考实现，运行时与构建路径都不经过它）。

| 语言 | 规模 | 负责什么 | 为什么不是别的语言 |
|---|---|---|---|
| Rust | 5,681 行 / 12 个文件 | 运行时本体、全部构建工具、壳应用（PTY、GL 渲染、字体、输入、生命周期） | 单一实现，`no_std` 路径层零依赖，无 GC |
| 汇编 | 271 行 / 2 个文件 | loader 入口桩与 `_dl_init` 钩子 | 这段代码必须在任何 Rust 运行时存在之前执行；稳定版 Rust 无法定义 naked function |
| Shell | 474 行 / 13 个脚本 | 设备测试 | 被测对象本身就是 shell 与 coreutils，换掉等于换掉被测对象 |
| C | 103 行 / 1 个文件 | 客舱基准的参考实现 | 只在没有推预编译 Rust 基准时兜底；Rust 版在 `runtime/crates/guest-bench` |
| Gradle kts | 46 行 / 6 个文件 | APK 打包 | Android 官方打包工具，不承载产品逻辑 |

壳应用没有 Activity 的 Java 类：APK 里声明的就是 `android.app.NativeActivity`，
平台直接调用我们 `.so` 里的 `ANativeActivity_onCreate`，从这里往下全是 Rust。

## 3. 三处改动与字节账

| 改动 | 工具 | 结果 |
|---|---|---|
| 追加一个可读可执行的 `PT_LOAD`，容纳 shim | `nr-tools embed` | 203,968 B → 290,816 B（+86,848） |
| `_dl_init` 挂接钩子（176 B，无重定位） | `init-hook.S` + `nr-tools embed` | 位于同一 `PT_LOAD` |
| 两处 `svc` → `nop` | `nr-tools scan-syscalls patch` | 字节级替换，无位移 |

嵌入段布局：文件偏移 `0x32000`、虚拟地址 `0x42000`；钩子地址 348,160；服务表地址 352,256。

`scan-syscalls` 不携带偏移表：它读取镜像的 `.gnu.version_d` 校验 C 运行库版本，
按指令模式定位调用点，并断言后继指令只写返回值寄存器、不读取它。
版本不符、数量不符、调用点缺失、后继指令读取返回值，四种情况均拒绝写出产物。

## 4. 符号介入的范围

共介入 76 个符号。其中约 45 个是 POSIX 入口；其余来自以下三类：

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
（`git show v0.1.0:tools/loader/nr-embed.py` 可取回），构建路径上不存在第二种实现。

## 6. 产物指纹

| 文件 | sha256 |
|---|---|
| `ld-embedded.so`（成品动态链接器） | `7b4300a4966edcc27fdf3528392cc7228b1615c01b9fa4d31acfdd7b6124f91a` |
| `ld-linux-aarch64.so.1.orig`（原始） | `bd29514c1b45cc29b152011dd7c504e00dce240c19e4dd120196053873af9180` |
| `libc-nop.so.6`（guest 使用，含 HUSK-SLOT-1 槽位页 + clone3→ENOSYS） | `32d4d3f9501a2c717cf2246a2df1a898f1361b49e0576614461d70fc94774b56` |
| `libpath_shim.so`（嵌入前） | `1b86173bea3ba33e8a86caf6232d9e508faed429a5ea0d90fb960f3a6262ed75` |

在本仓库执行 `tools/loader/build-embedded-loader.sh`，产出应与第一行一致。

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
