# 缺陷账本（Defect ledger）

规则：**只追加，不覆盖、不重编号**。每条带：编号 · 一句话 · 证据（文件/命令）·
状态（OPEN / FIXED / OUT-OF-SCOPE / UNVERIFIED）· 门禁归属。

编号前缀按轮次与类别：

| 前缀 | 含义 |
|---|---|
| `H<n>` | 工程门禁类（第 2 轮）：门禁看起来有、实际看不见 |
| `S<n>` | fd / 路径语义类（第 3 轮） |
| `P<n>` | 普通用户安装路径类（第 4 轮） |
| `C<n>` | 呈现链类（第 5 轮） |
| `G<n>` | GPU 全局原生化类（第 6 轮） |
| `D<n>` | 硬件四类落地类（第 7 轮） |

> 未落库项：第 1 轮的四张清单（A 已确认缺陷 / B 源码级风险 / C 文档与证据冲突 /
> D 未实现·未验证·环境跳过）目前只存在于会话记录里，仓库中没有对应文件 —— 见 `H5`。

| 编号 | 缺陷 | 修前证据 | 状态 | 门禁归属 |
|---|---|---|---|---|
| H1 | 干净 clone 只跑 `./gradlew` 会成功打出 0 块 `.so` 的空壳 APK | `BUILD SUCCESSFUL`，随后 `unzip -l` 里 `lib/arm64-v8a/` 计数为 0（evidence 293 §1） | **FIXED** `app/app/build.gradle.kts` 的 `huskVerifyNativePieces` | CI `apk` 作业 + Gradle `preBuild` |
| H2 | `make check` 只查 39 个 crate 里的 12 个 | `audio-relay` 注入 E0308 后旧列表 rc=0（evidence 293 §2） | **FIXED** `Makefile` 改 `--workspace` | `make check` / CI `check` |
| H3 | 单测只有 3 个 crate 进 CI；`video-protocol` 零测试 | `grep -rl "#\[cfg(test)\]"` 只有 5 个 crate（evidence 293 §3） | **FIXED** 新增 5 个测试 + CI 扩到 6 个 crate | CI `test` 作业 |
| H4 | 发布命名契约只有联网半，离线规则从未进 CI | `audit-release-names.sh` 需要 `gh`，只在 `make release-audit` 里跑（evidence 293 §4） | **FIXED** 拆出 `tools/ci/audit-release-offline.sh` | CI `scripts` 作业 + `make check` |
| H5 | 第 1 轮的四张审计清单没有落库，无法"保持可复现" | `docs/` 下无对应文件；`ls docs/design/evidence` 里只有 290–293 是本轮写的 | **OPEN** | 第 3 轮开工前补 |
| H6 | `video-protocol` / `husk-va-driver` 的单测在本机（macOS）只能编译不能执行 | `cargo test -p husk-va-driver` → `cannot find function memfd_create in crate libc` | **FIXED**（绕开：交叉编成 `aarch64-unknown-linux-musl` 静态二进制，真机跑，7/7 通过；macOS 仍不放 `make check`） | CI `test` 作业 + 真机执行 |
| H7 | `video-protocol` 的 `commit()` 不校验 `write_seq`：空环上 `commit(1)` 成功并把 `read_seq` 推到 1，`write - read` 下溢成 `u64::MAX`，环永久"满"，生产者只拿到 `WouldBlock` | 新单测修前在 `unwrap_err()` 上 panic（拿到 `Ok(())`） | **FIXED** `runtime/crates/video-protocol/src/lib.rs` 的 `commit()` 先看 `read >= write` 返回 `WouldBlock` | `cargo test -p video-protocol` |
| H8 | libc 门禁的钉子钉错版本：下载 8.12、期望 `69ceef37`，而没有任何产物等于它 → 门禁从第一天起就是红的，且它自称断言"发布要发的字节" | 本地两次逐字节相同：8.5→`19554f27`（=树里那份）、8.12→`2ed8771f`；`69ceef37` 无对应产物 | **FIXED（钉子）** 改钉 8.5/`19554f27`，输入换 launchpad；**OPEN（升版）** 8.12 是更新的安全修订，要升必须重跑设备验收 | CI `libc` 作业 |
| H9 | CI 的 `apk` / `android` 两个作业一直红：用了 `ANDROID_NDK_LATEST_HOME`，而 ubuntu-latest 上没有这个变量，拼出 `/toolchains/llvm/prebuilt/linux-x86_64/bin/clang` | run 45824212545：`no clang at /toolchains/.../clang`、`linker ... not found` | **FIXED** 两个作业各自解析 NDK（先找 `$ANDROID_HOME/ndk/*`，没有再装） | CI `apk` / `android` 作业 |
| H10 | 第 2 轮开工前 `main` 上连续 6 个提交的 CI 全红（apk/android 的 NDK 路径、apk 漏 linux-gnu target、libc 钉子），红得没有信息量，等于没有网 | `5bc335c`…`13231bd` 六个 run 全 failure | **FIXED** 三项各修，run `45825616837` 11/11 绿 | CI 全域 |
| S1 | 假 fd 身份有三份实现，规则各不相同：`drm-preload` 用子串 `contains("mali")`、`v4l2-shim` 用前缀 + 数字前缀、`path-shim` 用 `s[..17]=="/memfd:husk-v4l2-"`。于是 `/tmp/malicious-data` 能进 DRM 模拟，`husk-drm` 会被 v4l2 前缀吞掉，`/memfd:husk-v4l2-99999` 被当成越界下标 | 三处源码 `runtime/crates/{drm-preload,v4l2-shim,path-shim}/src/lib.rs`；evidence 294 §2 | **FIXED** 新增 `runtime/crates/fd-identity`（no_std，精确语法 + `(deleted)` 后缀，5 单测），三处改为调它 | `cargo test -p husk-fd-identity` + 真机 `run-fd-identity-probe.sh` |
| S2 | `path-shim` 对"无状态假 DRM fd"回 `EBADF(9)`：fd 本身是我们自己的假节点、完全有效，回 EBADF 等于告诉调用方"描述符无效"，把排障引到错树上（跨进程 `SCM_RIGHTS` 才是真原因） | 真机修前 `registered_fd=6 rc=-1 errno=38`；改后 `errno=95`（evidence 294 §2） | **FIXED** 回 `EOPNOTSUPP(95)`，并注明"能力不支持"≠"描述符无效" | `run-fd-identity-probe.sh` |
| S3 | 设备上存在三份 `libld.so`（APK lib 目录 / `files/linux/libld.so` / `files/linux/husk/libld.so`），探针默认读第三份。新构建只写进第二份 → 连续几轮"改了没变化"，实际跑的是 19:04 的旧件 | sha256：APK lib = `658f97e1…`、`files/linux/libld.so` = `b96b7860…`（21:11）、`files/linux/husk/libld.so` = `658f97e1…`（19:04）；把新件写进第三格后同一条用例立刻变绿（evidence 294 §1） | **FIXED** 探针启动前比对设备 `libld.so` 与 `tools/loader/ld-embedded.so` 的 sha256，不一致拒绝运行（正向 `matches local build`、负向 `rc=1`，evidence 294 §1） | `run-fd-identity-probe.sh` |
| S4 | `fd-identity-probe` 的 v4l2 名字语法用例用"errno≠ENOTTY 即被 shim 认领"当判据；本机内核在**任何** memfd 上的 ioctl 都回 `EACCES(13)`（SELinux ioctl xperm），判据恒真 → 三条用例长期假阳性 | 对照表见 evidence 294 §3：`plain` / `husk-v4l2-0` / `husk-v4l2-0x` 全是 `EACCES`，只有 `husk-drm` + DRM 请求是 `EOPNOTSUPP` | **FIXED** 判据改为"与同进程中性名 memfd 完全同答案" | `run-fd-identity-probe.sh` |
| S5 | `pathshim-drm-fake-fd-identity-lost` 用 `open("/dev/dri/renderD128")` + `open("/proc/self/fd/N")` 复现"身份丢失"；shim 对第二条路径会**重新建立身份**，所以用例测到的是 DRM 模拟层（`ENOSYS`），不是它自称的条件。本机根本没有 `renderD128`，用例却一直有输出 | 改前输出 `registered_fd=6 … errno=38 | orphan_fd=7 … errno=38`；改后 `memfd_fd=3 … 95 | dup_fd=4 … 95`（evidence 294 §2） | **FIXED** 改为原生 `memfd:husk-drm` + `dup`，即 `SCM_RIGHTS` 接收方的最小同形复现 | `run-fd-identity-probe.sh` |
| S6 | 同一条件两层答案不一致：无 DRM preload 时 `path-shim` 回 `EOPNOTSUPP(95)`；preload 层在场时它的 `CTX.dispatch` 对未模拟请求回 `ENOSYS(38)`。两者都"显式拒绝"，但码不同，客户端对 `ENOSYS` 有专门的 fallback 语义 | 新增用例 `pathshim-drm-fake-fd-identity-lost-preload` → `rc=-1 errno=38`（evidence 294 §0、§2） | **OPEN**（已量化，未统一）：统一要看真实 GPU 客户端对 38/95 的实际反应，本轮无证据，不擅自改语义 | `run-fd-identity-probe.sh` |
