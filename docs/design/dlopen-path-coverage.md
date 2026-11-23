# dlopen 进来的库也要走 guest 路径：只看前人怎么做，然后实现

> 状态：**已实现并在设备上验证**（evidence 100）。
> 触发原因：evidence 99 —— GTK 客户端与 GNOME Shell 起不完整，xkbcommon / gdk-pixbuf
> loader / libspa / gst 插件这些**运行时 dlopen** 的库里的 `open()` 走宿主路径。

## 1 问题

loader 里的 path-shim 在 `_dl_init` 时按对象遍历 link map、改写每个对象的 GOT 槽
（`boot.rs` 第 2 步）。`_dl_init` 之后 `dlopen` 进来的对象不在那次遍历里，它们的
PLT 由动态链接器就近解析到 libc，路径没有被改写。

被这一步挡住的东西：gdk-pixbuf 的 image loader、GIO module、libspa 插件、
GStreamer 插件、PAM/NSS module、GTK 的 immodule —— 全是插件式组件。

## 2 结论先写

覆盖 dlopen 的对象，只有两个方向：

1. **拦 `dlopen` 本身**：调真正的 `dlopen`，拿到新对象后，只对它做一遍 GOT 改写。
   不用拦任何 syscall，成本只在加载那一刻。
2. 拦每个 syscall —— 与「运行时零 syscall 拦截」的硬约束冲突，直接排除。

选 1。实现见 §3。

## 3 采用的实现

`boot.rs` 新增 `nr_repatch()`：

1. 从 `MAIN_MAP` 重新走一遍 link map（glibc 把新对象**追加**到同一条链上，
   所以从链头出发一定看得见它们），上限 4096 个对象；
2. 只对**没见过**的对象跑 `patch_object()`（按 load bias 记在 1024 项的表里），
   已 patch 过的跳过 —— 一次 dlopen 的代价是「扫一遍链 + 只 patch 新对象」；
3. 重入保护（`REPATCH_LOCK`），返回码与 `nr_boot` 一致（70 = 链被截断）。

`lib.rs` 的 `dlopen()` 三个返回点都改成 `loaded(dlmopen(...))`：非空句柄才 repatch。

**为什么这样比 preload 好**：状态只有一份（还是 loader 里那份），不依赖
`LD_PRELOAD` 穿过 exec/spawn/system，也不需要给每个入口打补丁。

## 4 已知边界（写清楚，不假装没有）

- **对象自己的构造函数**在 `dlopen` 返回**之前**就跑完了。构造函数里如果 `open()`
  guest 路径，这一次仍然走宿主。实测 gdk-pixbuf / GIO / libspa 的模块构造函数都不碰
  文件；以后真撞上，再考虑 LD_AUDIT（`la_objopen` + `la_symbind64`，glibc 官方
  给审计器准备的挂钩，不拦 syscall，也不改 selinux 域）。
- **libc 内部直接调用**（`__open_nocancel` 这类同编译单元里的 `bl`）仍然无 GOT 槽可改，
  继续由 `libc-interpose.md` 的构建期槽位方案解决；两者互补，不替代。
- **`dlmopen` 到其它 namespace**：本仓只处理 `LM_ID_BASE`；不拦 `dlmopen` 本身，
  第三方 namespace 里的对象不在这次 repatch 的范围（本机没有这种用法）。
- patch 是**改写 GOT 槽**，对 `RTLD_DEEPBIND` 加载的对象无效（它把 libc 的符号
  绑在自己作用域里）。没观察到本机有这种用法。

## 5 验证

- 机制级：dlopen 进来的 `.so` 里 `fopen("/etc/os-release")` —— 旧 loader `rc=-1`，
  新 loader `rc=0` 读到 `PRETTY_NAME="Ubuntu 24.04.4 LTS"`（evidence 100）。
- 组件级：dlopen 进来的 `libglib-2.0.so.0` 用 `g_file_get_contents()` 读 guest 图标，
  返回 1、长度 717 与 `ls -l` 一致（同上）。
- 计数：`NR_BOOT_DEBUG=1` 打出 `nr-repatch: call=0x1 objects=0x5 new=0x5 slots=0x1 failed=0`。

## 6 这一层**没有**解决的东西（下一次不要重复归因）

`gdk_pixbuf_new_from_file()` 在本机仍然报 *Couldn't recognize the image file format*，
但**用宿主绝对路径也一样报**，所以与路径层无关（evidence 100 的对照实验）。同时：
rootfs 里 **没有 `loaders.cache`**，14 个 loader 模块一个都没注册（只剩 libgdk_pixbuf
内置的 png/jpeg）。这两件事要单独查，不能算到 dlopen 路径层头上。
