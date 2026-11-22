# 快速开始

这份文档只做一件事：让你从零把 Husk 跑起来，并亲手看到 `bash`、`gcc`、`make` 在
Android 的 `untrusted_app` 域里正常工作。

预计时间：首次 20–30 分钟（大部分在下载 NDK / Gradle / Ubuntu 工具链）。

---

## 0. 你需要

| 项 | 要求 |
|---|---|
| 主机 | macOS 或 Linux，能跑 `adb` |
| Rust | stable，且装了 `aarch64-unknown-linux-gnu` 与 `aarch64-linux-android` 两个 target |
| NDK | r27+（设置 `NDK=<...>/toolchains/llvm/prebuilt/<host>/bin`） |
| JDK | 17+（Android Studio 自带 jbr 即可） |
| 设备 | 任意能 `adb` 的 Android 手机，**不需要 root、不需要解 BL、不需要开虚拟化** |
| rootfs | 一份 glibc 发行版目录树（本文以 Ubuntu 24.04 arm64 为例） |

检查：

```sh
rustup target list --installed | grep aarch64
adb devices
```

## 1. 准备 rootfs 的位置

探针 APK（`com.linuxhub.probe`）把 rootfs 放在自己的私有目录：

```
/data/data/com.linuxhub.probe/files/nrfull/root
```

app-private 目录不可执行但**可读**，这正好够用：loader 从 `nativeLibraryDir` 读，rootfs 只当数据。

> 你的 rootfs 从哪来不重要（debootstrap / 官方 rootfs tarball 都行），只要
> `usr/lib/aarch64-linux-gnu/libc.so.6` 与 `ld-linux-aarch64.so.1` 是 glibc 2.39 这一代。

## 2. 改出带 shim 的 loader

```sh
export NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin
./tools/loader/build-embedded-loader.sh
```

它做三件事，并打印它断言过的数字（76 个介入名 / 31 个外部符号 / 24 个自相对槽位）：

1. `cargo build` 出 `path-shim`（4K 段对齐）
2. 编译 `init-hook.S`，把 `svc` 调用点按模式定位并 `nop`
3. `nr-tools embed` 把 shim 作为额外 `PT_LOAD` 嵌进 `ld.so`

产物：`tools/probe-app/app/src/main/jniLibs/arm64-v8a/libld.so`
期望 sha256：`1600a850985624b05c8b1c1f9cee5fe8185462da91d0de369dcb9ff30f21cbee`

> 想验证 Rust 移植没跑偏：`NR_TOOLS=python ./tools/loader/build-embedded-loader.sh`
> 两条路径必须产出同一个 sha256。

## 3. 装工具链进 rootfs（可选，但这是最好玩的部分）

```sh
./tools/device-tests/provision-toolchain.sh
```

纯 Rust 实现：解析 `ports.ubuntu.com` 的 Packages 索引 → 下载 53 个 deb →
在主机上拆包 → 建好 `gcc`/`cc`/`as`/`ld` 等通用名 → 打包成 tar 流经 `run-as` 推进 rootfs。

它会**故意跳过 `libc6`**：rootfs 里的 libc 是改过的，覆盖就把改动冲掉了。

## 4. 跑

```sh
export DEV=<你的 adb 序列号>          # adb devices 第一列
cd tools/device-tests
DEV=$DEV ./run-guest.sh 01-functional-matrix.sh   # 期望 31 PASS / 0 FAIL
DEV=$DEV ./run-guest.sh 03-containment.sh         # 期望 #escapes=0
DEV=$DEV ./run-guest.sh accept-gcc-make.sh        # gcc + make 全链
```

`run-guest.sh` 会：push 脚本进 rootfs → 用 gradle 构建探针 APK 并安装 →
`am start` 让 APK 在自己的进程里 exec `libld.so`（也就是改过的 loader）→
把输出拉回来。

## 5. 跑到壳应用里

`app/` 是当前的产品入口：单会话终端，启动 rootfs 里的交互式 `bash`。

```sh
export NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin
J=app/app/src/main/jniLibs/arm64-v8a

# 1) 改过的 loader 进 jniLibs
NDK=$NDK OUT=$PWD/$J ./tools/loader/build-embedded-loader.sh

# 2) PTY 辅助库
$NDK/aarch64-linux-android24-clang -shared -fPIC -O2 -o $J/libhuskpty.so \
  app/app/src/main/jni/pty.c -llog

# 3) 构建安装（app/local.properties 被 .gitignore 忽略，需要自己写一行 sdk.dir）
echo "sdk.dir=$ANDROID_HOME" > app/local.properties
cd app && JAVA_HOME="<JDK 17+>" ./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk && cd ..

# 4) 导入 rootfs 到应用私有目录（约 155 MB）
adb shell "run-as com.linuxhub.husk sh -c 'mkdir -p files/rootfs && cd files/rootfs && tar -xf /data/local/tmp/husk-rootfs.tar'"
```

打开应用会先跑一次自检（`bash -c 'echo HI; id; pwd; ls /'`），然后进入交互会话。

## 6. 换自己的命令

```sh
DEV=$DEV ENV="FOO=bar" EXTRA=my.c ./run-guest.sh my-script.sh
```

- `ENV=` 追加环境变量（逗号分隔）
- `EXTRA=` 把宿主文件一起推进 rootfs（比如 C 源码）

## 常见问题

| 现象 | 原因 / 处理 |
|---|---|
| `DEV: parameter null or not set` | 没设 `DEV`；这是有意的，避免写死设备序列号 |
| APK 装不上 | 设备没开 USB 调试，或包名冲突：`adb uninstall com.linuxhub.probe` |
| `run-as: package not debuggable` | 探针 APK 必须用 debug 构建 |
| 卡在 gradle | 首次要下依赖；离线环境需要预置 gradle 缓存 |
| guest 里 `os-release` 读不到 | rootfs 路径不对：确认是 `files/nrfull/root` 且 APK 能看到 |
| 一切正常但 `make` 报 `/bin/sh` 不存在 | 工具链没装或 `dash` 缺失：重跑第 3 步 |
