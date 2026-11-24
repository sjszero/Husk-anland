# Evidence 51 — AHardwareBuffer 作为 LINEAR 2D image 导入 PanVK 并由 GPU 写入

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通应用域，`untrusted_app`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`

产物哈希：

```text
APK                 e3e74bc1a1c4ae472ca7152051883c5a1c252aab8c969c685b9bd557ded95075
libhuskapp.so       a131ba7e1849c2a6bf2deeeb0a2e6b7056337d96c1960d3bc7c31b90bad83913
vulkan-probe        5a93eb7f3e07d8c0c549ffcbcf36aa2b08ec8a685f7a7c2269c352e3e0e29a6a
```

## 结论

- **FACT**：app 侧用 `RGBA8 + GPU_SAMPLED + GPU_FRAMEBUFFER + CPU_READ + CPU_WRITE`
  分配 AHB；`AHardwareBuffer_describe()` 返回 `64x64x1`、format 1、usage `0x322`、stride 64。
- **FACT**：`AHardwareBuffer_lock()` 写入再读回 9 个角/边中点/中心 cookie，达到 9/9。
- **FACT**：该 AHB 的 dmabuf fd 经 `SCM_RIGHTS` 进入 glibc guest；`lseek(fd, 0, SEEK_END)`
  返回 16384，guest 直接 `mmap()` 后按 `stride * 4 = 256` 读回同一组 cookie，9/9 一致。
  因此这块新分配 AHB 的 dmabuf CPU 视图可按 `DRM_FORMAT_MOD_LINEAR`、offset 0、
  rowPitch 256 解释。
- **FACT**：PanVK 接受 `VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT` +
  `VkImageDrmFormatModifierExplicitCreateInfoEXT`，用 `DRM_FORMAT_MOD_LINEAR` 成功创建
  `64x64` RGBA8 2D image；image memory requirements 为 `size=16384 align=4096 bits=0x7`。
- **FACT**：用 `VK_EXT_external_memory_dma_buf` / `VK_KHR_external_memory_fd` 导入该 dmabuf，
  `vkAllocateMemory` 与 `vkBindImageMemory` 均返回 `VK_SUCCESS`。
- **FACT**：GPU 执行 `vkCmdClearColorImage` 清成红色后，staging buffer 回读 9/9 为
  `0xff0000ff`；同一 dmabuf 的直接 CPU `mmap` 也读回 9/9 红色。这证明 GPU 写进了 AHB
  的 dma-buf 后备存储，不只是创建了 Vulkan 对象。
- **FACT**：第一次实现把同一个 glibc fd 分别用于 import buffer 和 import image；buffer 导入
  成功，image 导入报 `invalid dmabuf size`。改成每次 import 各自 `dup(fd)` 后两条路径都在
  同一轮 probe 中成功。
- **UNKNOWN**：驱动内部为什么在重复使用同一个 fd 时走到该报错尚未定位，不能把“fd 被关闭”
  或某个具体驱动缺陷写成事实。当前结论只到“每次 import 独立 dup 可稳定复现 PASS”。
- **UNKNOWN / 未完成**：这不是 Surface 展示完成。尚无 `ASurfaceControl` /
  `ANativeWindow_queueBuffer`、present fence、SurfaceFlinger 合成、旋转、DPI、多窗口和
  10 分钟泄漏测试。

## 数据路径

```text
AHardwareBuffer_allocate(CPU_RW + GPU)
  -> AHardwareBuffer_lock: write 9 cookies
  -> native_handle: scan /dmabuf fd
  -> AF_UNIX: framed metadata + SCM_RIGHTS
  -> glibc guest: mmap(dmabuf), verify 9/9 at LINEAR pitch
  -> vkCreateImage(tiling=DRM_FORMAT_MODIFIER, modifier=LINEAR)
  -> vkAllocateMemory(import dup(fd))
  -> vkBindImageMemory
  -> vkCmdClearColorImage(red)
  -> staging readback: 9/9 red
  -> direct dma-buf mmap: 9/9 red
