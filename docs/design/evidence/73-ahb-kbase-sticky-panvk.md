# Evidence 73 — AHB dma-buf 的 kbase MEM_IMPORT / STICKY_RESOURCE_MAP 与 PanVK GPU 写回

日期：2026-11-23
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
包：`com.linuxhub.husk`（普通 `untrusted_app`）
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

## 组件哈希

```text
31905cc784c69dd8590983920e2381282c209543f9536bff341a0baebc07b49d  app-debug.apk
bc79e576569eecdae7dfd9d89b418a8c1c440d27782164923f95c6c6d5c9ab2f  libhuskapp.so
2a9bd65836cbc2a384baef81615b30d7f3162789562211a3c67893cb3c5247cd  gpu-probe-imp2
10a1fd47dc8eab25a3275ea47805146c50156337523938887c2b53e23f012ab7  vulkan-probe-imp2
```

## 结论

### 1. kbase MEM_IMPORT + sticky map

**FACT**：app 分配的 `64x64 RGBA8` AHardwareBuffer 经 `SCM_RIGHTS` 进入 glibc guest，
guest 扫描 native handle 找到 `/dmabuf:G:pid [...]` fd，并对 `/dev/mali0` 发出正确布局的
`MEM_IMPORT`：

```text
gpu|imp/mem_import|PASS|flags=0x4500f cookie=0x41000 va_pages=4
gpu|imp/mmap|PASS|size=16384 ptr=0x79b27da000
gpu|imp/sticky_map|PASS|errno=0 (ok) gpu_va=0x79b27da000
```

**FACT**：`STICKY_RESOURCE_MAP` 使用 `_IOW(0x80, 29, 16B)` = `0x4010801d`，结构为
`{ u64 count; u64 address; }`，`address` 指向 GPU VA 数组；对 SAME_VA import，数组元素是
`mmap` 后的 CPU address，不是 `MEM_IMPORT` 返回的 pre-mmap cookie。

**FACT**：kbase UMM mapping 不是权威 CPU 视图。探针改为直接 `mmap(dmabuf)` 写入，再通过
同一 dma-buf 读回：

```text
gpu|imp/write|PASS|red=64x64 before=0x9a5c0000
gpu|imp/dmabuf_read|PASS|direct_first=0xff0000ff
gpu|imp/kbase_view|INFO|kbase_first=0xff0000ff (CPU coherency is not required here)
```

**FACT**：app 侧收到 `D` 后读取同一 AHB 的 9 个点，全部为红，并通过现有
`ASurfaceControl` 路径上屏：

```text
ahb|bridge|ACK|byte=0x44
ahb|present|readback|red=9/9|unlock=0
ahb|present|PASS|SurfaceControl 1080x2400 from 64x64
ahb|bridge|PRESENT|true
```

截图：[`73-kbase-sticky-surface.png`](73-kbase-sticky-surface.png)（整屏纯红）。

### 2. PanVK GPU 写回同一个 AHB dma-buf

**FACT**：在正常 `rootfs-full` 根内，FristOneRR PanVK 枚举 `Mali-G77 MC9`，从
`/run/husk/ahb-bridge.sock` 收到同一个 AHB 的 dmabuf fd，完成 buffer import、GPU compute
写入和 readback：

```text
vk|dmabuf|found|fd=7 target=/dmabuf:G:pid [31460]
vk|dmabuf|fd_size|PASS|lseek_end=16384
vk|dmabuf|linear|PASS|match=9/9|64x64 stride=64 pitch=256 len=16384
vk|dmabuf|import|vkAllocateMemory VkResult=0 type=2 size=32768 fd=7
vk|dmabuf|bind|VkResult=0
vk|dmabuf|gpu_access|PASS|compute wrote and read 0x4855534b
```

**FACT**：同一 fd 再按 `DRM_FORMAT_MOD_LINEAR` 创建 external 2D image，GPU
`vkCmdClearColorImage` 后 staging 和直接 dma-buf CPU 视图均为 9/9 红色：

```text
vk|dmabuf|image_create|VkResult=0|64x64 pitch=256 modifier=LINEAR
vk|dmabuf|image_import|VkResult=0|type=2 size=16384 fd=7
vk|dmabuf|image_bind|VkResult=0
vk|dmabuf|image_readback|PASS|match=9/9 expected=0xff0000ff first=0xff0000ff
vk|dmabuf|image_dma_buf_view|PASS|match=9/9 expected=0xff0000ff first=0xff0000ff
vk|dmabuf|image_summary|PASS|LINEAR 2D image imported, cleared by GPU, read back 9/9 red
```

**FACT**：app 再次读取同一 AHB 为 9/9 红并实际上屏：

```text
ahb|bridge|ACK|byte=0x44
ahb|present|readback|red=9/9|unlock=0
ahb|present|PASS|SurfaceControl 1080x2400 from 64x64
ahb|bridge|PRESENT|true
```

截图：[`73-panvk-gpu-write-surface.png`](73-panvk-gpu-write-surface.png)（整屏纯红）。

## 边界

- **已达成的验收**：app 分配 AHB → dma-buf fd → guest `MEM_IMPORT` → sticky GPU mapping；
  guest CPU 和 guest PanVK GPU 都能写同一后备存储；app 能回读并零拷贝上屏。
- **未达成的验收**：本轮测试 AHB 是 `64x64 RGBA8 LINEAR`，不是 MediaCodec 解码输出的
  `AIMAGE_FORMAT_PRIVATE / YUV420_888` AHB。解码 AHB 的 plane layout、modifier 和采样路径仍需
  单独上机验证，不能把本证据写成硬解帧已经回导 guest。
- **未达成**：guest GL/X11/Wayland WSI、swapchain、Chromium / GStreamer / VA-API、AV 同步和
  10 分钟视频门禁不在本证据范围内。

## 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk
NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p gpu-probe --target aarch64-unknown-linux-gnu --manifest-path runtime/Cargo.toml

adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/gpu-probe /data/local/tmp/gpu-probe-imp2
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "
  cat /data/local/tmp/gpu-probe-imp2 > files/rootfs-full/usr/bin/gpu-probe-imp2
  chmod 755 files/rootfs-full/usr/bin/gpu-probe-imp2
  : > files/ahb-bridge
  rm -f files/rootfs-full/run/husk/ahb-bridge.sock
"'
adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start -n com.linuxhub.husk/android.app.NativeActivity
```

kbase 路径：

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/data/com.linuxhub.husk/files
  cd \$F
  timeout 60 env NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full \
    \$F/rootfs-full/usr/bin/gpu-probe-imp2 imp_socket /run/husk/ahb-bridge.sock 2>&1
'"
```

PanVK 路径（每次重启 app，因为 bridge 每次只服务一个连接）：

```sh
adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk sh -c "
  : > files/ahb-bridge
  rm -f files/rootfs-full/run/husk/ahb-bridge.sock
"'
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start -n com.linuxhub.husk/android.app.NativeActivity
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/data/com.linuxhub.husk/files; R=\$F/rootfs-full
  ICD=\$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
  export VK_ICD_FILENAMES=\$ICD VK_DRIVER_FILES=\$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink
  cd \$F
  timeout 60 env NR_LD=$LIB/libld.so sh gexec2.sh rootfs-full \
    \$F/vulkan-probe-imp2 --ahb-socket /run/husk/ahb-bridge.sock 2>&1
'"
```

完整日志见同目录的 `73-kbase-sticky-*.log` 与 `73-panvk-gpu-write-*.log`。
