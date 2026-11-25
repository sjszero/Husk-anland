# 233 · Adreno/KGSL 适配器代码落地与 Mali 回归

日期：2026-11-26 01:10–01:30（CST）。测试设备：RMX3357 / Android 13 / aarch64；
本机 GPU 为 Mali-G77 MC9，**没有 Adreno 设备**。本轮只验证了代码边界和 Mali 无回归，
不能把 Adreno 写成已支持。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| Adreno/KGSL 适配器代码 | 已实现：真实 KGSL fd、版本名、fd 继承、stat/statx/uevent、node type | FACT，静态代码 |
| `gpu-probe adreno` 在当前 Mali 机 | `SKIP /dev/kgsl-3d0 missing`，没有假装成功 | FACT，真机 |
| Mali 门禁 A | `PASS=38 FAIL=0` | FACT，真机 |
| Mali `vulkan-probe` | 枚举 `Mali-G77 MC9`，buffer 创建/绑定/回读通过 | FACT，真机 |
| Mali `glxinfo -B` | `direct rendering: Yes`，`zink Vulkan 1.3(Mali-G77 MC9)` | FACT，真机 |
| Mali 桌面用新 loader 重启并保持 | App 入口进入桌面；`gnome-shell` 与 `mutter-x11-fram` 的 exe 都是当前 `libld.so`，maps 中有 `/dev/mali0`，截图右侧无黑带 | FACT，真机 |
| `drmGetNodeTypeFromFd` 实际接入 | `libva-drm` 的 PLT 调用经改写后的 GOT 命中 shim，`vaGetDisplayDRM` 返回非空 | FACT，真机 |
| `@GPU_FAMILY` 不匹配 | rootfs 标 `adreno-kgsl`、当前 Mali 机，`gl-probe` 回退 `llvmpipe` | FACT，真机 |
| Adreno 打开 KGSL | **UNKNOWN**，没有设备 | 未验证 |
| Turnip 枚举 / KGSL ioctl / mmap / Adreno 桌面 | **UNKNOWN**，没有设备 | 未验证 |

设备上的 loader：

```text
files/linux/husk/libld.so
md5    0951bf75e5ccb7656b6f614809965dc7
sha256 9531397561f0075185be810b98cb161d3a8f62205a5c34b3a5afff826a6e6a67
```

## 2 · 实现的边界

代码落点：

- `runtime/crates/path-shim/src/plan.rs`
- `runtime/crates/path-shim/src/gpuenv.rs`
- `runtime/crates/path-shim/src/lib.rs`
- `tools/guest-config/husk-gpu-defaults.sh`
- `tools/guest/install-husk.sh`
- `runtime/crates/gpu-probe/src/main.rs`（新增 `gpu-probe adreno`）

行为：

1. `/dev/kgsl-3d0` 可读写时，设备族是 `adreno-kgsl`，不会再返回
   “adapter not implemented”。
2. guest 打开 `/dev/dri/renderD128`（或 `card0`）时，实际打开真实
   `/dev/kgsl-3d0`，返回的是 KGSL fd；不会进入 Mali 的 `DRM_CTX`/`FAKE_FDS`。
3. `DRM_IOCTL_VERSION` 返回驱动名 `kgsl`；KGSL 的其他 ioctl 和 mmap 原样直通。
4. `dup`/`dup2`/`dup3`/`fcntl(F_DUPFD*)` 继承追踪，`close`/`close_range`
   清理；每次版本查询还用实时 `st_rdev` 验证 fd，避免 fd 号复用误判。
5. render node 的 `stat`/`statx` 报真实 KGSL 设备号；`/sys/dev/char/.../uevent`
   报 `DRIVER=kgsl`、`qcom,kgsl-3d0`；`drmGetNodeTypeFromFd` 对 KGSL 返回
   `DRM_NODE_RENDER`。
6. 安装器写 `/etc/husk/gpu-env` 的第一行带 `@GPU_FAMILY=`。rootfs 被搬到另一个
   GPU 家族时，加载器不会注入旧家族的 ICD/驱动。
