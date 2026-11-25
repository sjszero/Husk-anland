# 244 · fixed-function OpenGL 的 PanVK GS 崩溃与 loader 环境物化

日期：2026-11-26 03:30–03:35（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 /
4 KiB 页 / Mali-G77 MC9。测试包 `com.linuxhub.husk`，rootfs `files/rootfs-full`。
本轮没有启动、停止或替换 `com.linuxhub.myth.husk` 的会话。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| fixed-function 复现不再依赖 X11 | 新增 `gl-probe fixed`，用 surfaceless EGL + desktop GL 固定管线画齿轮 | FACT，源码 + 真机 |
| 默认环境 600 帧 | 最终 APK loader `RESULT PASS`，588.5 fps，像素 `255,214,57,255` | FACT，真机 |
| 显式 `PANVK_DXVK_COMPAT=1` | 仍在 `fa=0x21` 以 `SIGSEGV` 退出，`RC=139` | FACT，真机 |
| 嵌套 `execve` 后环境可见 | `PANVK_DXVK_COMPAT=0`、Zink、GLES 3.2、canonical ICD 均在子进程 environ 中 | FACT，真机 |
| ES3 回归 | 纹理、MRT 两附件、逐像素校验全部通过 | FACT，真机 |
| Vulkan 回归 | 枚举 `Mali-G77 MC9`，compute、graphics、buffer 回读全部通过 | FACT，真机 |
| 真实 X11 `glxgears` / 桌面长跑 | 本轮未验证 | UNKNOWN |
| Adreno/KGSL | 仍无真机，打开、枚举、桌面和长跑均未验证 | UNKNOWN |

## 2 · 复现与根因

上一轮已把 `glxgears` 的崩溃缩小到 `libvulkan_panfrost_x11.so` 的
`emit_cf_list.isra.0+0x120`，故障地址 `fa=0x21`（evidence 232）。本轮新增的
`gl-probe fixed` 使用 desktop OpenGL 2.1 固定管线，不启动 X server，仍然复现同一故障；
因此崩溃发生在 fixed-function 转译后的 PanVK/Bifrost 着色器编译路径，不是 X11 窗口或呈现层。

当前 PanVK 驱动构造器会执行等价于

```c
setenv("PANVK_DXVK_COMPAT", "1", 0);
```

的默认设置。该兼容开关向 Zink 暴露 geometryShader；Zink 为固定管线四边形生成内部 GS，
当前 Bifrost 后端在这条路径上稳定崩溃。**这是本轮采用“默认关闭 DXVK 兼容 GS”的依据；
INFERENCE：具体损坏的 NIR 节点仍要由 PanVK/Mesa 上游定位。**

只拦截 `getenv` 不够：PanVK 构造器读取的是 libc 维护的真实 `environ`，而
`setenv(..., overwrite=0)` 也不会经过 `getenv` GOT。Husk 因此做了两层修复：

1. path-shim 拦截 `setenv`，让 `/etc/husk/gpu-env` 的 fill/force/hide 规则同样约束
   `setenv(..., 0)`；显式 `overwrite=1` 仍能覆盖 fill 默认值。
2. `execve` 前把 fill/force/hide 规则物化进传给子进程的 `envp`。这样程序自己的显式值优先，
   forced/hidden 规则保持原语义，且 `execve(..., envp)` 不会把 loader 默认值丢掉。

默认值同时写入 App 的 `gpu-env` crate、安装器和登录 shell；已有配置只补缺失项，
用户显式写的 `PANVK_DXVK_COMPAT=1` 不会被迁移覆盖。

## 3 · 构建标识（FACT）

当前源码重建的 loader 与推入设备测试路径的文件一致：

```text
app/app/src/main/jniLibs/arm64-v8a/libld.so
size=804352
sha256=020ab832e69e1aaadfe81219006636fc21347558a0e56b599e166a2240515c7c

files/libld-envexec-test.so
sha256=020ab832e69e1aaadfe81219006636fc21347558a0e56b599e166a2240515c7c

files/rootfs-full/tmp/gl-probe
size=584424
sha256=c1acc7b51eab7d60b8790a8ea95ecaec1c7ad3a4d75b3c23b0db538bf23fc229
```

启动方式为 `env -i`，显式设置 `NR_ROOTFS`、`NR_LD`、`PATH`、`HOME`，先启动 guest bash，
再由 bash 的 `exec` 进入探针。该路径同时覆盖环境物化与 loader 重入。

## 4 · fixed-function A/B（FACT，真机）

默认配置，600 帧：

