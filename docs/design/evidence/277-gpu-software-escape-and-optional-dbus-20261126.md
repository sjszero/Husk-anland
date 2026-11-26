# 277 · GPU 软件逃逸与可选 D-Bus 启动

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页 / arm64，运行域仍为 `untrusted_app`。

## 0 · 结论

FACT：`HUSK_GPU=llvmpipe` 现在有两层保证：

1. `husk-session.sh` 把 `HUSK_GPU`、`NR_FORCE_SOFTWARE_GPU` 和布尔化的
   `NR_GPU_DEFAULT_OFF` 穿过最终 `env -i` 边界；
2. loader 在软件模式下不只停止注入硬件默认值，还强制软件 Mesa 变量并隐藏父环境里已经
   继承的 PanVK/Vulkan 选择器。

FACT：没有 `dbus-daemon` 或 bus 配置的最小 rootfs 不会再让 session 等待 6 秒，也不会因为
测试夹具依赖 `nc -lU` 而在 Linux CI 上出现假失败。D-Bus 启动现在只有在 daemon 可执行且
配置文件存在时才尝试。

## 1 · 修复前负对照

FACT：使用修复前的 loader（sha256
`a2ecbb1ea74aebd98e77ca0bc6af9d37f7accd603ecbbebb85b378641596813d`）和当前
`husk-session.sh`，在已由 session 注入 PanVK 的父环境中执行：

```text
env HUSK_GPU=llvmpipe /tmp/gl-probe es3
```

实际仍选择硬件路径：

```text
GL_VERSION=OpenGL ES 3.2 ... | RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
RESULT PASS
```

这不是想要的结果：显式软件请求被父环境中的 `VK_ICD_FILENAMES` / `GALLIUM_DRIVER` 覆盖。

## 2 · 修复后真机结果

FACT：使用当前 loader（sha256
`2a6c2ddd496feb5f61c3287ed7b93e98544117be43587dccd7ffae0332732b57`）和当前
`husk-session.sh`，三个入口都回到软件路径：

| 入口 | RENDERER | 结果 |
|---|---|---|
| session 入口 `HUSK_GPU=llvmpipe` | `llvmpipe (LLVM 20.1.2, 128 bits)` | `RESULT PASS` |
| session 入口 `NR_GPU_DEFAULT_OFF=1` | `llvmpipe (LLVM 20.1.2, 128 bits)` | `RESULT PASS` |
| session 入口 `NR_FORCE_SOFTWARE_GPU=1` | `llvmpipe (LLVM 20.1.2, 128 bits)` | `RESULT PASS` |

FACT：同一新 loader 的默认路径仍然使用硬件，证明软件修复没有把全局 GPU 默认值关掉：

```text
GL_VERSION=OpenGL ES 3.2 ... | RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
RESULT PASS
```

FACT：父环境已继承 PanVK、孙子进程再执行
`env HUSK_GPU=llvmpipe /tmp/gl-probe es3` 时，结果也是
`llvmpipe (LLVM 20.1.2, 128 bits)`，`RESULT PASS`。这才是本轮修复覆盖的真实嵌套 exec 条件。

## 3 · 本地门禁

FACT：`make check` 通过，新增的 `tools/ci/test-session-gpu-escape.sh` 四条断言均通过：

```text
session GPU test: husk-gpu PASS
session GPU test: default-off PASS
session GPU test: force-software PASS
session GPU test: default-off-zero PASS
session GPU escape: PASS
```

FACT：完整真机门禁 A 通过：

```text
PASS=38 FAIL=0
coverage-probe: PASS
coverage-sweep: SKIP（rootfs 没有 strace）
dpkg conffile: PASS
V4L2: PASS
DRM: PASS
fd identity: PASS
identity-init race: PASS
groups race: PASS
session guard: PASS
rootfd race: PASS
path race: PASS
```

FACT：`coverage-sweep` 为 `SKIP`，不能算通过；这是当前测试树的缺包，不是本轮修复回归。

## 4 · 发布资产

FACT：0.0.33 候选资产已重新构建，包内版本为 `0.0.33`：

```text
3df41d3d17474b9877d992d3b6621989eaf4738ca58fdb01caa5a3221ee8e5eb  husk-arm64.tar.gz
2a6c2ddd496feb5f61c3287ed7b93e98544117be43587dccd7ffae0332732b57  tar/libld.so
c7e1ab50abbcb9ccfc53b5f591ff3dfe481795b643de2a563fd33b962a9aac36  tar/husk-session.sh
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  tar/libc.so.6
```

FACT：APK 构建成功，`aapt2 dump badging` 回读：

```text
package: name='com.linuxhub.husk' versionCode='33' versionName='0.0.33'
BUILD SUCCESSFUL
```

## 5 · 边界

- FACT：本轮动态验证只覆盖上述 RMX3357 设备组合。
- UNKNOWN：其他 Android 版本、16 KiB/64 KiB 真机、Adreno 和其他 GPU 家族上的同样行为。
- UNKNOWN：真实桌面会话在长时间运行中是否还存在其他环境继承路径。
- UNVERIFIED：没有人为注入 `dbus-daemon` 启动失败或配置文件损坏分支；代码只保证缺少
  daemon/配置时跳过，不会冒充总线已启动。
- FACT：`HUSK_GPU=llvmpipe` 只切换图形软件栈，不代表关闭其他硬件接口或改变 rootfs 权限。

