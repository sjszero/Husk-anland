# 291 · 加载器 GPU 默认值第一次真的生效：树里为什么"没有"，补回去之后量到什么

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / arm64。树：
`files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126`（945 包，Ubuntu 24.04.4，
Mesa 25.2.8-0ubuntu0.24.04.5）。

## 0 · 结论

- **FACT**：`/etc/husk/gpu-env` 是加载器给"非会话进程"补默认值的唯一来源
  （`runtime/crates/path-shim/src/gpuenv.rs`，`ENV_FILE=/etc/husk/gpu-env`；App 侧同一套决策在
  `runtime/crates/gpu-env/src/lib.rs` 的 `ensure_mali_defaults()`）。文件不存在＝一条都不注入，
  这些进程掉回 llvmpipe，**没有报错、没有日志**。
- **FACT**：安装器产出的树确实有这份文件。设备上两棵 base 树（`linux/…/rootfs` 93 包、
  `linux2/…/rootfs` 95 包）各有一份 **23 字节**的 `etc/husk/gpu-env`，内容只有
  `@GPU_FAMILY=mali-kbase` —— 因为那两棵树里没有 Mesa、没有 PanVK 驱动，安装器只记录家族，
  不写 ICD/GL 覆盖（`install-husk.sh` 的 `_husk_driver` 探测不到就 `unset`，见 2b-2c 段）。
- **FACT**：桌面树 `rootfs-broken-20261126` 在今天 19:37（evidence 289）和 19:44（290）两轮里
  既没有 `etc/husk/`，也没有 `usr/share/vulkan/`；同一棵树还缺 `etc/ld.so.husk-path`、
  `usr/local/share/linuxhub/`（系统总线配置）、`etc/profile.d` 的 husk 那几份 —— 也就是说
  **这棵树不是安装器产出的那棵树**，Husk 自己加的那一层整个不在，只剩 Ubuntu 包。
  `usr/share/locale-langpack/` 在，说明它来自我们自己的桌面 tar，但那份 tar 没带安装层。
- **FACT**（本轮补回并实测）：19:46 装回驱动与 ICD，19:49 按安装器同一套规则写 `etc/husk/gpu-env`，
  之后**不带任何 GPU 环境变量**、只经加载器启动的进程拿到
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`；`NR_GPU_DEFAULT_OFF=1` 与 `HUSK_GPU=llvmpipe`
  两个反向控制都掉回 `llvmpipe (LLVM 20.1.2)`。门禁 `18-gpu-loader-defaults.sh`
  **PASS=9 FAIL=0 SKIP=0**。
- **FACT**（新数字，2400×1080，12 帧，每帧 `glFinish`）：

  | 后端 | 渲染 | 单次整块 `glReadPixels W×H` | 逐行 `W×1` × 1080 |
  |---|---:|---:|---:|
  | zink（PanVK） | **2.36 ms**（0.91 ms/MP） | 35.53 ms（13.71 ms/MP） | **1344.11 ms**（518.56 ms/MP） |
  | llvmpipe | 6.06 ms（2.34 ms/MP） | 0.50 ms（0.19 ms/MP） | 7.07 ms（2.73 ms/MP） |

- **INFERENCE**：GPU 之前"一直是好的"不是因为注入一直在跑，而是因为**桌面会话脚本自己 export**
  （`tools/guest/husk-session.sh` 第 165–172 行：树里有
  `usr/share/vulkan/icd.d/husk_panvk_guest.json` 就把 `VK_ICD_FILENAMES`/zink 塞进 `env -i` 边界），
  会话 fork 出来的程序都继承。那条路要求同一份 ICD json —— 它在本树也是 19:46 才有的。
  所以之前那些 zink 记录（179/245/278）用的是**另一棵带 ICD 的树**，不是这一棵。
- **UNKNOWN**：那棵树具体在哪一步被换成现在这棵、旧树是否还在设备上，没有查到；树目录的
  top-level mtime（19:29）与树内文件 mtime（19:23 等）互相矛盾，像是 `cp -a` 出来的副本。

## 1 · 三条给 GPU 的路，只有第三条是"全局"

| 路径 | 谁设 | 谁吃得到 | 缺了会怎样 |
|---|---|---|---|
| `/etc/profile.d/husk-gpu.sh`（`tools/guest-config/husk-gpu-defaults.sh`） | 安装器写 | 登录 shell 及其子进程 | 非登录执行看不到，静默掉软件 |
| `husk-session.sh` / `husk-desktop.sh` 里的 `gpu_env=` | 会话脚本 | 该会话及 fork 出来的桌面/应用 | 会话外（cron、`sh -c` 再 exec、App 的独立 exec）掉软件 |
| **`/etc/husk/gpu-env` → 加载器 `getenv` 填空** | 安装器/App 写，加载器读 | **任何经 Husk 加载器起来的进程** | 整条链路静默失效，只剩前两条兜着 |

第三条是我们独有的那条（别家是 `env.sh` 注入登录环境、`MESA_LOADER_DRIVER_OVERRIDE` 手 export），
它的失效模式是"没有任何症状"：渲染器从 zink 变 llvmpipe，`glGetError` 仍 0，桌面照样出画，只是慢。
这轮就是栽在这个失效模式上——所以才把它做成门禁。

## 2 · 现场：三棵树逐文件（`run-as com.linuxhub.husk`，19:37–19:50）

```text
files/linux/distributions/ubuntu-24.04/rootfs               pkgs=93   gnome=no   gpuenv=yes(23B)  icd=none
files/linux/distributions/ubuntu-24.04/rootfs-broken-20261126 pkgs=945 gnome=yes gpuenv=no        icd=none(19:44) → 有(19:46)
files/linux2/distributions/ubuntu-24.04/rootfs              pkgs=95   gnome=no   gpuenv=yes(23B)  icd=none
```

桌面树缺的安装层文件（19:50 核）：

```text
MISS  etc/ld.so.husk-path
MISS  usr/local/share/linuxhub/linuxhub-system-bus.conf
MISS  usr/local/share/linuxhub/
MISS  etc/profile.d/zz-husk-env.sh
have  usr/share/locale-langpack/zh_CN/LC_MESSAGES/gtk30.mo
```

## 3 · 补回去做了什么

1. 驱动与 ICD（19:46，来自 `husk-panvk-builder` 容器 `frist-target-x11` 那份 X11 构建，
   `sha256 599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c`、16 056 688 字节）：

   ```text
   usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so
   usr/share/vulkan/icd.d/husk_panvk_guest.json   (api 1.3.0, library_path 为 guest 相对路径)
   ```

2. `etc/husk/gpu-env`（19:49，内容与 `install-husk.sh` 的 mali-kbase 分支逐行一致）：

   ```text
   @GPU_FAMILY=mali-kbase
   VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
   VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
   !MESA_LOADER_DRIVER_OVERRIDE=zink
   !GALLIUM_DRIVER=zink
   -LIBGL_ALWAYS_SOFTWARE
   MESA_GLES_VERSION_OVERRIDE=3.2
   PANVK_DXVK_COMPAT=0
   GSK_RENDERER=cairo
   ```

   `!` = 无条件覆盖（顶掉 2.0 会话硬塞的 swrast 默认值）、`-` = 让变量对程序不可见、
   无前缀 = 只在程序没设时填空。语义见 `gpuenv.rs` 文件头注释。

3. 门禁：`tools/device-tests/18-gpu-loader-defaults.sh`（本轮新增）——查树里驱动/文件名/
   manifest `library_path` 是否真能解析，然后跑三次真渲染：不带任何 GPU 变量、`NR_GPU_DEFAULT_OFF=1`、
   `HUSK_GPU=llvmpipe`。

## 4 · 门禁输出（19:52，真机）

```text
== tree ==
  PASS  tree carries a PanVK driver
  PASS  tree carries etc/husk/gpu-env
  PASS  gpu-env family line matches this device (mali-kbase)
  PASS  VK_ICD_FILENAMES points at a file inside the tree (/usr/share/vulkan/icd.d/husk_panvk_guest.json)
  PASS  manifest library_path resolves in the tree (/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so)
