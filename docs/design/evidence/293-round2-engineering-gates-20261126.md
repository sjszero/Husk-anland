# 293 · 第 2 轮工程门禁：两条判据、四个缺陷、修前与修后

日期：2026-11-26（CST）。机器：macOS arm64 开发机 + 真机
RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页。
版本：`VERSION=0.0.36`，最新 tag `v0.0.35`（未发版，与 §3 一致）。

本轮只做门禁，不改运行时行为。四条缺陷全部是"门禁看起来有、实际看不见"这一类：
它们不报错、不失败，只是让 `make check` / CI 绿得没有意义。

## 0 · 结论

| 编号 | 缺陷 | 状态 | 判据 |
|---|---|---|---|
| H1 | 干净 clone 只跑 `./gradlew` 会**成功**打出一个 0 块 `.so` 的空壳 APK | 已修 | 修前 `BUILD SUCCESSFUL` + APK 内 0 个 `.so`；修后 `huskVerifyNativePieces FAILED` |
| H2 | `make check` 只查 39 个 crate 里的 12 个，类型错误看不见 | 已修 | 注入 E0308 到 `audio-relay`：旧列表 rc=0，`--workspace` rc=1 |
| H3 | 单测只有 3 个 crate 进 CI；`nr-tools` 的测试从未跑过，`video-protocol` 一个测试都没有 | 已修 | 本地 14 项全绿；CI 新增 `-p nr-tools -p video-protocol -p husk-va-driver` |
| H4 | 发布命名契约只有"要 gh 的联网半"，离线可验证的规则从没进过 CI | 已修 | 拆出 `tools/ci/audit-release-offline.sh`，进 CI + `make check`；3 个负向用例全红 |

- **FACT**：两条判据（白名单外任意 crate 塞类型错误 CI 必红；干净 clone 只跑 gradlew 必红）
  都在本轮实测到修前失败、修后通过，命令与输出见 §1、§2。
- **UNVERIFIED**：`video-protocol` 与 `husk-va-driver` 的单测在 macOS 上**链接不进去**
  （它们调 `libc::memfd_create`，macOS 的 libc crate 没有这个符号），所以本地只验证了
  "编译到 linux 目标"，执行交给 CI 的 ubuntu runner。本轮的 docker 本地复核失败：
  `apt-get install build-essential` 在容器里 6 分钟没装完，已弃用（§4）。
- **FACT**：`make check` 现在是 `rc=0`，14 个单测全绿（gpu-env 6 / nr-tools 1 / session-env 4 / usermgr 3）。

## 1 · H1：干净 clone 只跑 gradlew 打空壳 APK

`.gitignore:18` 把 `app/app/src/main/jniLibs/` 整目录排除；三块原生件都是构建产物：

| 文件 | 谁产出的 | 缺了会怎样 |
|---|---|---|
| `libhuskapp.so` | `cargo build --manifest-path app-rs/Cargo.toml` | `hasCode=false` 又没有这块 = 应用没有任何代码 |
| `libhuskguard.so` | 同上（`husk-session-guard` 改名） | 会话看护不在 |
| `libld.so` | `make loader`（`tools/loader/build-embedded-loader.sh`） | **起不了 guest**，装了就是空壳 |

`make apk` 会先构建三块再调 Gradle，所以开发机上看不出问题；而 CI 的 `apk` 作业跑完之后
才用 `tools/ci/audit-apk-contents.sh` 断言。也就是说：**在 Gradle 这一层，空壳一直是合法的**。

### 修前（负向，实测）

把三块 `.so` 移走，并临时撤掉守卫（`git stash push app/app/build.gradle.kts`）：

```
$ ls app/app/src/main/jniLibs/arm64-v8a/ | wc -l
0
$ (cd app && ANDROID_HOME=$HOME/Library/Android/sdk ./gradlew --offline :app:assembleDebug)
BUILD SUCCESSFUL in 3s
34 actionable tasks: 4 executed, 30 up-to-date
$ unzip -l app/app/build/outputs/apk/debug/*.apk | grep -c "lib/arm64-v8a/"
0
```

**FACT**：构建成功，包内一个原生件都没有。这就是"干净 clone 只跑 gradlew"的真实结果。

### 修后（正向，实测）

`app/app/build.gradle.kts` 末尾加 `huskVerifyNativePieces`，挂在 `preBuild` 上：

```
$ (cd app && ./gradlew --offline :app:assembleDebug)
> Task :app:huskVerifyNativePieces FAILED
FAILURE: Build failed with an exception.
* What went wrong:
Execution failed for task ':app:huskVerifyNativePieces'
> Husk: 缺少原生件 libhuskapp.so, libhuskguard.so, libld.so （目录
  .../jniLibs/arm64-v8a）。这三块是构建产物，不进版本库：请用 `make apk`（或先 `make loader` +
  cargo build ...），不要直接调 Gradle。空壳 APK 装了也起不了 guest。
BUILD FAILED in 7s
```

把三块放回去，同一个命令立刻回到绿色，产物断言两条都过：

