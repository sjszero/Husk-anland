# 292 · GPU 并发长稳（无 X）与"桌面没有 X server"这条硬阻塞

日期：2026-11-26 19:55–20:05（CST）。设备 RMX3357 / MT6893 / Mali-G77 MC9 /
Android 13 / 4.19.191 / 4 KiB 页。树
`files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126`（945 包）。
注入路径是加载器读 `etc/husk/gpu-env`（evidence 291），运行命令里不带任何 GPU 环境变量。

## 0 · 结论

| 项 | 结果 | 等级 |
|---|---|---|
| 4 路并发、每路 300 s、2400×1080 clear+`glFinish` | 每路 **579 fps**（峰值 606–615），合计 ≈2 316 fps | FACT |
| 同上，每路的 RSS / fd / `/dev/mali0` fd | 起止一致（RSS 373 MB→373 MB 上下浮动 1 MB，fd 7→7，mali 2→2），GL 错误 0 | FACT |
| 单进程固定管线（即时模式，无光照） | `zink` 渲染器，**554 fps**，0 错误 | FACT |
| 单进程固定管线（光照 + 平滑着色 + 深度测试） | **320 fps**，0 错误 | FACT |
| 上面两个用例带 `PANVK_DXVK_COMPAT=1` | 也全部跑完（566 fps / 323 fps，0 错误） | FACT |
| 桌面 GNOME 会话里的 zink | **本应用起不来**：没有 X server | FACT（见 §3） |
| 其他 GPU 家族 / 16 KiB 页 | 无设备 | UNKNOWN |

## 1 · 并发与长稳（第 6 轮"多应用并发 + 长稳"）

四路同一秒启动，各自 EGL surfaceless + ES3 上下文 + 2400×1080 FBO，每秒一帧 `glClear`
加 `glFinish`：

```text
DONE p1 frames=173746 avg_fps=579.1 peak_fps=606 rss=372672kB (start 373272kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0
DONE p2 frames=173767 avg_fps=579.2 peak_fps=611 rss=373376kB (start 373736kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0
DONE p3 frames=173893 avg_fps=579.6 peak_fps=615 rss=372572kB (start 372932kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0
DONE p4 frames=174164 avg_fps=580.5 peak_fps=614 rss=373440kB (start 373800kB) fd=7 (start 7) mali_fd=2 (start 2) errors=0
```

- **FACT**：四路并发时每路仍是 579 fps，和单路（412 fps 的 12 s 短测、579 fps 的长测帧数不同
  是因为短测里每帧多算了 `glGetError`）同量级，没有出现"第二个上下文把第一个拖垮"。
- **FACT**：300 秒里句柄和内存都不涨。这条比帧率重要：kbase 上最容易出的是 BO 缓存和
  dma-buf 引用泄漏，肉眼看不出来，只有 fd 计数和 RSS 会露。
- 探针：`tools/device-tests/probes/glsoak.py`（纯 ctypes，无仓库代码依赖）。

## 2 · 固定管线边界：开关既不是病因也不是解药

`glxgears` 在这台设备上崩过（evidence 232：`RC=139`、`fa=0x21`、`emit_cf_list.isra.0+0x120`），
当时三组 Mesa 参数都没改变结论；evidence 244 用 `gl-probe fixed` 在无 X 的情况下复现了同一路径，
并给出"`PANVK_DXVK_COMPAT=0` 时 600 帧通过、`=1` 时仍崩"。本轮用独立探针
（`tools/device-tests/probes/fixedgl.py`，ctypes + 即时模式）重新分档：

| 用例 | `PANVK_DXVK_COMPAT=0` | `PANVK_DXVK_COMPAT=1` |
|---|---|---|
| 一个三角面（`plain`） | 554 fps，0 错误 | 566 fps，0 错误 |
| 60 个三角面 + 光照 + 平滑着色 + 深度测试（`lit`） | 320 fps，0 错误 | 323 fps，0 错误 |

- **FACT**：在当前装回来的 FristOneRR X11 构建上，这两个即时模式用例**两个取值都通过**。
- **INFERENCE**：所以"把 `PANVK_DXVK_COMPAT` 设成 0"不是对 `glxgears` 那类崩溃的修复，
  它只影响用户请求 geometry shader 时的后端行为；不能拿这个开关冒充修复，也不能写成
  "打开开关就崩 / 关掉就好"。真正的支持边界是：**这台设备的 PanVK/Bifrost 编译器对
  `glxgears` 那类固定管线程序仍会崩，触发条件没被缩到一条规则上**。
