# 239 · Husk 0.0.14 发布验证

日期：2026-11-26 02:16 CST。设备：RMX3357 / Android 13 / kernel 4.19.191 / aarch64 /
Mali-G77 / 4 KiB 页。测试包：`com.linuxhub.husk`。本轮只修改和研究 Husk，
没有改动 LinuxHub 目录，也没有操作 `com.linuxhub.myth.husk` 的数据。

## 1 · 发布内容

0.0.14 是基于 0.0.13 的补审修复版，收掉 4 条发布后确认的问题：

| 编号 | 修复 | 证据 |
|---|---|---|
| H-25 | 合成 `/proc`/sys VFS 的共享 4 KiB 缓冲并发串内容 | 235 |
| H-26 | 虚构身份状态的跨线程非原子发布 | 236 |
| H-27 | APK 版本元数据与仓库 `VERSION` 脱节 | 237 |
| H-28 | NDK `<prebuilt>` / `<prebuilt>/bin` 约定冲突，含并发探针脚本 | 238 |

## 2 · 静态门禁

```text
cargo fmt --all --check                  PASS
cargo check (target aarch64)             PASS
cargo clippy (runtime-core/nr-tools/ldconfig/path-shim) PASS
shell syntax: 69 script(s) checked       PASS
markdown: 260 files, 0 broken links      PASS
version ok: 0.0.14                       PASS
```

FACT：`make check` 退出码为 0。它仍有大量编译告警（path-shim 本轮一次构建报告 92 条，
全量 clippy 输出更多）；这些不是本次新增的 4 条修复，但也不能把仓库描述成“零告警”。

## 3 · APK 与安装

FACT：使用 NDK 28.2 和 `NDK=<prebuilt>/bin` 执行 `make apk` 成功，Gradle 输出
`BUILD SUCCESSFUL`，产物为：

```text
app/app/build/outputs/apk/debug/app-debug.apk  1680716 bytes
```

FACT：APK manifest 与设备安装结果一致：

```text
versionName=0.0.14
versionCode=14
lastUpdateTime=2026-11-26 02:16:27
```

FACT：APK 内 loader 与构建产物哈希一致：

```text
d708257b1b1c80e8c4570c07a7f861897fbb4551cd5831f96dd31d6004a74e44
```

## 4 · 真机会话

冷启动：

```text
session: guard pid=31192 ready
spawned pid=31196 ... armed=true
session: running pid=31196 ... 80x24
```

终端输入 `exit` 后：

```text
session: exited pid=31196 status=0 (exit 0)
```

点击已退出会话后：

```text
session: restart requested by user
session: guard pid=31721 ready
spawned pid=31722 ... armed=true
session: running pid=31722 ... 67x58
```

FACT：冷启动、guest bash 运行、正常退出、点击重启四条路径均通过。首次点击未触发的旧现象
本轮没有复现；触发条件仍是 UNKNOWN，不能据此宣称所有输入时序都已修好。

## 5 · 并发回归

最终 `0.0.14` 的 loader 上，16 线程 × 4000 轮合成 VFS 探针：

```text
RESULT bad=0 ok=64000 threads=16 rounds=4000 opens=64000
```

FACT：64,000 次打开没有跨文件串内容。修前基线见 evidence 235：35,348/64,000 失败。

## 6 · 未覆盖与限制

- UNKNOWN：本版没有重跑完整门禁 A。现有完整 fixture 属于另一个测试包；本轮按范围约束
  没有操作它。设备上的 `com.linuxhub.husk` rootfs 又缺完整门禁 fixture（例如 `/usr/bin/gcc`）。
  因此“0.0.14 门禁 A 通过”不成立，只有证据 239 列出的路径已验证。
- UNKNOWN：本轮没有重新验证 GNOME 桌面、X11 窗口呈现、音频和视频链路；这些沿用 0.0.13
  及更早证据，不外推为 0.0.14 新验证。
- KNOWN FAILURE：`glxgears` 在当前 PanVK/Bifrost 编译器路径仍会 SIGSEGV（evidence 232）；
  本版没有修改 GPU 栈，不能写成 GPU 长跑通过。
- UNKNOWN：Adreno/KGSL 只有代码与 Mali 回归，没有 Adreno 真机证据。
- FACT：APK 是 `assembleDebug` 产物，不是本证据验证过的商店发布签名包。
- FACT：仓库仍有 path-shim 大文件和并发静态状态告警；这是下一步全链路审计的输入，
  不能因为本版 4 条修复通过就视为机制已完全收口。

## 7 · 发布判定

本轮可以发布的依据是：版本四处一致、构建成功、安装版本正确、loader 哈希一致、
会话启动与重启通过、64,000 次并发回归通过。不得把未重跑的门禁 A、桌面、GPU 长跑或
跨设备兼容说成已通过。
