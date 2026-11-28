# 368 · 包名统一与 README 重排（2026-11-28）

## 结论

- 活动运行路径、Makefile、App、设备测试、探针和基准工具统一使用
  `com.linuxhub.myth.husk`。
- `tools/ci/audit-runtime-package.sh` 的 canonical 值与五个固定入口同步到同一包名。
- README 已整篇重排：首屏改为居中简介、技术栈徽章和中英文/文档导航；正文从目录、概览和
  跑起来的前提开始；独立的“为什么是 Husk”已合并进概览；性能、当前状态和已知限制放到后。
- 新增 `README_EN.md`，提供同一首屏和英文入口。
- 旧 evidence 未修改；历史原始输出保持原样。

## 验证

```text
runtime package audit: PASS (com.linuxhub.myth.husk)
loader repro: PASS (f2fe07f886712fad97c5b35aa759e50c8ab4bbb69ad7b7ec6ddeeef48619ec18, 1263104 B)
checked 398 markdown files, 0 broken link(s)
version ok: 0.0.38
```

`make check` 使用本机可用的 clang/llvm 工具链完成，覆盖格式化、aarch64 workspace check、Clippy、
App 检查、脚本语法、README/文档链接、版本、包名、loader 可复现构建、PanVK bundle 和运行时
边界审计。

使用 Android Studio NDK `28.2.13676358`（darwin-x86_64）重跑：

```text
NDK=/Users/l/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64/bin make test
Finished `test` profile [unoptimized + debuginfo] target(s) in 2.82s
```

生成 `gpu-env`、`nr-tools`、`runtime-core`、`v4l2-shim` 四组 aarch64 测试可执行文件。

## 未验证

- 当前没有连接设备；没有声称新的设备运行结果。
- 设备侧包名切换后的安装、升级和门禁运行标为 **NOT VERIFIED**，需要连接设备后重跑。
