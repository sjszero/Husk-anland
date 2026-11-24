# evidence 158 — Zink 在 PanVK 上默认只给 ES2：是 Mesa 的声明上限，不是 GPU 能力

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB。
安装：`/data/local/tmp/husk-c4/b02`（不装 APK，evidence 157），会话由 `husk-session.sh` 起。
时间：2026-11-24（CST）。探针：guest 内 `gcc` 现场编译的 `gl3test.c` / `gl46test.c`（无头文件依赖）。

## 1. 事实

| 试验 | 结果 |
|---|---|
| `vulkaninfo` 设备信息 | `apiVersion 1.3.354`、`deviceName Mali-G77 MC9`、`VK_KHR_dynamic_rendering` / `descriptor_indexing` / `robustness2` / `synchronization2` / `timeline_semaphore` / `maintenance1..9` 全在 |
| 默认 zink，请求 ES3.0 上下文 | **失败**：`eglCreateContext` → `EGL_BAD_CONFIG (0x3005)`；`eglinfo -B` 报 `GL 2.1 / GLES 2.0` |
| `MESA_GLES_VERSION_OVERRIDE=3.2` | ES3.2 上下文建立成功，64×64 FBO 清屏 + `glReadPixels` 得到 `3399e6ff`，`PASS` |
| `MESA_GL_VERSION_OVERRIDE=4.6` | GL 4.6 core 上下文建立成功，同样 `3399e6ff` / `PASS` |

`GL_RENDERER` 三种情况下都是 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。

## 2. 结论

- **FACT**：GPU 与 PanVK 具备跑更高 GL 等级的能力；默认的 ES2/GL2.1 是 Mesa 报出来的上限。
- **FACT**：`MESA_GLES_VERSION_OVERRIDE` / `MESA_GL_VERSION_OVERRIDE` 能把上下文抬到 3.2 / 4.6，
  而且一次简单的清屏 + 回读是正确的。
- **UNKNOWN（关键）**：抬上去之后**复杂特性**是否真的正确（纹理压缩、MSAA、几何/曲面细分、
  变换反馈、UBO/SSBO 边界、GLES3 shader 语义）。一次清屏回读不足以证明「ES3 可用」。
- 因此：override **不进发布配置**，只作实验开关；`husk-session.sh` 默认仍是 zink + 默认等级。

## 3. 还没查清的

默认只报 ES2 的判定发生在 Mesa 的 zink 里。今天没查到的：

- 这个 Mesa（27.0.0-devel，Ubuntu 树自带）里 zink 决定版本的具体条件；
- 为什么 `vkCreateInstance` 在初始化阶段先报一次 `VK_ERROR_INCOMPATIBLE_DRIVER`（另一个 ICD
  被枚举到、失败后继续），它对版本判定有没有影响；
- `ZINK_DEBUG=info` 在这棵树上没有输出，说明该构建可能裁掉了 debug 通道。

下一步要么读这棵树的 Mesa 源码/构建配置，要么换一份自己编的 Mesa 对照。**在那之前，桌面
仍按「软件渲染」对待**，不写「已硬件加速」。
