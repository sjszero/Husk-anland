# 参与贡献

Husk 把一段 glibc 用户态跑在普通 Android 应用进程里：不 root、不解 Bootloader、不开硬件
虚拟化、不做 `ptrace`。任何补丁都得保持这个形状——项目存在的理由就是它换来的那点性能。

## 术语表

下面五个词在全仓库里含义固定。如果某份文档和这张表不一致，那是文档错了。

| 词 | 含义 |
|---|---|
| **loader** | `libld.so`——guest 的 glibc `ld.so`，把 shim 嵌在一个额外的 `PT_LOAD` 里。它装载 guest 程序，并在每次 `execve` 时重新进入自己 |
| **path shim** | loader 里那段 Rust（`no_std`）代码：interpose 带路径的 libc 符号（`open`、`stat`、`execve`、`statvfs` 等），把它们解析到 guest 根下 |
| **guest** | loader 起来的进程树。它看到 rootfs 就是 `/`，uid 0，看不到 Android 的路径 |
| **rootfs** | 发行版目录树（设备上是 `files/linux/distributions/<发行版>/rootfs`）。基准里三个运行器共用同一份，运行器才是唯一变量 |
| **runner** | 启动 guest 的东西：`husk`（loader + shim）、`proot`、`proot-distro`。`tools/bench/` 把它们当成可互换的适配器 |

## 硬规则

1. **树里不放第三方代码。** 依赖由 Cargo 在构建时解析；必须随包发布的产物（loader、补丁版
   libc、guest 工具）都由本仓库的源码构建，只在 `docs/ENGINEERING.md` 写明的地方入库。
2. **不 root、不 `ptrace`、不用 namespace。** 需要这些的补丁是另一个项目，先开 issue 讨论。
3. **结论必须有测量。** 说「更快」的 PR 要在 `docs/design/evidence/` 留一条命令与输出，
   包括 Husk 输了的那几次。
4. **guest 代码用绝对路径，命令行上给相对路径。** loader 的调用形如
   `libld.so --library-path ... usr/bin/bash`，程序路径故意是相对的（见 `docs/RUNNING.md` §5）。
5. **绝不读写 `com.linuxhub.myth`。** 被测应用只走文档里写的 adb / `run-as` 路径。

## 构建

```sh
# 只做类型检查（对它实际运行的目标架构）
make check

# 格式门禁（CI 跑的那条）
make lint

# 三处版本号一致性
make version-check

# 重建 loader —— 需要 NDK（clang + llvm-objcopy）通过 $NDK 提供
make loader
```

`cargo test` 会编译 aarch64 内联汇编，只能在 arm64 Linux 主机上跑。在 macOS 或 x86_64 上
编译不过，那是平台限制，不是补丁坏了——那边用 `make check`。

## 风格

- Conventional Commits：`feat|fix|docs|perf|refactor|test|build|ci|chore`。正文写测到了什么，
  不写有多辛苦。
- Rust：`cargo fmt` 与 clippy 对**错误**必须安静；`path-shim` 里遗留的 warning 单独跟踪。
- Shell：POSIX `sh`、`set -eu`，`tools/` 里不写 bash 专有语法，用 `sh -n` 验。
- 文档：标题只用 `#`/`##`/`###`，正文 ≤100 列，代码块带语言标记。

## 哪些测试算数

| 层 | 怎么查 |
|---|---|
| shim / ELF 改写 | 在 arm64 Linux 主机上 `cargo test --target aarch64-unknown-linux-gnu -p runtime-core -p nr-tools` |
| guest 基础 | `tools/device-tests/14-gate-a-acceptance.sh`（36 项：bash、locale、用户管理、dpkg、apt、宿主路径泄漏、seccomp 陷阱） |
| 稳定性 | 空载 30 分钟：RSS、线程数、fd 数、僵尸数都不涨（`docs/design/evidence/153-*`） |
| 性能 | `tools/bench/run-all.sh <runner>`——同一份 rootfs、同一个 shell，只换运行器 |

## 发布

