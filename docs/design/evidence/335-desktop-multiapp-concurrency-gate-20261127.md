# 335：桌面多应用 GPU/进程并发门禁

日期：2026-11-27

## 结论

- **FACT**：第 6 轮原有 `gpu-soak` 只启动四个无桌面的读回工作负载；`gpu-audit` 只检查
  一个 nested GTK 应用。两者都不能证明 `gnome-text-editor`、`gnome-calculator` 和
  Nautilus 同时运行在同一个内嵌 X11 会话里，仍继承 Mali fd、PanVK/Vulkan、DISPLAY 和
  D-Bus 会话。
- **FACT**：新增 `tools/device-tests/23-desktop-multiapp.sh` 和 `make desktop-multiapp`。
  门禁要求桌面已经是 `ready`，然后通过真实 loader 并发启动三个 GUI 应用，检查：
  - 每个应用的真实进程仍存活且命令行匹配；
  - `DISPLAY=:0`；
  - `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/rt/bus`；
  - `/dev/mali0` 或 `/dev/kgsl-3d0` fd；
  - 已加载 `libvulkan_panfrost*` / `libvulkan_freedreno*`；
  - 没有 llvmpipe/swrast 软件覆盖；
  - 三个应用并发保持 `RUN_SECS` 后仍存活；
  - 第二张 raw 截图仍不是全黑或大块缺失。
- **FACT**：门禁只清理自己启动的三个 GUI PID，不重启或杀死其他 LinuxHub 包。
- **UNVERIFIED**：当前设备未进入桌面，门禁在健康门槛处返回 2，三个 GUI 的真实并发结果
  尚未取得。

## 修前问题

旧门禁无法回答以下问题：

```text
桌面已 ready 时，用户同时打开三个 GUI，是否仍然全部吃 GPU？
它们是否继承同一个 X11/DBus 会话，还是悄悄回到另一个环境？
运行 60 秒后是否只剩一个应用、或画面已经黑掉？
```

`15-gpu-global-audit.sh` 的 nested GTK 分支只测一个进程；`21-gpu-soak-audit.sh` 的四个
探针不连接 X11，也不创建窗口。

## 修复

新增设备门禁：

```sh
DEV=<serial> RUN_SECS=60 make desktop-multiapp
```

实现文件：

```text
tools/device-tests/23-desktop-multiapp.sh
```

门禁的进程识别不依赖启动瞬间的 wrapper PID。三个 GUI 可能是单实例程序，启动器可能把
窗口交给已有进程后自己退出；脚本等待 8 秒后从真实 `ps -A -o PID,ARGS` 重新解析活进程，
再对实际 PID 做 fd、maps 和环境检查。

## 验证

静态：

```text
sh -n tools/device-tests/23-desktop-multiapp.sh  PASS
make -n desktop-multiapp                         PASS
```

当前设备只读健康预检：

```text
load1=22.43
process/threads=31806
MemAvailable=3,268,356 kB
DESKTOP_MULTIAPP PASS=1 FAIL=2
exit=2 (BLOCKED: device health)
```

没有启动或安装任何新包。

## 待验证

- 三个 GUI 是否都能保持窗口和进程；
- 三进程是否同时持有 Mali fd 并加载硬件 Vulkan；
- 60 秒并发后 GPU/CPU/RSS/fd 的变化；
- 第二张截图是否保留所有窗口，且没有右侧/上方大块缺失；
- 应用退出后是否无僵尸、无残留窗口；
- 同一门禁在 Adreno/KGSL 真机上的结果。

## 边界

本次没有 bump `VERSION`、没有 tag、没有 Release。通过后只证明三个指定 GUI 的并发
路径，不自动证明所有 GTK/Qt/Electron 应用或所有 GPU 家族。
