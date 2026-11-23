# Evidence 81 — Rust VA-API 驱动入口与 GStreamer 元素注册

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
运行域：`untrusted_app`（`com.linuxhub.husk`）  
rootfs：`/data/data/com.linuxhub.husk/files/rootfs-full`

## 结论

**FACT — Husk 的纯 Rust VA-API 驱动已经能被系统 libva 加载。** `vainfo` 报出
`vendor=husk`，并枚举 `VAProfileH264High` 与 `VAProfileHEVCMain` 的
`VAEntrypointVLD`。驱动同时返回 NV12 image format，满足 gstreamer-vaapi 注册元素前的
能力查询。

**FACT — GStreamer 已识别 Husk VA 驱动并注册 4 个 element：**

```text
vaapidecodebin
vaapih264dec
vaapih265dec
vaapisink
```

这证明“标准 VA-API 系统层接口 + Husk 后端”的方向可被现有媒体栈直接发现，不需要修改
GStreamer 插件源码。

**UNKNOWN / 未完成 — 这不是硬解数据面通过。** 当前驱动只实现了 profile、entrypoint、
config 和 image-format 查询；surface/context/buffer/render/sync/export 仍是显式
`VA_STATUS_ERROR_UNIMPLEMENTED`。因此不能把本证据写成 H.264/HEVC 实际解码、Chromium
`<video>` 或完整视频链路完成。

## 驱动行为

- `__vaDriverInit_1_0`：设置 VA-API 1.20 上下文能力与 vendor 字符串。
- `vaQueryConfigProfiles`：报告 H.264 High、HEVC Main。
- `vaQueryConfigEntrypoints`：两者均报告 VLD。
- `vaQueryImageFormats`：报告 `NV12`，`bits_per_pixel=12`、`depth=12`。
- config 查询/创建/销毁：提供最小可用生命周期。
- 其余数据面入口：保留非空 stub，避免 libva 初始化检查失败，但明确返回
  `VA_STATUS_ERROR_UNIMPLEMENTED`。

## 上机输出

`va-probe`：

```text
HUSK_VA vendor=husk profiles=2
PROFILE id=7 entrypoints=1 first=1
PROFILE id=17 entrypoints=1 first=1
IMAGE_FORMAT count=1 fourcc=0x3231564e bpp=12 depth=12
HOST_RC=0
```

`vainfo`：

```text
libva info: User environment variable requested driver 'husk'
libva info: Found init function __vaDriverInit_1_0
libva info: va_openDriver() returns 0
vainfo: VA-API version: 1.20 (libva 2.12.0)
vainfo: Driver version: husk
      VAProfileH264High               : VAEntrypointVLD
      VAProfileHEVCMain               : VAEntrypointVLD
HOST_RC=0
```

`gst-inspect-1.0`：

```text
vaapidisplay: vendor: husk
vaapidisplay: 1 image formats
vaapidisplay:   NV12
vaapidisplay: 2 profiles
vaapidisplay:   VAProfileH264High
vaapidisplay:   VAProfileHEVCMain
  4 features:
  +-- 4 elements
```

## 测试专用 shim 与路径事实

**FACT — 本机 unprivileged guest 没有 `/dev/dri/renderD128`。** libva 的 DRM transport
因此会在加载驱动前拒绝测试。`va-node-shim` 仅把 `drmGetNodeTypeFromFd()` 报告为 render
node，让 libva 走到 Husk 驱动；它不模拟 DRM，也不参与产品数据面。

**FACT — GStreamer 的插件扫描路径下，`LIBVA_DRIVERS_PATH` 要传 rootfs 中驱动的宿主绝对
路径。** 传 guest 路径 `/usr/lib/aarch64-linux-gnu/dri` 时，libva 能打印目标路径但
`dlopen` 失败；传
`/data/data/com.linuxhub.husk/files/rootfs-full/usr/lib/aarch64-linux-gnu/dri`
后加载成功。该差异属于当前 loader/path-shim 的待收敛项，不能靠猜写成“驱动加载不稳定”。

## 复现命令

构建：

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p husk-va-driver -p va-node-shim \
  --target aarch64-unknown-linux-gnu
```

设备变量：

```sh
DEV=UKHMMB8PYTGECMNV
LIB=$(adb -s "$DEV" shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/data/com.linuxhub.husk/files/rootfs-full
```

`va-probe`：

```sh
adb -s "$DEV" shell "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files && \
  timeout 15 env NR_LD=$LIB/libld.so sh gexec-va.sh rootfs-full \
  $R/usr/bin/va-probe 2>&1; echo HOST_RC=\$?'"
```

`vainfo`：

```sh
adb -s "$DEV" shell "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files && \
  timeout 20 env HUSK_LD_PRELOAD=$R/usr/lib/aarch64-linux-gnu/libva_node_shim.so \
  LIBVA_DRIVER_NAME=husk LIBVA_DRIVERS_PATH=$R/usr/lib/aarch64-linux-gnu/dri \
  NR_LD=$LIB/libld.so sh gexec-va.sh rootfs-full \
  $R/usr/bin/vainfo --display drm --device /dev/mali0 2>&1; echo HOST_RC=\$?'"
```

`gst-inspect-1.0`：

```sh
adb -s "$DEV" shell "run-as com.linuxhub.husk sh -c 'cd /data/data/com.linuxhub.husk/files && \
  rm -f $R/tmp/husk-gst-registry.bin; timeout 30 env \
  HUSK_LD_PRELOAD=$R/usr/lib/aarch64-linux-gnu/libva_node_shim.so \
  LIBVA_DRIVER_NAME=husk LIBVA_DRIVERS_PATH=$R/usr/lib/aarch64-linux-gnu/dri \
  GST_PLUGIN_PATH=/usr/lib/aarch64-linux-gnu/gstreamer-1.0 \
  GST_REGISTRY=/tmp/husk-gst-registry.bin GST_REGISTRY_FORK=no \
  GST_VAAPI_DRM_DEVICE=/dev/mali0 GST_VAAPI_ALL_DRIVERS=1 \
  NR_LD=$LIB/libld.so sh gexec-va.sh rootfs-full \
  $R/usr/bin/gst-inspect-1.0 \
  /usr/lib/aarch64-linux-gnu/gstreamer-1.0/libgstvaapi.so 2>&1; echo HOST_RC=\$?'"
```

## 原始日志与组件哈希

- [`81-va-probe.log`](81-va-probe.log)
- [`81-vainfo.log`](81-vainfo.log)
- [`81-gst-inspect.log`](81-gst-inspect.log)
- [`81-components.sha256`](81-components.sha256)

## 下一步

1. 实现 VA surface/context/buffer 的 Rust 状态机，并让 `vaBeginPicture` /
   `vaRenderPicture` / `vaEndPicture` / `vaSyncSurface` 跑通。
2. 把压缩 access unit 接到现有 `video-protocol` 独立 ring；app 侧复用已实测的
   `AMediaCodec` 解码线程，不新增解码进程。
3. 输出 surface 再接 `ExportSurfaceHandle` / dma-buf / AHB，先允许 CPU 拷贝退化路径。
4. 用真实 H.264/HEVC 文件和 GStreamer `vaapih264dec` / `vaapih265dec` 验证，不以
   element 注册或合成彩条代替解码验收。