7. Adreno 的登录 shell 配置优先找 Turnip/freedreno ICD；存在原生
   `kgsl_dri.so`/`freedreno_dri.so` 时走原生 Gallium，否则走 Zink → Turnip 的硬件路径。
   没有匹配驱动时明确退到 llvmpipe，不伪造硬件。

## 3 · 当前 Mali 回归命令与输出

门禁 A：

```text
DEV=UKHMMB8PYTGECMNV \
PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs \
LD=files/linux/husk/libld.so \
EXPECT_PKGS=811 \
sh tools/device-tests/14-gate-a-acceptance.sh
```

关键输出：

```text
PASS  dpkg -C clean after (0)
PASS  no host system processes visible (0)
PASS  /proc/self/exe has no app dir (clean)
PASS  real SIGSYS traps (0)
PASS=38 FAIL=0
```

Vulkan：

```text
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|device|PASS|VkResult=0
vk|buffer|PASS|created/bound/mapped size=4096 type=2 readback=0x4855534b
```

GL：

```text
direct rendering: Yes
Accelerated: yes
OpenGL renderer string: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile version string: OpenGL ES 3.2
```

新 loader 的桌面重启（`husk-desktop.sh`）自检：

```text
CHECK wm=yes
CHECK ding=yes
CHECK gpu=zink
CHECK input=primed
CHECK im=fcitx5
```

从 App 入口重新进入桌面后，PID 22788 `gnome-shell` 的 exe 是当前 `libld.so`，不是 `(deleted)`；
PID 23090 `mutter-x11-fram` 同样使用当前 loader。`gnome-shell` maps 中有多段 `/dev/mali0`：

```text
5fca200000-5fca400000 rw-s ... /dev/mali0
70c6702000-70c6729000 r-xp ... /files/linux/husk/libld.so
```

该进程的原始环境里仍能看到
上游会话留下的 `MESA_LOADER_DRIVER_OVERRIDE=swrast` 和 `GALLIUM_DRIVER=llvmpipe`，
这只是 exec 保留的字符串；实际加载的是 PanVK，不能拿 `/proc/.../environ` 单独判渲染器：

```text
libld.so (current path, not deleted)
/dev/mali0  rw-s ...
MESA: warning: kbase: ... 
WARNING: panvk is not a conformant Vulkan implementation
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
```

会话内 `glxinfo -B` 仍为：

```text
direct rendering: Yes
    Accelerated: yes
OpenGL renderer string: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
OpenGL ES profile version string: OpenGL ES 3.2 Mesa 27.0.0-devel
```

截图：[`233-desktop-after-adreno-loader.png`](233-desktop-after-adreno-loader.png)。按右侧 220 px 宽、
顶部 20 px 以下做像素检查，`220×1060` 区域黑像素为 `0/233200`；之前出现过的右侧黑带
本次没有复现。截图是界面证据，不等于 Adreno 路径证据。

完整 guest 工具构建：

```text
husk-usermgr  entry 0x2A3C0
husk-ldconfig  entry 0x272C0
husk-rd       entry 0x29DC0
husk-su       entry 0x25640
husk-symlinks entry 0x26000
husk-window-wait entry 0x27180
gl-probe      entry 0x28280
ipc-probe     entry 0x26900
gpu-probe     entry 0x2B640
```

`gpu-probe` 新增 `va-node`：它通过 `dlopen` 加载真实的 `libva-drm.so.2`，再调用
`vaGetDisplayDRM(fd)`。`libva-drm` 内部走 `drmGetNodeTypeFromFd@plt`，所以这条输出直接验证
第三方库的 GOT 是否被改写，而不是用 `dlsym(RTLD_DEFAULT)` 硬找 loader 内部符号：

```text
gpu|va_node_type|PASS|vaGetDisplayDRM returned a display for the guest render node
```

