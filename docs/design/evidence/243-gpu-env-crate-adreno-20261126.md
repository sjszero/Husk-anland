# 243 · GPU 默认值统一为 Rust crate，并补 Adreno 启动级路径

日期：2026-11-26 03:00–03:08（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 /
4 KiB 页 / Mali-G77 MC9。测试包 `com.linuxhub.husk`，rootfs `files/rootfs-full`。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| GPU 默认值实现抽成纯 Rust `gpu-env` crate | App 初始环境与 `/etc/husk/gpu-env` 使用同一份生成代码，loader 读取该文件 | FACT，源码 + 构建 |
| GPU 默认值单元测试 | 5 passed / 0 failed | FACT，本机 |
| Mali 真机冷启动自动重建 | `gpu-env` 与 canonical ICD 均重建，内容一致 | FACT，真机 |
| 无 GPU 变量 `env -i` | `zink Vulkan 1.3(Mali-G77 MC9)`，`RESULT PASS` | FACT，真机 |
| `HUSK_GPU=llvmpipe` | `llvmpipe (LLVM 20.1.2, 128 bits)`，`RESULT PASS` | FACT，真机 |
| App 启动级 Adreno/KGSL 分支 | Freedreno/Turnip 存在时生成 Adreno 配置；无驱动不伪造 | FACT，单元测试 |
| Adreno 真机 | 没有 Adreno 设备，打开 KGSL、Turnip 枚举、桌面和长跑都是 UNKNOWN | 未验证 |

## 2 · 为什么改

0.0.17 已让 Mali 的 GPU 默认值由 App 生成、loader 读取，但生成逻辑只存在于 App 的
`term.rs`，而且只覆盖 Mali。安装器和登录 shell 已有 Adreno 分支，App 冷启动却没有；
在 rootfs 搬到 Adreno 设备或由 App 管理 rootfs 时，初始会话不会得到同一套 Turnip/freedreno
默认值。

另一个缺口在软件回退：App 原来只导出 `LIBGL_ALWAYS_SOFTWARE/GALLIUM_DRIVER=llvmpipe`，
没有把 `HUSK_GPU=llvmpipe` 传给子进程。loader 随后读取 `/etc/husk/gpu-env` 时，文件中
`!MESA_LOADER_DRIVER_OVERRIDE` / `!GALLIUM_DRIVER` 的无条件项可能再次覆盖回退。现在软件
回退显式带 `HUSK_GPU=llvmpipe`，loader 首先识别逃逸开关，不再重新注入硬件默认值。

## 3 · 代码改动（FACT，源码）

- 新增纯 Rust crate `runtime/crates/gpu-env`，集中管理 App 侧生成逻辑：
  - Mali/PanVK canonical ICD 与驱动候选；
  - Adreno/Freedreno/Turnip canonical ICD 与驱动候选；
  - `/etc/husk/gpu-env` 的原子写入与已有正确配置保留；
  - App 初始环境和 `/etc/husk/gpu-env` 的同一套字段。
- App 的 `term.rs` 只负责探测真实节点并调用 `gpu_env::build`：
  - `/dev/mali0` 可读写时走 Mali；
  - `/dev/kgsl-3d0` 可读写时走 Adreno/KGSL；
  - 驱动文件不存在时不生成假硬件配置。
- Adreno 分支：
  - 有 `kgsl_dri.so` / `freedreno_dri.so` 时使用 `MESA_LOADER_DRIVER_OVERRIDE=kgsl` 与
    `GALLIUM_DRIVER=freedreno`；
  - 只有 Vulkan Freedreno/Turnip ICD 时退回 `zink`；
  - 统一写入 `TURNIP_KMD=kgsl`、`FD_FORCE_KGSL=1`。
- `make check` 现在同时检查 `gpu-env`，并执行它的主机侧单元测试。

## 4 · 单元测试（FACT，本机）

```text
running 5 tests
test tests::software_override_is_forwarded_to_loader ... ok
test tests::missing_driver_does_not_fake_hardware_or_write_defaults ... ok
test tests::preserves_existing_matching_loader_defaults ... ok
test tests::mali_generates_guest_relative_icd_and_loader_defaults ... ok
test tests::adreno_prefers_native_freedreno_then_falls_back_to_zink ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 5 · 最终 APK 与冷启动（FACT，真机）

最终 APK：

```text
versionName=0.0.18
versionCode=18
size=1690720
sha256=004e42c1adce466a0414d3348a7ea9e3e82c9296224597ae06c8e77e520374d8
```

发布包：

```text
dist/husk-arm64.tar.gz  size=7126250
sha256=99966d94c28bbf31edda0e5ea45caf05adf0ab9c67a1def96e0e978a0db2b34a

dist/SHA256SUMS         size=84
sha256=6d41e948966772b5b9a69ab130e038eac5dcfc77bbfe386f400b7a2a6f15d2a2
```

测试前删除 App 管理 rootfs 中的：

```text
files/rootfs-full/etc/husk/gpu-env
files/rootfs-full/usr/share/vulkan/icd.d/husk_panvk_guest.json
```

覆盖安装、force-stop、冷启动后两份文件自动重建。最终 `gpu-env`：

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

canonical ICD：

```json
{
  "ICD": {
    "api_version": "1.4.352",
    "library_arch": "64",
    "library_path": "/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so"
  },
  "file_format_version": "1.0.1"
}
```

初始 guest shell 的环境中没有手工测试变量，仍包含：

```text
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/husk_panvk_guest.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

## 6 · 无变量与软件回退（FACT，真机）

从 App 私有目录复制同一份 loader 到普通文件名下，`cd` 到 rootfs 后执行：

```sh
env -i \
  NR_ROOTFS=$R \
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  HOME=/root \
  <loader> --library-path "$R/usr/lib/aarch64-linux-gnu" tmp/gl-probe es3
```

结果：

```text
RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
att0 像素异常数=0  纹理四色命中数=64/64/64/64
att1（MRT 第二附件）与 1-att0 不符数=0
RESULT PASS
GPU_RC=0
```

同一入口只额外加 `HUSK_GPU=llvmpipe`：

```text
RENDERER=llvmpipe (LLVM 20.1.2, 128 bits)
att0 像素异常数=0  纹理四色命中数=64/64/64/64
RESULT PASS
SOFT_RC=0
```

设备上的临时测试 loader `files/husk-global-test-loader.so` 已在两次复验后删除。

## 7 · 边界

- **FACT**：Adreno 分支目前只经过源码审查和主机侧单元测试；没有 Adreno 真机，
  `/dev/kgsl-3d0` 的真实打开、Turnip 枚举、ioctl/mmap、X11 桌面与长跑仍为 UNKNOWN。
- **FACT**：X11 窗口内容进入 X server 仍是 SHM 回读加拷贝，本次没有改变呈现路径。
- **UNKNOWN**：`glxgears` 的 PanVK/Bifrost 编译器 SIGSEGV 没有在本次重测为已修。
- **UNKNOWN**：Adreno 驱动文件若使用本仓库未列出的私有命名，需要在真机上补充候选或改为目录扫描。
