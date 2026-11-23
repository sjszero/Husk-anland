# evidence 96 — mutter 46.2 在 Husk loader 下跑起来；link_map 上限是真 bug

设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 1. loader 的 link_map 遍历上限卡死了大程序（FACT，已修）

`runtime/crates/path-shim/src/boot.rs` 里 GOT interpose 的遍历写成
`while l != 0 && guard < 64`。依赖链超过 64 个 DSO 的程序一律报
`nr-boot: link_map chain truncated` 并返回 70。mutter 的依赖远超 64。

修：上限改 4096（仍保留环检测作用）。回归：`bash -c 'echo bash-ok; head -2 /etc/os-release'`
正常，`PRETTY_NAME="Ubuntu 24.04.4 LTS"`。

## 2. mutter 的库不在默认库目录（FACT）

```
usr/lib/aarch64-linux-gnu/mutter-14/libmutter-cogl-14.so.0
```
`gexec2.sh` 的 `--library-path` 只有 `usr/lib/aarch64-linux-gnu`，所以
`usr/bin/mutter --version` 报 `libmutter-cogl-14.so.0: cannot open shared object file`。
补上 `mutter-14` 与 `gnome-shell` 两个目录后：

```
$ sh gexec3.sh rootfs-full usr/bin/mutter --version
mutter 46.2
```
（`tools/device-tests/gexec.sh` 是这份启动器的仓库副本。）

## 3. `mutter --headless` 的真实行为（FACT）

```
$ mutter --headless --display-server
Failed to configure: Can't run in display server mode headlessly
```
—— 与 `src/core/meta-context-main.c:138` 的检查一致，是设计如此，不是我们的问题。

```
$ mutter --headless
libmutter-Message: Running Mutter (using mutter 46.2) as a Wayland display server
libmutter-WARNING: Failed to make thread 'KMS thread' realtime scheduled:
                   Failed to acquire RTKit D-Bus proxy (no such file or directory)
libmutter-Message: Created surfaceless renderer without GPU
Failed to setup: Unable to initialize the Clutter backend: no available drivers found.
```
结论：
- mutter 本身能初始化到 Wayland display server 阶段（D-Bus/RTKit 警告不是致命的）。
- 卡在 **Clutter 后端拿不到 GL 驱动**：headless 走 surfaceless renderer，但没有把我们的
  DRM 语义层（`/dev/dri/renderD128` → `/dev/mali0` kbase）和 Mesa 驱动喂给它。
- 这正是显示路线 B 剩下的那一段（见 `device-interface-choices.md` §5）。

## 4. 结论

- "GNOME 起不来" 不是 mutter 的问题，先是我们 loader 的 64 个 DSO 上限，再是库里没带对
  路径，最后才是 EGL/DRM 这一层。前两个已修，可复现。
- 下一步（显示路线 B）：让 mutter 看到 `renderD128` + Mesa（`MESA_LOADER_DRIVER_OVERRIDE`
  / `LIBGL_DRIVERS_PATH` / `GBM_BACKEND`），再看 `--headless` 能否拿到 GPU 渲染器；
  拿到后再接 ScreenCast→dmabuf 与 RemoteDesktop/EIS 输入。