- **UNKNOWN**：`glxgears` 在今天的树 + 今天的驱动上是否还是 `RC=139` —— 它要 X server，
  而本应用现在没有（§3）。不能沿用 232 的结论当"今天仍然如此"。

## 3 · 桌面这条硬阻塞：本应用没有 X server

`husk-desktop.sh` 的头部写着"X server 由应用进程持有，本脚本不重启它"。本轮核了这句话到底靠什么：

**FACT（APK）**：`app-debug.apk` 里只有三个 `.so` —— `libhuskapp.so`、`libld.so`、
`libhuskguard.so`，没有任何 X server 二进制，`assets/` 里也没有。

**FACT（树）**：945 包的桌面树里有完整的 X 客户端（`xterm`、`xeglgears`、`xfce4-*`、
`Xwayland`），但 `usr/bin/Xorg` 不存在，`usr/lib/xorg/` 里只有一个 `protocol.txt`。

**FACT（设备上唯一的 X server）**：`com.linuxhub.myth.husk` 的 `:x11` 进程（pid 28083）在跑，监听

```text
abstract @/data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/tmp/.X11-unix/X0
file     /data/user/0/com.linuxhub.myth.husk/.../rootfs/tmp/.X11-unix/X0
```

**FACT（能不能借用）**：从 `com.linuxhub.husk` 的 uid、经 Husk 加载器起的 python 连那个
abstract socket，X11 握手前就失败：

```text
CONNECT FAIL [Errno 13] Permission denied
```

**INFERENCE**：这是 Android 的跨应用 unix socket 限制（`untrusted_app` 之间），不是路径层
能绕的：抽象命名空间本来全机共享，被拒说明是 SELinux 策略拦的。所以"借 2.0 的 X server"这条路
对发布形态不成立。

**结论（FACT）**：`com.linuxhub.husk` 现在**没有可用的 X server**，`husk-desktop.sh` 一跑就会在
"等 X"那一步失败。这不是本轮引入的，是从来没被写进验收：以前那些"桌面 zink / GLX 600 秒"
记录（evidence 245/278）用的就是 2.0 包的 `:x11`，245 的文件头自己写着
"测试对象是现有 2.0 包 `com.linuxhub.myth.husk` 的 X server（`:0`）"。

**要满足第 6 轮的"验证桌面 + 多应用并发 + 长稳"，必须先有 X server**，两条路：

1. **把 X server 搬进 guest**（evidence 228 的建议）：Ubuntu 的 `xserver-xorg-core` 进树，
   走 shim 的 SysV IPC，MIT-SHM 才有意义；代价是它要一个能出图的 DDX，而本机没有 render node。
2. **在 APK 里带一个 bionic X server**（Termux:X11 那类），由应用进程持有、在应用的
   `/dev/mali0` 权限下渲染，guest 通过 shim 连它。这条能保住"保持 X11、不换合成器"，
   代价是一个新的原生组件和一套输入/呈现接线。

两条都还没做。这一条现在是第 6 轮和"完整 Linux 运行环境"之间最大的空白，写在这里而不是
留在脑子里。

## 4 · 本轮同时补的安装层缺件

见 evidence 291 §2：桌面树缺 `etc/husk/`、`etc/profile.d/husk-*`、`usr/local/share/linuxhub/`。
本轮补进去并验收：

```text
TREE_INTEGRITY PASS=11 FAIL=0     （EXPECT=desktop，当前桌面树）
TREE_INTEGRITY PASS=7  FAIL=0     （EXPECT=base，0.0.36 干净树）
```

新增门禁 `tools/device-tests/20-tree-integrity.sh`，模板在 `tools/guest-config/linuxhub-system-bus.conf`，
安装器现在会写 `usr/local/share/linuxhub/linuxhub-system-bus.conf`（以前这文件只在 2.0 的树里
存在，Husk 自己的树没有 → `husk-session.sh` 静默跳过启动系统总线）。系统总线本身已经实测：

```text
daemon=12327
method return ... sender=org.freedesktop.DBus -> destination=:1.0 serial=3 reply_serial=2
   array [ string "org.freedesktop.DBus"  string ":1.0" ]
```

## 5 · 下一步

1. X server 二选一（§3），做完才能验桌面；在那之前"桌面 GPU 已通"的记录必须挂着
   "用的是 2.0 的 X server"这条限定。
2. `glxgears` 在有 X 之后重测一次，把 232 的结论从"当前"降级为"当时的构建"。
3. Adreno/KGSL 仍然没有设备（UNKNOWN）。