== push probe ==
  PASS  probe pushed
== loader defaults, no GPU env of any kind ==
  STATE plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
  PASS  a plain exec inherits hardware GL from the loader alone
== negative control: NR_GPU_DEFAULT_OFF=1 ==
  STATE off renderer: llvmpipe (LLVM 20.1.2, 128 bits)
  PASS  NR_GPU_DEFAULT_OFF=1 really turns the defaults off
== negative control: HUSK_GPU=llvmpipe ==
  STATE soft renderer: llvmpipe (LLVM 20.1.2, 128 bits)
  PASS  HUSK_GPU=llvmpipe wins over the file
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
```

启动时的两条 warning 是已知项，不是本轮新增：`kbase: KBASE_IOCTL_MEM_EXEC_INIT failed: EPERM`
（evidence 176，只挡 CPU 可执行映射）、`Mali-G77 MC9 doesn't support base Zink requirements:
feats.features.shaderClipDistance`（PanVK 走的是非 base 路径）。

## 5 · 顺带量到的三段代价（2400×1080，12 帧，每帧 `glFinish`）

```text
zink      COST render 2.36 ms  read-1call 35.53 ms  read-perrow 1344.11 ms
llvmpipe  COST render 6.06 ms  read-1call  0.50 ms  read-perrow    7.07 ms
```

三条推论：

1. **渲染确实是硬件在跑**：zink 2.36 ms vs llvmpipe 6.06 ms，2.6×（同一探针、同一棵树、
   同一帧内容，只有注入开关不同）。
2. **evidence 179/289 的"逐行读是绕法"只对 llvmpipe 成立**。zink 上每行一次 `glReadPixels`
   就是一次 Vulkan 同步，2400×1080 一帧 **1.34 秒**，比整块读慢 38 倍。工程口径要改成：
   逐行只在 llvmpipe 下用；zink 上要么整块读 + 自己按行距重排，要么走 PBO/分块，二选一得实测。
3. **zink 上"整块读快但错位、逐行读对但 1.3 秒"**——截图/录屏/像素比对现在没有一条便宜又正确的路。
   整块读 35.53 ms 与 evidence 174 的 30.27 ms 同量级（测法不同），所以 290 里"零拷贝能拿回
   30 ms"的方向不变，但代价要按 35 ms 记。

## 6 · 下一步

1. 加载器注入过了，但**没有在桌面会话里复跑**：`GNOME + gl-probe window 2400x1080 600`、
   多应用并发、长稳还没做（第 6 轮后半）。
2. zink 回读：先量 PBO + 分块是否绕开行距缺陷，再决定截图/录屏用哪条；不要先动 X server。
3. 桌面树的安装层缺失要变成可发现项：`18-gpu-loader-defaults.sh` 现在只查 GPU 那几份文件，
   `etc/ld.so.husk-path`、`usr/local/share/linuxhub/`、`etc/profile.d` 是否齐还没进门禁。