静态接线也一致：`libpath_shim.so` 的 `.dynsym` 有 `GLOBAL FUNC drmGetNodeTypeFromFd`，
`tools/loader/ld-embedded.so` 的字符串表中能看到该名字；`nr-tools embed` 会把所有导出的
全局函数加入 interpose 表。

Adreno 探针在当前 Mali 机上的结果：

```text
gpu|adreno_kgsl|SKIP|/dev/kgsl-3d0 missing (not an Adreno/KGSL device)
```

这条 `SKIP` 是预期的：当前设备没有 KGSL，不能把 Mali 的成功外推到 Adreno。

`@GPU_FAMILY` 的两个方向也在当前 Mali 机上实测。原文件先备份，测完恢复并核对 sha256：

```text
@GPU_FAMILY=mali-kbase
GL_VERSION=... RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) ...
RESULT PASS

@GPU_FAMILY=adreno-kgsl
GL_VERSION=... RENDERER=llvmpipe (LLVM 20.1.2, 128 bits) ...
RESULT PASS

恢复后 sha256=7680600c4177052dd9177ee2583100c5265062f803897c0862402f3862e259d9
```

第二组不是“Adreno 失败”，而是配置隔离生效：Mali 机看到 Adreno 标记后没有注入 Mali 的
ICD/GL 覆盖，按发行版默认落到 llvmpipe。

## 4 · 到 Adreno 真机必须补验的项

在 Adreno 设备上把 `gpu-probe` 放进 guest，执行：

```sh
tmp/gpu-probe adreno
```

它只做可逆的读取和最小 DRM 版本查询，断言：

- `/dev/kgsl-3d0` 和 guest `/dev/dri/renderD128` 能打开；
- 两个 fd 的 `st_dev`/`st_rdev` 一致，证明 render node 是真实 KGSL；
- `DRM_IOCTL_VERSION` 返回 `kgsl`；
- `dup` 后的 fd 仍返回 `kgsl`；
- `/proc/self/fd/N` 指向 `/dev/kgsl-3d0`。

`drmGetNodeTypeFromFd` 的 interpose 已在 Mali 上通过 `gpu-probe va-node` 走
`libva-drm` 的 PLT 实测；Adreno 仍需跑同一探针，另外验证 KGSL fd 的
`st_rdev`/`DRM_IOCTL_VERSION`、Turnip 枚举和桌面。Mali 上这条通过只证明“GOT 改写与
VA 客户端调用链成立”，不证明 Adreno 硬件链路成立。

以下仍必须在真机另行测，探针通过不等于桌面通过：

- Turnip 能枚举 Adreno 设备；
- KGSL 的 `k` ioctl 和 mmap 在真实应用域可用；
- X11 窗口分配、swap 与桌面首帧；
- 呈现路径是否仍需要 GPU→CPU 回读；
- 长跑和不同 Adreno/Android 内核组合。

## 5 · 判断边界

- **FACT**：当前 Mali 设备上，sha256 `95313975…e6a67` 的新 loader 门禁 A 38/38、Vulkan/GL 探针通过，桌面 `gnome-shell` 实际映射该文件。
- **FACT**：`/dev/kgsl-3d0` 不存在时，Adreno 探针输出 `SKIP`，没有伪造硬件成功。
- **FACT**：Mali 上 `gpu-probe va-node` 经由 `libva-drm` 调用链拿到非空 display，证明该符号确实被 loader interpose。
- **INFERENCE**：代码已经把 KGSL 的关键接口接到真实 fd；但链路是否在 Adreno 上成立，
  必须由真机输出回答。
- **UNKNOWN**：Adreno/KGSL 真机的打开、Turnip、ioctl、mmap、桌面和长跑。
- **FACT（新发现的会话边界）**：`am force-stop` 不会回收已经 setsid 的 guest 子进程；本轮先人工杀掉旧
  `libld.so` 进程再启动，才得到不指向 `(deleted)` 的新会话。这个生命周期问题还未修，不能把
  “App 重启”当成天然会终止旧桌面会话。
