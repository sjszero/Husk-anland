# 238 · `make apk` 的 NDK 路径自相矛盾

日期：2026-11-26 02:13（CST）。审计对象：`Makefile` 与
`tools/loader/build-embedded-loader.sh`。

## 1 · 结论

FACT：同一份 Makefile 对 `NDK` 有两种互相冲突的约定：

- `make loader` 把 `NDK` 当作 `<prebuilt>/bin`，执行 `$NDK/clang`；
- `make apk` 把 `NDK` 当作 `<prebuilt>`，执行 `$NDK/aarch64-linux-android24-clang`。

因此按脚本自己写的 `NDK=<ndk>/bin` 调用 `make apk`，会在 loader 阶段报：

```text
tools/loader/build-embedded-loader.sh: line 56:
  .../darwin-x86_64/clang: No such file or directory
```

这是构建入口缺陷，不是 NDK 安装损坏。

## 2 · 修法

1. `build-embedded-loader.sh` 接受两种写法：以 `/bin` 结尾就直接用，否则自动补 `/bin`；
   并在编译前断言 `$NDK/clang` 存在。
2. Makefile 先把 `NDK_BIN` 归一化为真正的工具目录，再选择
   `$NDK_BIN/aarch64-linux-android24-clang` 和 `$NDK_BIN/llvm-ar`。
3. `run-virt-concurrency-probe.sh` 做同样的归一化；修前传 `<prebuilt>/bin` 会拼出
   `<prebuilt>/bin/bin/clang`。

修后以下两种调用都必须能走到同一个 Gradle 构建：

```sh
NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host> make apk
NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin make apk
```

## 3 · 验证状态

- FACT：第一次完整构建暴露了反向错误：旧 Makefile 把 `<prebuilt>/bin` 去掉后拼成
  `<prebuilt>/aarch64-linux-android24-clang`，app-rs 阶段报 `linker not found`。
- FACT：修正为 `NDK_BIN` 后，使用 `NDK=<prebuilt>/bin` 执行 `make apk` 成功；loader、
  app-rs 两个 native 产物和 Gradle `assembleDebug` 全部完成。
- FACT：并发探针脚本传 `<prebuilt>/bin` 修前报
  `missing clang: .../bin/bin/clang`；修正后用同一写法完成编译，并实测
  `RESULT bad=0 ok=64000 threads=16 rounds=4000 opens=64000`。
- FACT：两种 NDK 写法现在由同一个 `NDK_BIN` 归一化规则处理；本轮完整 APK 使用
  `<prebuilt>/bin` 形式通过，探针使用相同形式复跑通过。`<prebuilt>` 形式的代码路径已检查，
  但未把两种形式都各构建一次，属于 INFERENCE 而非双跑验证。
- FACT：`VERSION` 升到 `0.0.14` 后再次用同一 NDK 写法完整构建 APK，loader、app-rs、
  Gradle 均成功；说明修复不是只对旧版本偶然成立。
