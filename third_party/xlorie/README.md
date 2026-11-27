# Xlorie：Husk 内嵌 X11 服务端

`libXlorie.so` 是 Husk 自己的 X11 服务端实现。它运行在 Husk Android 应用进程内，
直接使用 `NativeActivity` 提供的 `ANativeWindow`，不依赖第二个 APK、Java
`LorieView`、`MainActivity` 或独立 X11 应用。

## 来源与许可证

- 上游：<https://github.com/termux/termux-x11>
- 基线提交：`e382844d468881c3a3460291cb02d4eaaa84c024`
- Husk 补丁：`patches/0001-husk-embedded-x11.patch`
- 许可证：GNU General Public License v3.0，见 `LICENSE`

`libXlorie.so` 是对应二进制。完整对应源码由以下三部分组成：

1. 上述上游提交及其固定子模块；
2. `patches/0001-husk-embedded-x11.patch`；
3. `build.sh` 中的 Android 构建参数。

补丁 SHA-256：

```text
3ca7797cd79f64ff60fd0eb10e312044bddec97e95dc107dd9e28e695415504a  patches/0001-husk-embedded-x11.patch
7ba07fbb866977622b1f072b805b29dcaacdf208d6eb6d8cc761ea4f61b4c59d  patches/0002-libepoxy-android.patch
5baf23495b11c8bec9e184d2686202dc731b5cb9ffc671b4b5463d69d1ed5f4f  patches/0003-libx11-android.patch
b336a927917dfca924fc48e46c8f18653ea630163de47e0edfddcdb3adabb4cf  patches/0004-libxkbfile-android.patch
aa3442ab3dc0c871cd410662dfd27ba79192195bbeda4059a02e6042d31062a1  patches/0005-libxtrans-android.patch
b6e4fe1d15c1e1155d123dc95435cccd7aa3594bceea69789e8e250634c56904  patches/0006-pixman-android.patch
9646b19922e8d4583aee24e264d815e14bbb2b6f1667311a844db09aefe987b6  patches/0007-xkbcomp-android.patch
c51cf76c6cfe7bd38ed346136e582c93f399749cf18abdacb1c04991db0d3479  patches/0008-xserver-android.patch
```

`0002`–`0008` 是固定子模块在 Android/NDK 下的必要补丁，不是未记录的本地改动。
`build.sh` 会逐个子模块校验 SHA-256，再执行 `git apply --check` 和 `git apply`。

当前随仓库分发的二进制 SHA-256：

```text
76b4abc6827a0494c43ba55dcbc68cd8ad69d1ec0a761a8b9af6c5350b0abc34  arm64-v8a/libXlorie.so
```

该文件使用 Release 构建并在复制到仓库前执行 `llvm-strip --strip-all`，不携带
构建目录、调试符号或 GNU build-id。两个不同 `XLORIE_BUILD_DIR` 的干净构建必须得到
同一 SHA-256；不同哈希视为发行阻断。

## Husk 修改范围

补丁保留上游 X server、Composite、DRI3、Present、输入与 AHardwareBuffer
基础设施，只修改应用绑定和 Husk 需要的呈现/输入边界：

- 去掉对 Termux:X11 Java `LorieView` 和 `MainActivity` 的启动依赖；
- 增加 `husk_xlorie_*` C ABI，供 Rust NativeActivity 直接调用；
- 接收 `ANativeWindow` 并把 X11 根缓冲区呈现到 Android Surface；
- 通过 Unix socket 注入触摸、鼠标、键盘和窗口尺寸事件；
- 保留 AHardwareBuffer 根缓冲区与 GPU copy，供截图和呈现读取；
- 保留 X11，不引入 Wayland 或第二套合成器。

## 重新构建

要求：

- Git；
- Android NDK `29.0.14206865`；
- CMake 3.22 或更新版本；
- Ninja。

```bash
export ANDROID_NDK_HOME="$HOME/Library/Android/sdk/ndk/29.0.14206865"
export XLORIE_BUILD_DIR="/tmp/husk-xlorie-build"
third_party/xlorie/build.sh
```

构建成功后脚本会更新 `third_party/xlorie/arm64-v8a/libXlorie.so` 并打印 SHA-256。
如果二进制发生变化，必须同步更新本文件、APK 审计和真机证据，不能只替换文件。

脚本只重置 `XLORIE_BUILD_DIR/termux-x11` 下的临时 Git 树和 CMake 输出目录，可重复执行。
复用已有构建目录是受支持的；脚本会清理旧补丁和旧对象，不会复用上次的源码状态。

## 边界

Xlorie 解决 X11 协议和 Android Surface 之间的呈现问题，不负责 Linux 路径语义，
也不是安全隔离层。Husk 的路径兼容与安全边界仍由 Loader/Path Shim 和独立威胁模型决定。