```text
FIXED_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
FIXED_VERSION=2.1 Mesa 25.2.8-0ubuntu0.24.04.5
FIXED frames=600 elapsed=1.020s fps=588.5 pixel=255,214,57,255 err=0x0
RESULT PASS
FINAL_FIXED_RC=0
```

显式 `PANVK_DXVK_COMPAT=1`，同一探针、同一构建：

```text
FIXED_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
FIXED_VERSION=2.1 Mesa 25.2.8-0ubuntu0.24.04.5
segv|sig=11|fa=21|pc=507688709856|lr=32|x0=0|x1=21|x2=0|x8=0|ld=507940388864|lc=0
COMPAT1_RC=139
```

这组反证说明默认关闭没有把用户显式选择硬编码掉；也说明 PanVK/Bifrost 的 GS 编译缺陷仍在。

## 5 · 环境物化（FACT，真机）

`env -i` 启动 bash，再由 bash `exec usr/bin/env`，子进程实际收到的相关环境为：

```text
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
GSK_RENDERER=cairo
PANVK_DXVK_COMPAT=0
```

直接由 loader 启动目标进程时，变量仍由 shim 的 `getenv` 语义提供；发生 `execve` 后，
同一组默认值会被物化成真实 environ，供不经过 shim `getenv` 的库检查。

## 6 · GPU 回归（FACT，真机）

ES3：

```text
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
MRT(2 附件) FBO 完整 OK
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
```

Vulkan：

```text
vk|physical|PASS|count=1 name=Mali-G77 MC9
vk|buffer|PASS|created/bound/mapped size=4096 type=2 readback=0x4855534b
vk|compute|PASS|readback=0x4855534b
vk|graphics|PASS|pixel=0xff0000ff expected=0xff0000ff
```

## 7 · 最终 APK、冷启动迁移与发行包（FACT）

最终 APK：

```text
versionName=0.0.19
versionCode=19
size=1692596
sha256=63a83fb559fbabf32098336d130c81dad51c37f476ddea24927190c46d3c6e2c
```

覆盖安装并冷启动 `com.linuxhub.husk` 前，先从 `/etc/husk/gpu-env` 删除
`PANVK_DXVK_COMPAT=0`。App 启动后该行自动补回，说明 0.0.18 已有配置可以迁移。

安装包实际解出的 loader：

```text
size=804352
sha256=020ab832e69e1aaadfe81219006636fc21347558a0e56b599e166a2240515c7c
```

复制到 App 私有目录后用 `env -i` 跑 600 帧，得到上面的 `588.5 fps / PASS`；因此结果对应最终
APK 里的同一二进制，不是只测临时构建。

发行包：

```text
dist/husk-arm64.tar.gz  size=7127418
sha256=b94a368f1b157e10840a48a31edc8817e50f27c7e2fe0d3488d5fc515d8ee510

dist/SHA256SUMS         size=84
sha256=e28064c9a1af8c49ea40090c3f7b87846c7810021a7e647c417a01d1fdb88b36
```

主机门禁 `make check`：fmt、目标架构 `cargo check`、6 个 `gpu-env` 单测、Clippy、69 个 shell
脚本、269 个 Markdown 链接和 `VERSION=0.0.19` 一致性全部通过。

## 8 · 判断边界

- **FACT**：这是运行时默认值和环境传递修复，不是 PanVK 编译器修复。显式
  `PANVK_DXVK_COMPAT=1` 仍以 `fa=0x21` 崩溃。
- **FACT**：关闭 DXVK 兼容 GS 会影响依赖 geometryShader 的 Vulkan/DXVK 程序；这类程序必须
  显式设置 `PANVK_DXVK_COMPAT=1`，当前默认值不保证 DXVK 可用。
- **FACT**：本轮只验证了 fixed-function 离屏探针、ES3 和 Vulkan 探针；真实 X11 窗口里的
  `glxgears`、GNOME 桌面长跑、多应用并发和 10 分钟稳定性仍是 UNKNOWN。
- **FACT**：X11 的“窗口内容 → X server”仍是 SHM 回读加拷贝，本轮没有把呈现改成 dma-buf 零拷贝。
- **UNKNOWN**：其他 fixed-function 程序、其他 Mesa/PanVK 版本和其他 Mali GPU 是否同样稳定；
  不能据 600 帧探针宣称通用兼容。

发布口径：可以说“fixed-function PanVK GS 崩溃已有默认规避，600 帧探针通过，ES3/Vulkan 回归通过”；
不能写“PanVK 编译器已修”“`glxgears`/X11 已长跑通过”或“所有 GPU 程序已稳定”。
