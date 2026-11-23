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

## 2 先看前人怎么做

### proroot（`coderredlab/proroot`，闭源二进制，只审计不采用）

五个 `.so`：`libproroot.so`（launcher）/ `-runtime` / `-linker` / `-bridge` /
`-stub-loader`。README 自己写明是 **LD_PRELOAD** 路线。把
`libproroot-runtime.so` 的动态符号表拉出来（`llvm-nm -D --defined-only`，
**259 个定义**），它对我们这个问题的答案写在符号名里：

- 路径全家桶：`open open64 openat openat64 __open_2 __open_nocancel __open64_nocancel
  __openat_2 fopen fopen64 stat stat64 lstat lstat64 fstat fstat64 fstatat fstatat64
  statx __xstat __xstat64 __lxstat __lxstat64 __fxstat __fxstat64 statfs statfs64
  statvfs statvfs64 access faccessat readlink readlinkat __readlink_chk __readlinkat_chk
  realpath __realpath_chk opendir scandir scandir64 mkdir mkdirat rmdir unlink unlinkat
  rename renameat renameat2 symlink symlinkat link linkat chmod fchmodat chown lchown
  fchownat truncate utimensat utimes mkstemp* mkostemp* mkdtemp getxattr setxattr …`
- 动态加载本身：**`dlopen` `dlsym` `dl_iterate_phdr` `dladdr` `dladdr1` `dlinfo` `dlerror`**
- 还有 `proroot_patch_should_scan_object`、`proroot_resolve_linker_symbol`、
  `translate_path`、`detranslate_path` —— 即**每次 dlopen 之后重新扫一遍新出现的对象**。

结论（INFERENCE，依据是上面这些导出名 + README）：proroot 覆盖 dlopen 的办法是
「拦 dlopen → 调真的 → 对新对象再做一遍 patch」，而不是「拦每一个 syscall」。

### Shroot（`mekhontsev/shroot`，GPL-3.0，有源码）

`interception.md` 写的是 **选择性 seccomp TRACE + USER_NOTIF**，加一层
「lazy SVC 协作式改写」。它确实解决得最彻底（连 libc 内部直接调用都盖住），但：

1. 它要求宿主内核 **5.15 或更新**（README 的 Requirements），本机是 **4.19.191**；
2. 它是 syscall 拦截模型，与我们「运行时零 syscall 拦截、只在必要处 interpose」
   这条硬约束正面冲突。

所以 Shroot 只作为**语义对照**（它的 descriptor/inode store、mount-views 文档解释了
guest 文件名到宿主的映射该做到什么程度），不采纳它的机制。

### 顺带否掉的两条

- **整包 LD_PRELOAD 一个路径层**：可行（`v4l2-preload` / `drm-preload` 就是这个手法），
  但会得到**两份 shim 状态**（loader 里的那份 + preload 那份）：fd→guest 路径表、
  cwd、`/proc/self/fd` 仿真各记一份，跨层调用容易出现「A 打开的 fd 在 B 里查不到」。
  dynamic loader 的 preload 还要求每个入口都带上正确的 `LD_PRELOAD`（exec、posix_spawn、
  gdb、system() 全都要）。
- **按 `libc-interpose.md` 把 redirect 烘进 libc**：那条仍然要做，但它解决的是
  **libc 自己内部的直接调用**（没有 GOT 槽可改），不是 dlopen。两件不同的事。

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
