# 300 · 第 6 轮（部分）：GPU 默认值对"非登录进程"生效，三进程并发仍是 zink

设备 `UKHMMB8PYTGECMNV`（Mali-G77 MC9 / Android 13 / 4.19.191），树
`files/linux/distributions/ubuntu-24.04/rootfs`（862 包，Mesa 25.2.8），loader `files/linux/husk/libld.so`。

> **更正（evidence 314）**：本文的 `PASS=17` 是旧 T3 负向套的输出。探针 PixelStore 常量错配修正后，同一读回套为 `PASS=19 FAIL=0`；此处的三进程并发结论仍有效，但没有按修正后的 19 项套重跑。

## 0. 结论

- **FACT**：不经登录 shell（`bash -c`，不读 `/etc/profile.d`）启动的进程，环境里已经有
  `VK_ICD_FILENAMES` / `VK_DRIVER_FILES`（指向 `husk_panvk_guest.json`）、
  `MESA_LOADER_DRIVER_OVERRIDE=zink`、`GALLIUM_DRIVER=zink`、`MESA_GLES_VERSION_OVERRIDE=3.2`、
  `PANVK_DXVK_COMPAT=0`、`GSK_RENDERER=cairo`。
- **FACT**：同一个进程里建 GL 上下文，拿到的是
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` / `OpenGL ES 3.2`，不是 llvmpipe。
- **FACT**：三个进程**同时**跑同一套读回测试，三个都是 zink/PanVK，各 `PASS=17 FAIL=0`。
- **UNVERIFIED**：整桌面 + 多应用同时渲染的长稳（本轮只做了同树三进程，没有开桌面会话）。

## 1. 为什么这条值得单独记

`/etc/profile.d` 只对登录 shell 生效。桌面会话、`env -i` 的子进程、服务管理器拉起的进程
都读不到，于是同一个 rootfs 里会出现"终端里的程序吃得到 GPU、别处掉回 llvmpipe"——
这正是第 6 轮要消灭的那类不一致。现在默认值的来源是加载器（`path-shim/src/gpuenv.rs`，
数据来自 guest 的 `/etc/husk/gpu-env`），所以只要进程经 Husk 加载器起，看到的就是同一份。

## 2. 量到的证据（FACT，原文）

```
$ husk-exec2 … usr/bin/bash -c 'env | grep -iE "VK_|GALLIUM|MESA_|GSK|PANVK"'
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
PANVK_DXVK_COMPAT=0
GSK_RENDERER=cairo

$ … usr/bin/bash -c 'python3 /tmp/readback-suite.py | head -1'
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
```

三进程并发（同一棵树，同一时刻起三个）：

```
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) … PASS=17 FAIL=0
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) … PASS=17 FAIL=0
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) … PASS=17 FAIL=0
```

树里的 `/etc/husk/gpu-env` 原文（安装器产出，9 行）：

```
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

## 3. `PANVK_DXVK_COMPAT` 这条边界的准确说法

PanVK 的 Bifrost/kbase 构建默认打开 DXVK 兼容的 `geometryShader` 路径，Zink 的固定管线
转译会撞进去，在 Mali-G77 上可能 SIGSEGV（evidence 273）。所以：

- **默认值**是 `PANVK_DXVK_COMPAT=0`，由 `gpu-env` 文件写、加载器注入（上表已证）；
- **调用方显式设 `=1` 会覆盖默认**，那是"已知会崩的那条路"，不是我们推荐的配置；
- 这条**不能用"加个开关"冒充修复**：它是**支持边界**——geometry shader 在本机走
  PanVK 时可用的前提是 `PANVK_DXVK_COMPAT=1`，而那是崩溃风险自负。

## 4. 边界（没测的照写）

- **Adreno / KGSL：UNKNOWN**。这台机器是 Mali-G77，没设备可验。
- **整桌面 + 多应用并发 + 长稳：UNVERIFIED**（本轮只到三进程）。
  2026-11-24 的 soak 数据在 `docs/design/evidence/soak-husk-runtime-20261124.csv`，
  但那是旧树旧默认值，不能拿来替这次的结论。
- X11 保持不动：本轮没有换合成器，也没有动 X server。

## 5. 复现

```sh
adb shell 'run-as com.linuxhub.husk sh files/linux/husk-exec2.sh \
  files/linux/distributions/ubuntu-24.04/rootfs files/linux/husk/libld.so \
  usr/bin/bash -c "env | grep -iE \"VK_|GALLIUM|MESA_|GSK|PANVK\""'
```
