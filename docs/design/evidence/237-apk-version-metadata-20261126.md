# 237 · APK 版本号与仓库 VERSION 脱节

日期：2026-11-26 02:12（CST）。审计对象：`app/app/build.gradle.kts`。

## 1 · 结论

| 项 | 修前 | 修后 |
|---|---|---|
| 仓库 `VERSION` | `0.0.13` | 当前 `0.0.14`（发布时） |
| APK `versionName` | `0.1`（硬编码） | 从根目录 `VERSION` 读取 |
| APK `versionCode` | `1`（硬编码） | 从 `VERSION` 逐段换算，当前为 `13` |

FACT：修前 APK 与仓库/Release 版本不一致，安装后的 `dumpsys package` 会把新构建标成
`0.1`，也无法用 versionCode 判断升级顺序。这是 release engineering 缺陷，不是运行时权限问题。

## 2 · 修法

`app/app/build.gradle.kts` 不再写死版本：

```kotlin
val huskVersion = rootProject.file("../VERSION").readText().trim()
val huskVersionCode = huskVersion.split('.').fold(0) { value, part ->
    value * 100 + part.toInt()
}
...
versionCode = huskVersionCode
versionName = huskVersion
```

版本号唯一来源仍是 `VERSION`；CHANGELOG 和 tag 的一致性继续由 `make version-check` 检查。
APK 构建后再用 `aapt dump badging` 或 `dumpsys package` 验证，不能只看 Gradle 配置。

## 3 · 验证状态

- FACT：`make check` 通过，`version ok: 0.0.13`。
- FACT：2026-11-26 02:13 CST，使用 NDK 28.2 完整执行 `make apk`，Gradle `BUILD SUCCESSFUL`；
  产物 `app/app/build/outputs/apk/debug/app-debug.apk` 为 1,680,716 字节。
- FACT：APK 内 `versionName=0.0.13`、`versionCode=13`。
- FACT：`adb install -r` 覆盖安装到 `com.linuxhub.husk` 后，设备
  `dumpsys package com.linuxhub.husk` 实测为 `versionCode=13`、`versionName=0.0.13`；
  `lastUpdateTime=2026-11-26 02:13:24`。
- FACT：APK 内 `lib/arm64-v8a/libld.so` 与构建产物 SHA-256 一致：
  `d708257b1b1c80e8c4570c07a7f861897fbb4551cd5831f96dd31d6004a74e44`。
- FACT：覆盖安装后 App 冷启动成功，Rust 会话看门狗 ready，guest bash 进入 running；
  终端输入 `exit` 后会话正常退出，点击死会话触发 `session: restart requested by user`，
  随后生成新的 guard 与 guest 进程。首次漏点击仍未复现；原因 UNKNOWN。
- FACT：发布前将 `VERSION` 升到 `0.0.14` 后重新构建，APK 内为 `versionName=0.0.14`、
  `versionCode=14`；覆盖安装后的设备 `dumpsys package` 与 APK manifest 一致，
  `lastUpdateTime=2026-11-26 02:16:27`。
- FACT：最终 `0.0.14` APK 再次冷启动成功，`exit → 点击 → guard ready → guest running`
  重启链路复跑通过；最终 loader SHA-256 仍为
  `d708257b1b1c80e8c4570c07a7f861897fbb4551cd5831f96dd31d6004a74e44`。