```

## 复现命令

```sh
cd $REPO
NDK=$NDK

CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
(cd app && ./gradlew :app:assembleDebug -q)

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu \
  --manifest-path runtime/Cargo.toml

adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-v18
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cat /data/local/tmp/vulkan-probe-v18 > files/vulkan-probe-v18
  chmod 755 files/vulkan-probe-v18
  [ ! -e files/ahb-bridge.sock ] || unlink files/ahb-bridge.sock
  : > files/ahb-bridge
'"
adb -s UKHMMB8PYTGECMNV install -r app/app/build/outputs/apk/debug/app-debug.apk
adb -s UKHMMB8PYTGECMNV logcat -c
adb -s UKHMMB8PYTGECMNV shell am force-stop com.linuxhub.husk
adb -s UKHMMB8PYTGECMNV shell am start \
  -n com.linuxhub.husk/android.app.NativeActivity

adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  F=/data/user/0/com.linuxhub.husk/files
  R=\$F/rootfs-full
  ICD=\$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
  export VK_ICD_FILENAMES=\$ICD VK_DRIVER_FILES=\$ICD
  export PANVK_DEBUG=startup PANVK_KBASE_DVFS=none
  export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink
  cd \$F
  timeout 60 sh gexec2.sh rootfs-full \$F/vulkan-probe-v18 \
    --ahb-socket \$F/ahb-bridge.sock 2>&1
'"
```

## 关键原始输出

```text
vk|dmabuf|meta|PASS|64x64 stride=64 format=1 usage=0x322
vk|dmabuf|fd_count|3
vk|dmabuf|found|fd=8 target=/dmabuf:G:pid [25699]
vk|dmabuf|fd_size|PASS|lseek_end=16384
vk|dmabuf|linear|PASS|match=9/9|64x64 stride=64 pitch=256 len=16384 ...
vk|dmabuf|external_props|features=0x6 export=0x201 compatible=0x201
vk|dmabuf|buffer|create VkResult=0
vk|dmabuf|import_fd|PASS|fd=6 source_size=16384
vk|dmabuf|import|vkAllocateMemory VkResult=0 type=2 size=32768 fd=8
vk|dmabuf|bind|VkResult=0
vk|dmabuf|gpu_access|PASS|compute wrote and read 0x4855534b
vk|dmabuf|image_create|VkResult=0|64x64 pitch=256 modifier=LINEAR
vk|dmabuf|image_requirements|size=16384 align=4096 bits=0x7
vk|dmabuf|image_import|VkResult=0|type=2 size=16384 fd=8
vk|dmabuf|image_bind|VkResult=0
vk|dmabuf|image_submit|submit=0 wait=0
vk|dmabuf|image_readback|PASS|red=9/9 first=0xff0000ff
vk|dmabuf|image_dma_buf_view|PASS|red=9/9 first=0xff0000ff
vk|dmabuf|image_summary|PASS|LINEAR 2D image imported, cleared by GPU, read back 9/9 red
```

app 侧关键日志：

```text
I Husk : ahb|describe|64x64x1 format=1 usage=0x322 stride=64
I Husk : ahb|linear|cpu_view|match=9/9 ... unlock=0
I Husk : ahb|linear|cpu_contract|PASS
I Husk : ahb|bridge|META_SEND|rc=Ok(())
I Husk : ahb|bridge|SEND|rc=0
```

## 下一步

1. 把这套 image import 收敛成产品 API，不再由一次性 probe 内联。
2. 接 `ASurfaceControl` / `ASurfaceTransaction_setBuffer`：app 持有 AHB，guest 将同一 dmabuf
   GPU 渲染，app 用 render fence 提交到 SurfaceFlinger。
3. 实测旋转、DPI、多窗口、连续 10 分钟无泄漏；在此之前“显示零拷贝”仍是未完成。
