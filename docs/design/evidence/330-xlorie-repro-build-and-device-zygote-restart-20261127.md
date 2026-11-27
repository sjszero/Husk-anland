# 330：Xlorie 可复现构建与设备 zygote 重启阻断

日期：2026-11-27

## 结论

- **FACT**：`third_party/xlorie/arm64-v8a/libXlorie.so` 已从不可复现的
  `RelWithDebInfo` 构建改为确定性的 Release + `llvm-strip --strip-all` 构建。
- **FACT**：两个不同 `XLORIE_BUILD_DIR` 的独立构建都得到
  `76b4abc6827a0494c43ba55dcbc68cd8ad69d1ec0a761a8b9af6c5350b0abc34`。
- **FACT**：APK 已重新构建，四个原生件均在包内，版本仍为 `0.0.36 / 36`。
- **UNVERIFIED**：禁用 MIT-SHM 后桌面是否不再黑屏。设备在启动 Husk 时发生
  Android 用户空间重启，未拿到 Husk 自身的启动日志。
- **INFERENCE**：本次重启高度疑似由设备现存线程耗尽触发，而不是已证实的 Husk 缺陷；
  当前 `u0_a300` 有 26,181 个进程/线程项，多个系统应用持续报
  `OutOfMemoryError: pthread_create ... Try again`。

## 修前问题

1. `build.sh` 对已应用的源码树再次执行 `git apply`，第二次构建直接失败。
2. 源树和子模块没有先重置，旧补丁/旧对象可能污染下一次构建。
3. 使用 `RelWithDebInfo`，调试段和构建目录字符串进入 ELF；相同 `.text/.rodata/.data`
   的两个构建仍会产生不同 SHA-256。
4. README 记录的旧哈希 `7989f2af...` 与后来工作树中的二进制不一致。

修前实测：

```text
RelWithDebInfo 构建 A  fd35d0991cd21d4412a506d8d154a57eae41c87b45407229004dfab9060d0602
RelWithDebInfo 构建 B  6b8c10035724e80a0804416319826dc458282f121387048f416ce1b61aa8e47d
.text / .rodata / .data  两者完全相同
差异来源：GNU build-id、debug_* 段、临时构建路径和 symtab
```

## 修复

`third_party/xlorie/build.sh` 现在：

- 只重置 `XLORIE_BUILD_DIR/termux-x11` 这个受控临时树；
- `git reset --hard`、`git clean -fdx`、递归重置所有子模块；
- 删除旧 CMake 输出目录，禁止复用旧对象；
- 使用 `Release`、固定的文件/调试/宏路径映射和 `-Wl,--build-id=none`；
- 复制前执行 NDK `llvm-strip --strip-all`。

新增 `tools/ci/audit-xlorie-asset.sh`，固定校验：

- 二进制 SHA-256；
- README 中的哈希钉；
- ELF64 / AArch64 / DYN；
- 不存在 `.debug_*` 和 `.note.gnu.build-id`；
- 9 个 `husk_xlorie_*` C ABI 符号。

## 修后验证

两个不同构建目录分别执行：

```bash
XLORIE_BUILD_DIR=/tmp/husk-xlorie-a.OGBa7r JOBS=8 third_party/xlorie/build.sh
XLORIE_BUILD_DIR=/tmp/husk-xlorie-b.eKPwUy JOBS=8 third_party/xlorie/build.sh
```

结果：

```text
构建 A  76b4abc6827a0494c43ba55dcbc68cd8ad69d1ec0a761a8b9af6c5350b0abc34
构建 B  76b4abc6827a0494c43ba55dcbc68cd8ad69d1ec0a761a8b9af6c5350b0abc34
```

APK 验证：

```text
make apk                                      PASS
bash tools/ci/audit-apk-contents.sh            PASS
app-debug.apk                                 3,506,725 bytes
versionCode / versionName                     36 / 0.0.36
```

包内原生件：

```text
lib/arm64-v8a/libld.so
lib/arm64-v8a/libhuskapp.so
lib/arm64-v8a/libhuskguard.so
lib/arm64-v8a/libXlorie.so
```

## 真机阻断

`adb install -r` 成功，包版本读取为 `36 / 0.0.36`。启动
`com.linuxhub.husk/android.app.NativeActivity` 时，Android `activity` 服务返回
`Broken pipe (32)`，随后 ADB 短暂消失。设备恢复后确认：

```text
persist.sys.system.abnormalboot=2026-11-27 10:43:00:zygote reboot by zygote_secondary
/proc/loadavg=262.77 112.69 55.56
u0_a300 processes/threads=26181
system_server / permissioncontroller / com.tencent.mm:
  java.lang.OutOfMemoryError: pthread_create (1040KB stack) failed: Try again
```

没有取得 Husk PID、`desktop: start`、Xlorie renderer 或 MIT-SHM 相关日志，因此不能把
本机 Android 重启写成“Husk 已修好”，也不能据此判定 Husk 启动必然导致重启。

## 待办

- 在 `u0_a300` 资源泄漏消失、系统 load 恢复且 `activity` 服务稳定后，只启动
  `com.linuxhub.husk` 重测；
- 验证 `screencap` 非黑、X11 连接成功、无 `Failed to attach to x11 shm`；
- 再执行输入、多应用并发、GPU audit、soak 和门禁 A / A-hw。

## 边界

不得为了本次测试清理或杀死 `com.linuxhub.myth`、`com.linuxhub.myth:x11`、`u0_a300`
或 PID 6626。Xlorie 是 GPLv3 的 X11 服务端组件；Husk 的路径兼容与安全边界仍由
Loader/Path Shim 和独立威胁模型决定。
