# 与既有 Terminal 应用的集成

宿主的终端壳用既有应用（`com.terminal`，Kotlin + 原生 PTY 渲染器），不自己造 UI。

## 改动（两个文件）

| 文件 | 改动 |
|---|---|
| `app/src/main/java/com/terminal/HuskSession.kt` | 新增。判断 rootfs 与 loader 是否就绪，产出 argv/env |
| `app/src/main/java/com/terminal/NativeTerminalRenderer.kt` | `start()` 里优先启动 Husk 会话，否则回落到 `/system/bin/sh -i` |

```
HuskSession.resolve(context)
  loader  = nativeLibraryDir/libld.so
  rootfs  = filesDir/rootfs
  条件    = loader 存在 && rootfs/usr/bin/bash 存在 && rootfs/usr/lib/aarch64-linux-gnu 存在
  argv    = <loader> --library-path <rootfs>/usr/lib/aarch64-linux-gnu <rootfs>/usr/bin/bash -i
  env     = NR_ROOTFS, NR_LD, TERM, LANG, USER, HOME=/root, SHELL, PATH
```

## 设备侧准备

```sh
# 1) loader 进 jniLibs（build-embedded-loader.sh 的产物）
cp <repo>/tools/nr-loader-hook/ld-embedded.so app/src/main/jniLibs/arm64-v8a/libld.so

# 2) rootfs 进应用私有目录
adb shell "run-as com.terminal sh -c 'mkdir -p files/rootfs && cd files/rootfs && tar xf /data/local/tmp/husk-rootfs.tar'"

# 3) 构建安装
JAVA_HOME=<jdk> ./gradlew :app:assembleDebug && adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`libld.so` 必须被解包到 `nativeLibraryDir`（该工程已设 `jniLibs.useLegacyPackaging = true`）。

## 现状

集成代码可编译（`BUILD SUCCESSFUL`）、APK 可安装、rootfs 已就位；**但交互式会话不显示任何输出**
（详见 `docs/STATUS.md` 的未解决项）。这一条与自建壳的表现一致，说明问题在"loader + 交互式 bash + PTY"这条路径，
不在宿主 UI。
