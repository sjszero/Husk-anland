# Evidence 44 — 假 `/dev/dri/renderD128` 生效：DRM interpose 全链路通

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-23 02:5x（CST）
工具：`path-shim`（loader 内嵌 shim）+ `drm-shim`（no_std）+ `drmfake`

> M1 的收尾：让**只懂 DRM、完全不知道 kbase 的程序**，在根本没有 `/dev/dri/renderD128` 的机器上
> 成功 alloc 显存、映射、读写。这是 Mesa panfrost 起来的前提。

## 1. 结果（FACT，一次跑完）

```
drmfake|open|PASS|/dev/dri/renderD128 fd=5（宿主上不存在这个节点）
drmfake|paramGPU_PROD_ID|PASS|0x9080
drmfake|paramJS_PRESENT|PASS|0x7
drmfake|create_bo|PASS|handle=1 offset=0x41000
drmfake|mmap_bo|PASS|offset=0x41000
drmfake|mmap|PASS|va=0x41000 写读一致
drmfake|get_bo_offset|PASS|0x41000
drmfake|verdict|PASS|假 renderD128 上的 DRM 语义已生效（interpose 通了）
```

`drmfake` 全程只用 libc 的 `open` / `ioctl` / `mmap`，**没有任何 kbase 知识**。

## 2. 实现（三个钩子，都在 `runtime/crates/path-shim`）

| 钩子 | 行为 |
|---|---|
| `openat` | `/dev/dri/renderD128`、`/dev/dri/card0` → 返回一个真 fd（`/dev/null`）并登记在 `FAKE_FDS`；首次触发时懒初始化 `drm_shim::DrmCtx`（打开 `/dev/mali0` + `VERSION_CHECK`/`SET_FLAGS`/`GET_GPUPROPS`） |
| `ioctl` | fd 是假 fd → `DrmCtx::dispatch`（CREATE_BO/MMAP_BO/GET_PARAM/GET_BO_OFFSET/SUBMIT…）；否则原样 `SYS_IOCTL` |
| `mmap` / `mmap64` | fd 是假 fd → 把 DRAM 的 offset（= GPU VA）转到 `/dev/mali0` 上真映射，prot 由 BO 的 kbase flags 决定；否则原样 `SYS_MMAP` |

两个实现细节：

- `drm-shim` 改成 **`no_std` + 固定容量表**（192 个硬件参数、256 个 BO），因为它要编进 loader 里的 shim；
  ioctl 原语用**闭包**传入，所以本 crate 不链接任何 libc。
- `ioctl` 是 C 变参函数。AAPCS64 下 `ioctl(fd, req, arg)` 的第三个参数就在 `x2`，
  所以用固定三参数定义即可 ABI 兼容（不引入 `c_variadic`）。

## 3. loader 尺寸没超（FACT）

`tools/loader/build-embedded-loader.sh` 正常跑完（`nr-tools embed` 自己会拒绝不合规的 loader）：

```
"interposed": 92, "externs": 33, "relocations": {"relative": 95, "ext": 33},
"total_load": 139264, "out_size": 345600
```

新加的三个符号（`ioctl`/`mmap`/`mmap64`）走的是 shim 自己的导出表 → GOT interpose 自动生效，
不需要往 `BODY_PATCH`（libc 内部调用那套）里加。

## 4. 下一步（M2）

现在缺的只有"让 Mesa 真的去用这个节点"：

1. 写 `gl-probe`（纯 Rust）：`dlopen("libEGL.so.1")` → `eglGetPlatformDisplay(EGL_PLATFORM_SURFACELESS_MESA)`
   → `eglInitialize` → `eglQueryString(EGL_VENDOR/RENDERER)`。
2. 环境：`MESA_LOADER_DRIVER_OVERRIDE=panfrost`、`LIBGL_ALWAYS_SOFTWARE=0`、rootfs 用 `rootfs-full`（Mesa 25.2.8）。
3. 判据：日志里 panfrost probe 通过、`EGL_RENDERER` 里出现 `Mali`（而不是 `llvmpipe`）。

## 5. 复现命令

```sh
cd $REPO
NDK=<ndk>/toolchains/llvm/prebuilt/darwin-x86_64/bin sh tools/loader/build-embedded-loader.sh
adb push tools/probe-app/app/src/main/jniLibs/arm64-v8a/libld.so /data/local/tmp/ld-gpu.so
adb push runtime/target/aarch64-unknown-linux-gnu/release/drmfake /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/ld-gpu.so files/ld-gpu.so'
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/drmfake files/rootfs/usr/bin/'
adb shell 'run-as com.linuxhub.husk sh -c "NR_LD=/data/data/com.linuxhub.husk/files/ld-gpu.so sh files/gexec.sh usr/bin/drmfake"'
```
