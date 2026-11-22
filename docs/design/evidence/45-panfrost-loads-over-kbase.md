# evidence 45 — Mesa panfrost 在 guest 里被拉起来，并从 kbase 读到了 GPU 参数（还没出画面）

日期：2026-11-23 · 设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191

## 做了什么

1. **两层 shim 合流**：loader 里的 `path-shim` 抢走了启动对象的 `open/ioctl/mmap`（GOT 改写），
   Mesa 运行时 dlopen 的 `libdrm/libgbm/panfrost_dri.so` 只有 `LD_PRELOAD` 能盖住。
   两个 no_std 层不能各建一个 kbase context，所以由 preload **独占** DRM 状态，
   path-shim 通过环境变量 `HUSK_DRM_API`（函数表：fake_open/ioctl/mmap/is_fake）转交。
2. **libdrm 替换层**（`runtime/crates/drm-preload/src/drmapi.rs`）：实现
   `drmGetVersion / drmFreeVersion / drmGetDevice2 / drmGetDevice / drmGetDevices2 / drmGetDevices /
   drmFreeDevice(s) / drmDevicesEqual / drmGetNodeTypeFromFd / drmGetRenderDeviceNameFromFd /
   drmGetCap / drmIoctl / drmCloseBufferHandle`。理由：假 fd 是 memfd 时 `st_rdev` 是 0:0，
   libdrm 判断不出设备 → Mesa 只打印 `failed to retrieve device information` 然后退回 llvmpipe。
3. **假 fd 改成真 `/dev/mali0` fd**（不再用 memfd//dev/null）：`dup()` 出来的 fd 靠
   `/proc/self/fd` 里的 `mali` 能认回来，libdrm 也能看到合法字符设备。
4. **GET_PARAM 补 Mesa 25.2 的枚举**：`THREAD_TLS_ALLOC=0`、`SYSTEM_TIMESTAMP=0`、
   `SYSTEM_TIMESTAMP_FREQUENCY=1e9`，未知参数返回 0 而不是 EINVAL。
5. **踩到一个必须记住的坑**：`#[no_mangle] extern "C" fn ioctl` 在自己的 crate 里被直接调用时，
   可能经 PLT 解析回 libc 的 `ioctl`（被 LD_PRELOAD 语义绕回内核），表现为
   `DRM_IOCTL_PANFROST_GET_PARAM` 直接打到 `/dev/mali0` 后卡死。修法：内部实现改名
   `ioctl_impl`，导出符号只做转发。

## 实测结果（FACT）

```
gl|open_render|PASS|fd=5                     ← 假 renderD128 打开成功
[preload] kbase CTX.init ok                  ← kbase 上下文建成（r32p1 / UK 11.38）
gl|gbm_create_device|PASS                    ← gbm 接受这个 fd
GET_PARAM × 15（两轮）：
  id=00 GPU_PROD_ID       0x9080             ← Mali-G77
  id=01 GPU_REVISION      0x1
  id=02 SHADER_PRESENT    0x660067
  id=0a TILER_FEATURES    0x809
  id=0b MEM_FEATURES      0x301
  id=0c MMU_FEATURES      0x2830
  id=0d THREAD_FEATURES   0x48000
  id=0e MAX_THREADS       0x400
  id=0f WORKGROUP_MAX     0x200
  id=12..15 TEXTURE_FEATURES0..3
  id=27 THREAD_TLS_ALLOC  0
  id=28 AFBC_FEATURES     0
libEGL warning: egl: failed to create dri2 screen   ← 当前卡点
gl|initialize|FAIL|EGL 0.0 (0x3001)
```

- 若把我们的设备从 `drmGetDevices2` 里拿掉：EGL 正常初始化，但 `GL_RENDERER=llvmpipe`
  （= 根本没有 panfrost 参与），所以**必须**保留这个设备项。
- 结论：**Mesa 的 panfrost gallium 已经在 guest 里加载，并且通过我们的 Rust 语义层读到了
  真实 kbase GPU 参数**。还没到"能出画面"。

## 未完成 / 下一步

- `panfrost_open_device()` 返回非 0（屏幕创建失败）。下一步按 A 计划：对着 Mesa 25.2 的
  `panfrost_screen_create` / `panfrost_open_device` 逐项核对它需要的 ioctl 和参数语义
  （尤其是 `GET_PARAM` 的编码是否符合 panfrost 位定义，而不是 kbase 原始值）。
- 参考实现规格书：[../mali-kbase-prior-art.md](../mali-kbase-prior-art.md)
  （`mesa-kbase-j4lte` 的 libdrm-kbase 1785 行、atom/fence 模型都已抄进文档）。
- 若 panfrost gallium 在 Valhall + kbase 上确实走不通，兜底 = panvk kbase fork 编 glibc 版 + Zink
  （已有 Mali-G77 实机验证的先例，见 references.md §3）。

## 复现

```sh
adb shell 'run-as com.linuxhub.husk sh -c "cd files && \
  MESA_LOADER_DRIVER_OVERRIDE=panfrost NR_LD=$PWD/ld-gpu.so \
  timeout 30 sh gexec3.sh rootfs-full usr/bin/gl_probe"'
```
