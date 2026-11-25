# 182 · 图形默认值下沉到加载器：非登录执行也吃到 GPU

日期：2026-11-25 11:40–11:55（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13），
应用 `com.linuxhub.myth.husk`，loader `files/linux/husk/libld.so`（当日构建，738 816 B）。

## 为什么改

GPU 的默认值（选哪个 Vulkan ICD、GL 走 zink、GLES 上限）原来只写在
`/etc/profile.d/husk-gpu.sh` 里。**profile.d 只对登录 shell 生效**：cron、systemd user
unit、桌面 fork 出来的进程、`env -i`、`ssh host -- prog` 全都读不到，于是同一个 rootfs 里
会出现两套渲染器——终端里 zink，别处掉回 llvmpipe，而且没有任何报错。

这件事本身是通行做法（Termux 用 `MESA_LOADER_DRIVER_OVERRIDE`/`VK_ICD_FILENAMES`，
别的方案用 `env.sh` 或往登录环境注入），所以不是缺陷；但把默认值搬到**加载器**里之后，
"Husk 里的任何进程默认就吃 GPU" 这一条是别人没有的。

## 做法

1. `path-shim` 负责 `getenv` / `secure_getenv` / `__secure_getenv`（这是 libc 符号层，
   不是 syscall 拦截，程序看到的仍然是自己的环境）；
2. 查询顺序：**先 `environ`，查不到才落到 `/etc/husk/gpu-env`**——程序自己 export 的
   永远优先；
3. `/etc/husk/gpu-env` 由安装器写（`NAME=VALUE` 一行一个），ICD 文件名在安装时解析一次，
   Rust 里不硬编码；
4. 逃逸开关：`HUSK_GPU=llvmpipe` 或 `NR_GPU_DEFAULT_OFF=1` 时一条都不注入；
5. `secure_getenv` 尊重 `AT_SECURE`（setuid 起动一律返回 NULL），否则默认值会被带进
   本不该看环境的进程；
6. 文件不存在 / 树里没有 `husk_*.json` → 一条都不注入，行为与没有这个特性时完全一致。

为什么不是启动时 `setenv`：`_dl_init` 钩子跑在 `__libc_start_main` 之前，那时 `environ`
还是 NULL（`boot.rs` 里早就记着这一点），`setenv` 写进去的环境会被 libc 起动流程覆盖。

## 实测（全部 `env -i`，一条图形变量都不给）

调用方式：`env -i NR_ROOTFS=… NR_LD=… NR_LIBRARY_PATH=… HOME=/root PATH=… libld.so
--library-path … gl-probe es3`。这等价于「非登录执行看到的最小环境」。

| 场景 | RENDERER | 结果 |
|---|---|---|
| 没有 `/etc/husk/gpu-env`（改动前的行为） | `llvmpipe (LLVM 20.1.2, 128 bits)` | PASS |
| 有 `gpu-env` | `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` | PASS |
| `HUSK_GPU=llvmpipe` | `llvmpipe` | PASS |
| `NR_GPU_DEFAULT_OFF=1` | `llvmpipe` | PASS |
| 程序自己 `GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=swrast LIBGL_ALWAYS_SOFTWARE=1` | `llvmpipe` | PASS |
| 程序自己只设 `MESA_GLES_VERSION_OVERRIDE=3.1` | `zink …`，`GL_VERSION=OpenGL ES 3.1` | PASS |

最后一行的意义：只覆盖一个变量时，其余三个仍然来自加载器默认值——这正是 `getenv`
语义该有的样子，不是"整套环境被替换"。

`gpu-env` 内容（安装器写的就是这五行）：

```text
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

## 回归

同一棵树上跑门禁 A（36 项）：**PASS=36 FAIL=0**，`real SIGSYS traps (0)`、
`host paths only in internal NR_ vars (0)`。

## 不能说明什么

- 只在本机（Mali-G77 / Android 13 / 4 KiB 页）验过；高通、16K 页、Adreno 未验。
- **只对经过 Husk 加载器的进程生效**：静态链接的 ELF、以及应用侧 bionic 进程都看不到
  这份默认值（它们本来也读不到 profile.d，不是回归）。
- 桌面里的真实应用（Electron / WebKit）没重测，只验了 `gl-probe es3` 这条最小链路。
- `/etc/profile.d/husk-gpu.sh` 保留：登录 shell 仍然由它兜一次，两边值相同，先到先得。
