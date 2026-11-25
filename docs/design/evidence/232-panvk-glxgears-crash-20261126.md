# 232 · PanVK/Bifrost 编译器限制：`glxgears` 长跑在固定着色器路径崩溃

日期：2026-11-26 00:54–00:59（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页。
桌面包 `com.linuxhub.myth.husk` 正在运行，X11 root 为 2400×1080。测试只通过现有 App 树启动
guest 探针，没有修改 2.0 源码、没有替换桌面会话。

## 1 · 结论

| 用例 | 结果 | 证据等级 |
|---|---|---|
| 当前 loader，两路并行 `glxgears -info` | 两路都在 45 秒超时前退出，`RC=139`、`fa=0x21` | FACT，真机 |
| pre-229 loader，同一程序 | 同样 `RC=139`、`fa=0x21`，崩溃函数相同 | FACT，真机 |
| 当前 loader + Mesa 参数开关三组 | 三组都同样崩溃，参数没有改变结论 | FACT，真机 |
| llvmpipe 对照 | `RC=124`（预期的 20 秒超时），15 秒采样约 348–371 FPS | FACT，真机 |
| `gl-probe window` | PASS：60 次 swap，97.2 FPS，像素回读为绿色 | FACT，真机 |
| `gl-probe es3` | PASS：ES3 上下文、纹理、MRT、像素校验全过 | FACT，真机 |
| `vulkan-probe` | PASS：枚举 `Mali-G77 MC9`，buffer 创建/绑定/回读全过 | FACT，真机 |

因此不能写“GPU 10 分钟长稳通过”。当前能确认的是：Mali/PanVK 的常规探针和很多桌面路径仍然
可用，但 `glxgears` 走到的 Bifrost 着色器编译路径会稳定崩溃。它是 **0.0.13 的已知限制**，不是
已修项。

## 2 · 当前 loader 的双路复现

当前 loader 是 `files/linux/husk/libld.so`，md5 `c3156aee16176b1e63eaac7f3413de46`。
两路都通过 `xe.sh` 启动，外层加 45 秒 `timeout`，这样如果程序被外部超时终止会得到 124；
实际结果是 139，说明程序自己先死了：

```text
RC_A=139 RC_B=139

A:
GL_RENDERER   = zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
segv|sig=11|fa=21|pc=498026855504|lr=32|x0=0|x1=21|x2=0|x8=0|ld=498257682432|lc=0

B:
GL_RENDERER   = zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
segv|sig=11|fa=21|pc=487157775440|lr=32|x0=0|x1=21|x2=0|x8=0|ld=487388561408|lc=0
```

两台进程的 `ld` 地址不同，`pc - 0x38bc50` 得到的模块基址也不同（`0x73f47e0000` 与
`0x716ca50000`），但相对崩溃点完全一致。命令整体 4 秒内返回，两个进程都没有活到 45 秒。

## 3 · 符号定位

从设备拉出同一份
`usr/lib/aarch64-linux-gnu/libvulkan_panfrost_x11.so`（Build ID
`56271afde866d837c77baaa1f7ca2b56358c90ed`），用 LLVM nm 和 objdump 对照：

```text
000000000038bb30 t emit_cf_list.isra.0
```

`0x38bc50` 是该函数内的 `+0x120`。反汇编在这一点是：

```text
38bc50: f940039c  ldr x28, [x28]
```

崩溃处理器给出的故障地址是 `0x21`，寄存器 `x1=0x21`、`x28` 指向无效链表节点。二进制里同时
保留 `bifrost_compile.c` 字符串，所以可以把现象限定在 PanVK 的 Bifrost 编译器，而不是把
它归成“GPU 没起来”：同一进程已经完成 Vulkan 设备枚举和 Zink 渲染器初始化，错误发生在
着色器 NIR 列表的编译阶段。

## 4 · 参数开关与 pre-229 对照

为避免把某个缓存或优化选项误当成根因，分别跑：

```text
MESA_SHADER_CACHE_DISABLE=true  -> RC=139, segv|sig=11|fa=21
PANVK_DEBUG=noopt               -> RC=139, segv|sig=11|fa=21
MESA_GL_VERSION_OVERRIDE=4.6COMPAT -> RC=139, segv|sig=11|fa=21
```

三次的 `pc - 0x38bc50` 也都落在 `emit_cf_list.isra.0 + 0x120`。

再用 0.0.13 之前的 loader（md5 `5e6788f53799d45110ce599b256ed4f5`）跑同一个程序：

```text
RC=139
segv|sig=11|fa=21|pc=488240823376|lr=32|x0=0|x1=21|x2=0|x8=0|ld=488471638016|lc=0
```

相对点仍是 `emit_cf_list.isra.0 + 0x120`。这只能证明 **不是 229 的 BO/锁修复引入的回归**；
它不能证明旧 loader 完全无关，也不能证明某个 Mesa 版本一定没有这个问题。

## 5 · 软件对照和其余 GPU 冒烟

用 `HUSK_GPU=llvmpipe GALLIUM_DRIVER=llvmpipe LIBGL_ALWAYS_SOFTWARE=1` 跑同一程序：

```text
GL_RENDERER = llvmpipe (LLVM 20.1.2, 128 bits)
1838 frames in 5.0 seconds = 367.552 FPS
1857 frames in 5.0 seconds = 371.259 FPS
1742 frames in 5.0 seconds = 348.327 FPS
RC=124
```

`RC=124` 是外层 20 秒超时，程序没有崩。与此同时，当前 loader 的 GPU 探针仍为：

```text
gl-probe window: RESULT PASS, 60 swaps OK (617.5 ms, 97.2 fps)
gl-probe es3:    RESULT PASS, att0 异常数=0, MRT 校验=0
vulkan-probe:    PASS, physical=Mali-G77 MC9, buffer readback=0x4855534b
```

## 6 · 判断边界

- **FACT**：`glxgears` 在 zink → PanVK 上稳定触发 `SIGSEGV`，崩溃点是
  `libvulkan_panfrost_x11.so` 的 `emit_cf_list.isra.0+0x120`，故障地址 `0x21`。
- **FACT**：llvmpipe 同一程序没有崩，GPU 的 `gl-probe window`、ES3 workload、Vulkan probe 都
  通过。
- **FACT**：pre-229 loader 同样在相对同一点崩溃，因此不是 229 的 BO 生命周期或 `DRM_LOCK`
  修复引入的回归。
- **INFERENCE**：问题落在 Bifrost 编译器的 NIR 链表/指令发射路径，属于 Mesa/PanVK 层；不是
  “Mali 完全不可用”。
- **UNKNOWN**：具体是哪个 NIR 节点被破坏、是否会命中其他真实应用、以及需要哪一版 Mesa 修复。
  本轮没有改 PanVK 源码，也没有把 `glxgears` 的崩溃通过参数开关绕过去。

发布口径：0.0.13 可以说“常规 GPU 探针和桌面出帧通过”，不能说“GPU 长跑通过”或“所有 GPU
程序稳定”。`HUSK_GPU=llvmpipe` 仍可作为单个不兼容程序的回退，但不能把回退当成硬件加速修复。
