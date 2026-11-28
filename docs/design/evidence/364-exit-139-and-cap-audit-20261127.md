# 364 · 「GNOME 退出码 139」到底是谁：**实验补丁自己**；顺带把固定表上限审计了一遍

日期：2026-11-27 22:36–23:05（CST）。设备 RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
2400×1080，应用 `com.linuxhub.myth.husk`。承接 363（楔子机制）。

## 1 · 复现：一次「退出码 139」（FACT）

在 22:36 的会话里，从 guest 连开 12 个 `gnome-text-editor` 窗口，15 s 内桌面没了，
宿主界面打出 **「GNOME 已退出，退出码 139」**（图：`364-gnome-exit-139-20261127.png`）。
139 = 128+11 = SIGSEGV。

## 2 · 时间线把嫌疑人指出来（FACT）

| 时间 | 事件 |
|---|---|
| 22:29 | 给 `libgallium-27.0.0-devel.so` 打“zink 采样 LOD 钳 0”补丁（363 §4） |
| 22:30 | 给 `libmutter-14.so` 打“`meta_shaped_texture_set_create_mipmaps` → ret”补丁（363 §5） |
| 22:31 | **起会话**（`gnome-shell` 27496 那一代除外，见下）；此后应用盘截图正常 |
| 22:33 | 两个库**回滚成原文件**——但**运行中的进程仍映射着旧 inode**，回滚对当前会话无效 |
| 22:36–22:40 | 12 个窗口 → **139** |
| 22:44 | force-stop 重启会话 → 干净库 |
| 22:50 | 同样 12 个窗口：pid 前后都是 27496，**不崩** |
| 22:55 | 加到 26 个窗口 + 桌面塞 120 个图标：pid 仍是 27496，**不崩** |

⇒ **22:40 那次 139 发生在“带着我的两个补丁”的会话里，干净栈上同样的负载不复现。**
（INFERENCE，但时间线 + 同负载重测两边都指向同一结论。）

这条结论反过来也是 363 的补强：那两个补丁不只是“没用”，它们**有害**，回滚是对的。

## 3 · 顺便做掉：固定表上限审计（FACT）

用户问“是不是只设了 32 个”。把 `runtime/crates/**` 里所有固定上限数了一遍：

| 位置 | 上限 | 满了会怎样 |
|---|---|---|
| `path-shim/gpuenv.rs` `MAX_ENTRIES` | 12（**已改 128**） | 插入前判 `SLOT_N < MAX_ENTRIES`，满了就忽略，不越界 |
| `path-shim` `MNT_MAX` | 64 | 判 `MNT_N >= MNT_MAX` → ENOSPC（evidence 286 的老问题） |
| `path-shim` `SEEN_NR` | 24（**已改 128**） | 判 `c < SEEN_NR.len()`，满了只是少记一行 |
| `path-shim` `FAKE_GROUPS_MAX` | 32（**已改 128**） | 截断，不越界 |
| `path-shim` `fill_members` 的 `PTRS` | 32 | `k < 31` 后写 NULL，**截断**（组成员超过 31 个只显示前 31 个） |
| `v4l2-shim` | 4 设备 / 8 buffer / 16 size | 判越界后返回错误 |
| `drm-shim` | 192 prop / 256 bo | 同上 |
| `husk-init` `MAX_SERVICES` | 8 | 同上 |

**没有任何“进程数/窗口数 = 32”的上限**；12 个窗口就崩（§1）而 32 还差很远，所以
“32 太小导致崩”这个假设**不成立**（FACT：崩溃发生在 12，不发生在 26）。

按要求把能做大的三张表提到 **128**（`gpuenv` 12→128、`SEEN_NR` 24→128、`FAKE_GROUPS_MAX`
32→128）。`MNT_MAX` **保持 64**：`MNT_SAVE_MAX = 20480` 是按 `64 × 261` 算出来的，而序列化
缓冲在**栈上**（`let mut out = [0u8; MNT_SAVE_MAX]`），直接翻倍等于往栈上压 40 KB，
这是为了“修一个不存在的上限”去制造一个真问题，不做。

## 4 · 崩溃栈上线：下次 139 能直接说出是哪个库（FACT）

原来的 `on_segv` 用栈上的 224 字节缓冲写记录——可**栈溢出本身就会 SIGSEGV**，处理器在
已经耗尽的栈上再压栈，立刻二次崩溃，内核走默认动作，于是只看到 139、日志里**一个字都没有**
（本次 `nr-sigsys.log` 里 `segv|` 记录数 = 0）。改了三处：

1. `sigaltstack()` + `SA_ONSTACK`：处理器换到 32 KB 备用栈（`CRASH_STACK`）；
2. 记录改写到 `static mut` 缓冲（`CRASH_BUF`），不再用局部数组；
3. 记录里补 `sp` / `fp`，并把 `/proc/self/maps` 落到 `<rootfs>/tmp/nr-crash-<pid>.maps`
   ——没有 maps，一个落在 25 MB 库里的 `pc` 只是个数字。

记录格式：`segv|sig=|fa=|pc=|lr=|sp=|fp=|x0=|x1=|x2=|x8=|ld=|lc=`。
loader：`9a22094281c0eaa6…`（设备上 `files/linux/husk/libld.so`，旧件备份
`libld.so.bak-pre-crash` = `7970ce78…`）。
换 loader 时 `cp` 报 `Text file busy`（残留 guest 进程还映射着旧 inode），改成
`cp ... .new && mv -f` 原子替换，绕开 ETXTBSY。

自检：新 loader 起会话正常，桌面 26 窗口 + 120 图标不崩；`make check` 通过（374 md / 0 断链 / 0.0.37）。

## 5 · 仍然未知（UNVERIFIED）

- 用户历史上那些 139 的触发条件：**没复现**。“垃圾一多”具体指什么（窗口？桌面文件？
  常驻小程序？），需要一次现场描述或一次复现步骤。现在崩溃栈已经在位，只要能再现，
  就能直接读出 `pc` 落在哪个库、`sp` 是否贴着栈底（＝栈溢出）。
- 黑楔子（H49）仍未修，且已排除“采样到坏 mip”“窗口 mip 链”“图像内容”“ICD 差异”
  四条；剩下的方向在 363 §8。
