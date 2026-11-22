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
| rootfs | 由 `nr-tools rootfs` 生成（第 1 步），或自备一份 glibc 2.39 aarch64 目录树 |

检查：

```sh
rustup target list --installed | grep aarch64
adb devices
```

## 1. 生成 rootfs（主机上，一条命令）

```sh
cargo build --release -p nr-tools --manifest-path runtime/Cargo.toml
./runtime/target/release/nr-tools rootfs \
  --out /tmp/husk-rootfs --tar /tmp/husk-rootfs.tar.gz
```

它解析 `ports.ubuntu.com` 上的 noble 索引 → 下载 61 个包 → 解包成目录 →
补 usr-merge 符号链接、`/etc/passwd`、`dpkg` 状态库、`apt` 源 →
把 guest libc 里两个可选加速器调用 `nop` 掉（拒绝时直接失败）→ 打 tar。
产物约 25 MB、`usr/bin` 里有 253 个程序，`bash`、`coreutils`、`dpkg`、`apt` 都在。

> 已经有别处做的 rootfs tarball 也能用，只要 libc 是 glibc 2.39 这一代；
> 但 loader 必须和镜像同源（见下一节）。

## 2. 改出带 shim 的 loader

```sh
export NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin
J=app/app/src/main/jniLibs/arm64-v8a

# a) 先编出 shim 与钩子（这条命令同时会用固定镜像产出一份 loader 作为回归基准）
OUT=/tmp/loader-check ./tools/loader/build-embedded-loader.sh

# b) 用刚才那份 rootfs 自己的 ld.so 产出配套 loader
./runtime/target/release/nr-tools rootfs \
  --out /tmp/husk-rootfs --cache /tmp/nr-rootfs/deb \
  --shim runtime/target/aarch64-unknown-linux-gnu/release/libpath_shim.so \
  --hook tools/loader/init-hook.bin \
  --loader-out $J/libld.so
```

为什么不用一份 loader 配所有镜像：loader 和它加载的 libc 必须是同一个 glibc 构建。
`--loader-out` 就是在镜像自己的 `ld-linux-aarch64.so.1` 上做同样的三处改动，
产出 `libld.so`（约 290,816 B）。

固定镜像的回归基准：`OUT=/tmp/x ./tools/loader/build-embedded-loader.sh` 的产物应为
sha256 `1600a850985624b05c8b1c1f9cee5fe8185462da91d0de369dcb9ff30f21cbee`；
换了镜像，这个值自然不同，这不是退化。

## 2.5 把 rootfs 放进 app

```sh
adb push /tmp/husk-rootfs.tar.gz /data/local/tmp/
adb shell "run-as com.linuxhub.husk sh -c 'mkdir -p files/rootfs && cd files/rootfs && tar -xzf /data/local/tmp/husk-rootfs.tar.gz'"
```

探针 APK（`com.linuxhub.probe`）的路径是 `files/nrfull/root`，测试脚本按它找。

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
壳应用没有 Java：APK 声明 `android.app.NativeActivity`，逻辑全在 `app-rs/` 的 Rust 库里。

```sh
export NDK=$ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin
J=app/app/src/main/jniLibs/arm64-v8a

# 1) 改过的 loader 进 jniLibs
NDK=$NDK OUT=$PWD/$J ./tools/loader/build-embedded-loader.sh

# 2) 壳应用本体（Rust -> libhuskapp.so）
cd app-rs
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android
cp target/aarch64-linux-android/release/libhuskapp.so ../$J/
cd ..

# 3) 打包安装（gradle 只负责把两个 .so 塞进 APK；app/local.properties 被忽略，需自己写）
echo "sdk.dir=$ANDROID_HOME" > app/local.properties
cd app && JAVA_HOME="<JDK 17+>" ./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk && cd ..

# 4) 导入 rootfs 到应用私有目录（第 2.5 步已经推过就跳过）
adb shell "run-as com.linuxhub.husk sh -c 'mkdir -p files/rootfs && cd files/rootfs && tar -xzf /data/local/tmp/husk-rootfs.tar.gz'"
```

终端解析器有单元测试，可以在主机上直接跑：`cd app-rs && cargo test`。

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
