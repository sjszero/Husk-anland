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
| H6 | `video-protocol` / `husk-va-driver` 的单测在本机（macOS）只能编译不能执行 | `cargo test -p husk-va-driver` → `cannot find function memfd_create in crate libc` | **UNVERIFIED**（执行结果以 CI ubuntu runner 为准） | CI `test` 作业 |
