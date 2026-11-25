# 242 · GPU 默认值落到加载器：`env -i` 也拿到真实 Mali 渲染器

日期：2026-11-26 02:50–02:52（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 /
4 KiB 页 / Mali-G77 MC9。测试包 `com.linuxhub.husk`，rootfs `files/rootfs-full`。

## 1 · 结论

- **FACT**：App 现在不仅给初始 shell 注入 GPU 环境，还会在 Mali/kbase 节点真实可打开时生成
  guest 内的 `/etc/husk/gpu-env`。因此凡是经过 Husk loader 启动的 guest 进程，即使使用
  `env -i`、完全没有传入 Vulkan/EGL 变量，也能由加载器补上同一套 GPU 默认值。
- **FACT**：删除 `etc/husk/gpu-env` 和 canonical ICD 后冷启动 App，两份文件都自动重建；初始
  guest shell 直接指向 `/usr/share/vulkan/icd.d/husk_panvk_guest.json`。
- **FACT**：`env -i` 只保留 `NR_ROOTFS/PATH/HOME` 启动 `gl-probe es3`，renderer 仍是
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，完整 ES3 workload `RESULT PASS`。
- **FACT**：同一 `env -i` 加上 `HUSK_GPU=llvmpipe` 后 renderer 变成
  `llvmpipe (LLVM 20.1.2, 128 bits)`，说明用户显式回退仍然优先。
- **UNKNOWN**：Adreno/KGSL 和通用 DRM 的设备族没有在对应真机上验证；本次只证明 Mali 路径
  的加载器级默认值和 Mali 设备门控。

## 2 · 修复前的缺口

上一版解决了 ICD 内部 `library_path` 的宿主路径污染，但 App 只在创建初始 `Term` 时把环境变量
放在子进程环境里。guest 内其他经 loader 启动、环境被清空的进程不会自动继承；如果 rootfs 没有
安装器写入的 `/etc/husk/gpu-env`，就会掉回软件路径或读到旧 manifest。

这不是 GPU 驱动问题，而是“默认值放在哪一层”的问题。正确位置是 loader：登录 shell、cron、
桌面 fork 的子进程、脚本里的 `env -i` 都应看到同一份配置。

## 3 · 代码改动（FACT，源码）

- App 只有 `/dev/mali0` 以读写方式真实可打开时，才选择 PanVK、生成 canonical ICD 并写
  `/etc/husk/gpu-env`；不会因为 rootfs 里碰巧带了 PanVK 文件就把 Adreno/其他 GPU 强行改成
  Mali 路径。
- `/etc/husk/gpu-env` 的内容：
  - `@GPU_FAMILY=mali-kbase`
  - guest 相对 EGL vendor manifest
  - guest 相对 PanVK ICD
  - Zink / GLES 3.2 默认值
  - 屏蔽旧会话可能塞入的 `LIBGL_ALWAYS_SOFTWARE`
  - GTK 默认 `GSK_RENDERER=cairo`，和安装器口径一致
- 如果已有文件已经包含 `@GPU_FAMILY=mali-kbase` 和 canonical ICD，则不覆盖用户内容；缺失、
  旧 host-path 或 glob 选出的旧 manifest 才会被修复。
- 文件使用同目录临时文件 + rename，避免并发启动时读到半写配置。

## 4 · 冷启动自动重建（FACT，真机）

测试前删除：

```text
files/rootfs-full/etc/husk/gpu-env
files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_guest.json
```

force-stop 后冷启动 App。初始 guest shell 的 `/proc/<pid>/environ`：

```text
__EGL_VENDOR_LIBRARY_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

自动生成的 `/etc/husk/gpu-env`：

```text
@GPU_FAMILY=mali-kbase
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
!MESA_LOADER_DRIVER_OVERRIDE=zink
!GALLIUM_DRIVER=zink
-LIBGL_ALWAYS_SOFTWARE
MESA_GLES_VERSION_OVERRIDE=3.2
GSK_RENDERER=cairo
```

## 5 · 不继承父 shell 的 `env -i`（FACT，真机）

测试方法：从当前 App 的 loader 可执行文件复制一份到 App 私有目录作为测试入口，然后：

```sh
env -i \
  NR_ROOTFS=$R \
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  HOME=/root \
  <loader> --library-path "$R/usr/lib/aarch64-linux-gnu" tmp/gl-probe es3
```

命令里没有 `VK_*`、`MESA_*`、`GALLIUM_DRIVER`、`__EGL_*` 或 `HUSK_GPU`。结果：

```text
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
EGL 1.5
ES3 位 config: 有
ES3 上下文建立 OK
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
GLSL=OpenGL ES GLSL ES 3.20
MRT(2 附件) FBO 完整 OK
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
RC=0
```

这证明 GPU 默认值已经不再依赖 App 初始 shell 的父环境，而是由 loader 读取 guest 配置后注入。

## 6 · 软件回退（FACT，真机）

同一 `env -i`，只额外加：

```text
HUSK_GPU=llvmpipe
```

结果：

```text
RENDERER=llvmpipe (LLVM 20.1.2, 128 bits)
ES3 上下文建立 OK
RESULT PASS
RC=0
```

这证明回退开关仍然按进程优先，不会因为 loader 默认值把用户明确选择的软件渲染覆盖掉。

## 7 · 边界

- **FACT**：本次是默认值和设备门控修复，不是 X11 零拷贝；窗口内容进入 X server 仍走
  SHM 回读加拷贝。
- **FACT**：PanVK 的 `MESA_GLES_VERSION_OVERRIDE=3.2` 仍是兼容性覆盖，不是设备原生默认值。
- **UNKNOWN**：`glxgears` 的 PanVK/Bifrost 编译器崩溃没有在本次重测；不能用 `gl-probe`
  单次通过写成 GPU 长跑通过。
- **UNKNOWN**：Adreno/KGSL 和通用 DRM 的加载器默认值需要对应真机验证。

## 8 · 最终 0.0.17 重跑（FACT，真机）

发布 APK：

```text
versionName=0.0.17
versionCode=17
size=1687820
sha256=03f7716d5359b1af8611a7ceefec43271274dc1cd3e32dbf29fd8faedc692359
```

安装前删除 `etc/husk/gpu-env`、canonical ICD 和临时测试 loader，覆盖安装后 force-stop、冷启动。
App 自动重建两份配置，初始 guest shell（PID 13241）环境：

```text
__EGL_VENDOR_LIBRARY_FILENAMES=/data/user/0/com.linuxhub.husk/files/rootfs-full/usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

最终构建的 `env -i` 无 GPU 变量测试：

```text
FristOneRR 1.2.0 (Mesa 26.3.0-devel)
EGL 1.5
ES3 位 config: 有
ES3 上下文建立 OK
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
GLSL=OpenGL ES GLSL ES 3.20
MRT(2 附件) FBO 完整 OK
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
GPU_RC=0
```

同一构建加 `HUSK_GPU=llvmpipe`：

```text
RENDERER=llvmpipe (LLVM 20.1.2, 128 bits)
RESULT PASS
SOFT_RC=0
```

发布前曾删除临时测试 loader；最终 `env -i` 复验时又临时复制了一份
`files/husk-global-test-loader.so`，复验结束后已删除。它不是产品资产。

## 9 · 发布产物（FACT，本地构建）

```text
husk-arm64.tar.gz  7126250 bytes
sha256             b473cd15be02ed473784b5b8537609bdff94ebd09a3981389ece268dbac2071f
SHA256SUMS         84 bytes
sha256             48065f2e987b1077d57dda0e60fbb698516d9cc3be9be642c2b8f82ab8ed6a3c
```