版本号只有一个来源：仓库根的 `VERSION`，内容是**三段式 SemVer**（`X.Y.Z`），每个分量只能是
`0..99`。版本在四处出现，数字必须完全相同，但格式各有固定写法：

| 位置 | 固定格式 |
|---|---|
| `VERSION`、`husk --version`、正文 | `X.Y.Z` |
| annotated git tag | `vX.Y.Z` |
| GitHub Release 标题 | `Husk X.Y.Z` |
| `CHANGELOG.md` 小节 | `[X.Y.Z] - YYYY-MM-DD` |

`v` 只属于 git tag。Release 标题不能写成 `Husk vX.Y.Z`，也不能把主题拼在后面；
主题只放正文。每个 annotated tag 的说明首行也必须严格写成 `Husk X.Y.Z`，不能让 tag 页显示
另一套标题。tag 的提交目标和发布资产不可移动；历史上只含标题的主题文字可以统一为规范标题。

发布序号按 **100 进位**，所以不存在 `0.0.100`：

```text
0.0.98 -> 0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

每次发版先运行 `make version-bump`，不要手改数字。`./husk --version`、APK `versionName`、
`CHANGELOG.md` 最新一节、release tag 和徽章由 `make version-check` 强制对齐。

**已经发出去的 tag 不改名，也不移动提交目标**——否则下载同一个链接的人会拿到不同的东西。
只有 tag 的 annotation 显示文字可以统一为 `Husk X.Y.Z`，且必须保留原 commit、Release 和
附件不变。只有刚发布当天就发现版本号本身写错，才允许修正版本号，并在 CHANGELOG 留一行说明。
发版流程（命令从 `VERSION` 读取版本，不手写当前版本）：

```sh
# 1. 版本与变更
make version-bump
version=$(cat VERSION)
# 在 CHANGELOG.md 顶部加一节 ## [$version] - YYYY-MM-DD，并把链接改成
# [$version]: https://github.com/xishanp/Husk/releases/tag/v$version
make check                     # 含 version-check
LIBC=<pristine glibc 2.39 libc.so.6> NDK=<ndk>/prebuilt make dist
# 产出 dist/husk-arm64.tar.gz + SHA256SUMS

# 2. 打标签、推、建 release（annotated tag，不要轻量标签）
git tag -a "v$version" -m "Husk $version"
git push origin main && git push origin "v$version"
notes=$(mktemp)
sh tools/ci/release-notes.sh > "$notes"
gh release create "v$version" dist/husk-arm64.tar.gz dist/SHA256SUMS \
  --title "Husk $version" --notes-file "$notes"
rm "$notes"

# 3. 回读发布页，检查 tag、Release 标题和正文首行
make release-audit
```

GitHub Release 只承载版本页：标题只写 `Husk X.Y.Z`，不拼主题或项目介绍。正文第一行固定为
`## 概要`，下面只写这一版自己的结论；不得复制 README 的项目介绍，也不得使用 `> 主题：**…**`
当页头。后续章节统一用二级标题，例如 `## 修复`、`## 验证`、`## 边界`、`## 产物`，不再用
`###`。正文必须写：本版修了什么（带 evidence 编号）、已知问题、产物 sha256。
**没在设备上验过的东西，不写进 release notes。**

`make check` 还会检查仓库首屏结构与 Release 正文生成器：首屏只能有 `Husk` 标题、六边形图标和
徽章，项目介绍必须从 `## 概览` 开始；Release 正文必须以 `## 概要` 开始，且不能混入项目总介绍
或三级章节。`make release-audit` 会同时检查全部远端 Release 的正文契约，以及 tag 是否按 100
进位连续递增，不能跳过版本。文档里用 `git show vX.Y.Z:<path>` 指向历史文件时，对应 tag 必须
真实存在；被删除的 tag 只能改用不可变的提交号引用。

## 报告 bug

用 issue 模板。附上它要的设备属性、完整命令、完整输出。如果运行时打了 `SIGSYS`，说清是
哪个系统调用号——[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) 写了哪些是预期内的。