```
$ (cd app && ./gradlew --offline :app:assembleDebug)      -> BUILD SUCCESSFUL
$ sh tools/ci/audit-apk-contents.sh apk
OK   lib/arm64-v8a/libhuskapp.so
OK   lib/arm64-v8a/libhuskguard.so
OK   lib/arm64-v8a/libld.so
$ sh tools/ci/audit-apk-version.sh
APK version contract OK: 0.0.36 (36)
```

### 回归

- `make apk` 路径不受影响（它先构建三块）。
- 守卫只在 `preBuild` 跑，不碰 `clean` / `tasks` 之类的调用。

## 2 · H2：`make check` 只查 12/39 个 crate

CI 早就改成 `--workspace`，但本地入口 `make check` 还是手写的那 12 个 `-p`。
两边不一致的后果：本地绿、CI 红，或者反过来；而"本地绿"是开发者最常看的那一个。

### 修前（负向，实测）

往 `runtime/crates/audio-relay/src/main.rs` 末尾注入：

```rust
const _HUSK_NEGATIVE_TYPE_ERROR: u8 = "not a u8";
```

```
$ cargo check --target aarch64-unknown-linux-gnu \
    -p runtime-core -p nr-tools -p ldconfig -p path-shim -p gpu-env -p session-env \
    -p v4l2-preload -p v4l2-ref-probe -p v4l2-test -p camera-fixture -p fd-identity-probe -p drm-preload
    Finished `dev` profile ... (rc=0)     <-- 12 个 crate 全绿，错误在这 12 个之外

$ make check            (改 Makefile 之前)
rc=0                                        <-- 门禁放过了它
```

### 修后（正向，实测）

`Makefile` 的 `check` 改成 `cargo check --workspace` / `cargo clippy --workspace`，与 CI 同一条规则：

```
$ make check
error[E0308]: mismatched types
   --> crates/audio-relay/src/main.rs:210:39
    |                                  --   ^^^^^^^^^^ expected `u8`, found `&str`
error: could not compile `audio-relay` (bin "audio-relay") due to 1 previous error
make: *** [check] Error 2        (rc=2)
```

撤销注入后：

```
$ make check
... 14 tests passed, 0 failed
release offline audit: PASS (version=0.0.36)
version ok: 0.0.36        (rc=0)
```

### 回归

`--workspace` 与 12 个 crate 的结果在干净树上一致（都 rc=0），说明没有把本来不编译的
crate 拉进来（`cargo check --workspace` 之前已在 CI 跑过，本轮本地复现：9.38 s，rc=0）。
`--all-targets` 仍然不加：`path-shim` 等 no_std crate 自带 panic handler，测试目标会撞。

## 3 · H3：单测覆盖与 CI 跑的那一份不一致

修前：39 个 crate 里只有 5 个带 `#[cfg(test)]`（`gpu-env` / `nr-tools` / `session-env` /
`usermgr` / `va-driver`），CI 只跑 3 个（`gpu-env` / `session-env` / `usermgr`）。
`nr-tools` 的测试（libc 插槽表集合相等这类，最该被回归的东西）**一次都没在 CI 里跑过**；
`video-protocol` —— 视频桥的共享内存环协议，生产者/消费者布局一改就静默丢帧 —— **一个测试都没有**。

### 修后

1. `runtime/crates/video-protocol/src/lib.rs` 新增 5 个测试（纯逻辑，不需要设备）：

   | 测试 | 断言什么 |
   |---|---|
   | `layout_rejects_out_of_range_and_is_aligned` | 0 槽 / >64 槽 / 槽小于记录头 / 槽 >64 MiB 全部拒绝；stride 64 字节对齐；`bytes = 4096 + slots*stride` |
   | `hex_round_trips_and_refuses_junk` | 0..=255 往返；奇数长度与非十六进制字符被拒 |
   | `ring_is_fifo_and_reports_full` | 写满报 `WouldBlock`；`read_into` 不推进 `read_seq`、`commit` 才推进；腾出槽位后序号必须连续可写 |
   | `empty_ring_blocks_instead_of_returning_garbage` | 空 ring 读是 `WouldBlock`，不是返回未初始化内存 |
   | `oversized_and_out_of_order_writes_are_refused` | 超槽样本、跳号写、空 ring 上的 `commit` 全部 `InvalidInput`，且 `sequences()` 保持 `(0,0)` |

2. CI `test` 作业从 3 个 crate 变成 6 个：
   `-p gpu-env -p session-env -p usermgr -p nr-tools -p video-protocol -p husk-va-driver`。
   后两个标了 "Linux-host-only"：它们调 `libc::memfd_create`，macOS 上连链接都过不去。

### 本地实测（macOS，能跑的都在跑）

```
$ make check     # 含 cargo test -p gpu-env -p session-env -p usermgr -p nr-tools
test result: ok. 6 passed; 0 failed   (gpu-env)
test result: ok. 1 passed; 0 failed   (nr-tools)
test result: ok. 4 passed; 0 failed   (session-env)
test result: ok. 3 passed; 0 failed   (usermgr)
rc=0
```

