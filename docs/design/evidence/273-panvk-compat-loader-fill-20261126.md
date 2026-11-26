# 273 · PanVK DXVK 兼容开关的加载器兜底

日期：2026-11-26（CST）。

## 0 · 结论

FACT：当前设备 rootfs 的 `/etc/husk/gpu-env` 是旧格式，里面没有
`PANVK_DXVK_COMPAT` 这一行。只靠旧文件启动时，PanVK 的 Bifrost kbase 构建会打开包含
`geometryShader` 的 DXVK 兼容路径，Zink 的固定管线转译可能以 `fa=0x21` 触发 SIGSEGV。

修复不要求用户先重装 rootfs：Mali 设备上如果配置文件缺少该变量，loader 在内存中补一个
`PANVK_DXVK_COMPAT=0` 的 fill 默认值。以下三种情况仍优先：

- 调用方显式设置 `PANVK_DXVK_COMPAT=1`；
- `/etc/husk/gpu-env` 使用 `!PANVK_DXVK_COMPAT=...` 强制值；
- `/etc/husk/gpu-env` 使用 `-PANVK_DXVK_COMPAT` 隐藏该变量。

## 1 · 真机证据

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB。

在配置文件确实没有该行、外层也没有设置该变量的情况下，嵌套 `execve` 后 `/usr/bin/env`：

```text
PANVK_DXVK_COMPAT=0
```

guest 内 C 探针同时调用 `getenv` 与 `secure_getenv`：

```text
PANVK_GETENV=0
PANVK_SECURE=0
```

显式 `PANVK_DXVK_COMPAT=1` 时，两者都返回 `1`，说明用户覆盖仍然有效：

```text
PANVK_GETENV=1
PANVK_SECURE=1
```

同一 loader 上的 GPU 回归：

```text
FIXED_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
FIXED frames=60 elapsed=0.257s fps=233.0 pixel=255,221,59,255 err=0x0
RESULT PASS
```

```text
GL_VERSION=OpenGL ES 3.2 Mesa 27.0.0-devel
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
RESULT PASS
```

FACT：完整门禁 A 为 `PASS=38 FAIL=0`，coverage-probe 为 PASS；该轮没有把
`PANVK_DXVK_COMPAT=1` 的已知编译器崩溃重新当作通过项。

## 2 · 边界

- 这是**默认配置兜底**，不是修复 PanVK 编译器。显式打开 `PANVK_DXVK_COMPAT=1` 仍可能崩溃，
  见 evidence 244。
- `UNKNOWN`：Adreno/KGSL 真机上没有重跑本探针；代码按 `GPU_MALI` 门控，不会把 Mali 的开关塞给
  Adreno。
- `UNKNOWN`：其它 PanVK 版本是否使用同样的 DXVK 兼容开关语义；本次只验证设备上的 Mesa/PanVK 构建。