## 4 · H4：发布命名契约只有联网半

修前 `tools/ci/audit-release-names.sh` 是一个整体：`git ls-remote` 取远端 tag、`gh api` 读
annotated tag 注释与 Release 正文/资产名，然后才是版本三元组、tag 命名、序列不跳号这些
**完全不需要网络**的规则。它只在 `make release-audit`（人工）里跑，**从没进过 CI**。
后果：tag 序列跳号、`VERSION` 与最新 tag 差好几版这类错误，只有发版当天靠人记得跑才看得见。

### 修后：拆两半，规则只有一份

| 文件 | 跑在哪 | 查什么 |
|---|---|---|
| `tools/ci/audit-release-offline.sh`（新） | CI `scripts` 作业 + `make check` | `VERSION` 是 100 进制三元组；README 首屏头块；tag 命名 `vX.Y.Z`；序列不跳号；`VERSION` 与最新 tag 的一致关系 |
| `tools/ci/audit-release-names.sh`（改） | 人工（`make release-audit`，需 `gh`） | 先调离线半，再查 annotated tag 注释正文、Release 标题/正文结构/资产名 |

`VERSION` 的合法状态明确为两种：**等于最新 tag**（已发版）或**正好是最新 tag 的下一个号**
（这一版正在准备）。否则"准备发版"这个状态本身就会让门禁变红——那等于逼人跳过门禁。

### 负向用例（实测，三个全红）

```
$ printf '0.0.40\n' > VERSION && sh tools/ci/audit-release-offline.sh
VERSION is 0.0.40, but the newest tag is v0.0.35 (allowed: 0.0.35 or 0.0.36)   rc=1
$ printf '0.0.38\n' > VERSION && sh tools/ci/audit-release-offline.sh
VERSION is 0.0.38, but the newest tag is v0.0.35 (allowed: 0.0.35 or 0.0.36)   rc=1
$ printf '0.0.100\n' > VERSION && sh tools/ci/audit-release-offline.sh
VERSION is not a 100-carry triple (0..99 per component, no leading zeros)      rc=1
$ cat VERSION
0.0.36                                                                          (已复原)
```

正向：

```
$ sh tools/ci/audit-release-offline.sh
README header: PASS (title -> icon -> badges -> 概览)
VERSION 0.0.36 is consistent with newest tag v0.0.35
release offline audit: PASS (version=0.0.36)
```

CI 侧给了 `fetch-depth: 0`，否则 checkout 没有 tag，序列检查会打 `SKIP` 而不是通过
——这个 `SKIP` 是显式打印的，不会被当成绿灯。

## 5 · 本地复核失败的部分（照写）

想在 docker 里复跑一遍 Linux 侧的 `cargo test -p husk-va-driver`：

```
$ docker run --rm -v "$PWD":/w -w /w/runtime ubuntu:24.04 bash -lc \
    'apt-get install -y curl build-essential && curl -sSf https://sh.rustup.rs | sh -s -- -y ...'
# 6 分钟卡在 `apt-get install -y -qq curl build-essential`，容器被 kill
$ docker pull rust:1.92-slim      # 3 分钟没有下载出任何层
```

**UNVERIFIED**：`video-protocol` 的 5 个新测试与 `husk-va-driver` 的 1 个既有测试，
在本机（macOS）上只能做到 `cargo check --target aarch64-unknown-linux-gnu` 通过，
**没有在本地执行过**；它们进 CI 之后由 ubuntu runner 执行，以那次运行结果为准。
不得把"编译通过"当成"测试通过"。

## 6 · 本轮改动文件

| 文件 | 改动 |
|---|---|
| `app/app/build.gradle.kts` | 新增 `huskVerifyNativePieces`，挂 `preBuild`（H1 判据） |
| `Makefile` | `check` 的 check/clippy 改 `--workspace`；test 加 `-p nr-tools`；新增离线发布审计（H2/H4） |
| `.github/workflows/ci.yml` | test 作业加 `nr-tools`/`video-protocol`/`husk-va-driver`；`scripts` 作业加离线发布审计 + `fetch-depth: 0`（H3/H4） |
| `tools/ci/audit-release-offline.sh` | 新文件：发布契约离线半 |
| `tools/ci/audit-release-names.sh` | 删掉重复的本地规则块，改为调用离线半，只保留联网半 |
| `runtime/crates/video-protocol/src/lib.rs` | 新增 5 个单元测试 |

## 7 · 下一步（第 3 轮入口，不在本轮范围）

- `runtime-core` 的符号链接递归深度 16 → 约 40，且必须先把 `resolve_at` 的 4 KiB 栈缓冲
  挪出递归帧，否则是拿栈溢出换 ELOOP；测 16/17/40/41 层。
- 假 DRM / V4L2 / netlink fd 的身份判定改精确名字（现在 `drm-preload` 用子串 `mali`）。
- `usermgr` 四库事务化；只读挂载补 `chmod`/`chown`/`utimensat`。
