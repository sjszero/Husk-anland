# Evidence 50 — AHardwareBuffer dma-buf 跨进程导入 PanVK 并由 GPU 访问

日期：2026-11-23  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB / arm64  
包：`com.linuxhub.husk`（普通应用域，UID `u0_a302`）  
rootfs：`/data/user/0/com.linuxhub.husk/files/rootfs-full`  
APK sha256：`4996f71a59b46ba729b55e76bff5b854d12372f45109bef5f6b12a4d8d3dbce2`  
`libhuskapp.so` sha256：`f9f74d7d20bd6a2d6c11732dec0f365d433896231338246f23190ab86ef2dd54`  
`vulkan-probe-v9` sha256：`a9f4cc15f324743a44e094ac79296d1fbb4c5e3292d3929c7f3475fd2cdb61d5`

## 结论

- **FACT**：app 进程分配的真实 AHardwareBuffer，其 native handle 经 Unix socket 跨进程传到
  glibc guest；guest 在 `SCM_RIGHTS` 控制消息中收到 3 个 fd，其中 fd 8 是 dmabuf。
- **FACT**：PanVK 对
  `VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT` 的 buffer 能力报告为：
  `externalMemoryFeatures=0x6`、`compatibleHandleTypes=0x201`。
- **FACT**：guest 创建带 `VkExternalMemoryBufferCreateInfo` 的 buffer 成功；
  对 dma-buf 调用 `vkAllocateMemory(pNext=VkImportMemoryFdInfoKHR)` 成功，
  得到 `VkResult=0`，memory type 2，size 32768。
- **FACT**：`vkBindBufferMemory` 成功。随后现有 GPU compute 路径写入并回读
  `0x4855534b`，返回 `PASS`，证明不是只有“kbase 接受 ioctl”，而是 GPU 确实访问了导入后的
  dma-buf 后备存储。
- **UNKNOWN / 未验证**：这次仍是 **buffer** 导入，不是 AHB 的 2D image modifier 导入。
  要成为窗口渲染面，还必须解决 `VkExternalMemoryImageCreateInfo` /
  `VkImageDrmFormatModifierExplicitCreateInfoEXT`、AHB 的 format/modifier 匹配和布局。
- **UNKNOWN / 未验证**：同一块 AHB 经 `ANativeWindow_queueBuffer` 交给 SurfaceFlinger、
  render-done fence、旋转 / DPI / 多窗口 / 10 分钟泄漏均未做。不能把本条写成“显示零拷贝完成”。

## 数据路径

```text
AHardwareBuffer_allocate (app/bionic)
  -> native_handle 中扫描出 /dmabuf fd
  -> AHardwareBuffer_sendHandleToUnixSocket
  -> AF_UNIX SCM_RIGHTS
  -> glibc guest recvmsg
  -> VK_EXT_external_memory_dma_buf +
     VK_KHR_external_memory_fd
  -> vkAllocateMemory(import fd)
  -> vkBindBufferMemory
  -> GPU compute write + readback
```

路线仍是 app 分配、guest 直接导入、GPU 直接访问；没有 virgl、没有独立 GPU 进程、没有
ptrace/root/虚拟化。

## 复现命令

### 1. 构建

```sh
NDK=$NDK
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang \
  cargo build --release --target aarch64-linux-android --manifest-path app-rs/Cargo.toml
cp app-rs/target/aarch64-linux-android/release/libhuskapp.so \
  app/app/src/main/jniLibs/arm64-v8a/libhuskapp.so
cd app
JAVA_HOME=${JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home} \
  ./gradlew :app:assembleDebug -q
cd ..

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS='-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot -C link-arg=-fuse-ld=lld' \
  cargo build --release -p vulkan-probe --target aarch64-unknown-linux-gnu \
  --manifest-path runtime/Cargo.toml
```

### 2. 启动 app bridge 并运行 guest

```sh
adb -s UKHMMB8PYTGECMNV push \
  runtime/target/aarch64-unknown-linux-gnu/release/vulkan-probe \
  /data/local/tmp/vulkan-probe-v9
adb -s UKHMMB8PYTGECMNV shell "run-as com.linuxhub.husk sh -c '
  cat /data/local/tmp/vulkan-probe-v9 > files/vulkan-probe-v9
  chmod 755 files/vulkan-probe-v9
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
  timeout 60 sh gexec2.sh rootfs-full \$F/vulkan-probe-v9 \
    --ahb-socket \$F/ahb-bridge.sock 2>&1
'"
```

## guest 原始输出（dma-buf 段）

```text
vk|dmabuf|socket|PASS|/data/user/0/com.linuxhub.husk/files/ahb-bridge.sock
vk|dmabuf|recv|bytes=356 control_len=32
vk|dmabuf|fd_count|3
vk|dmabuf|fd|fd=7 target=anon_inode:gralloc_extra
vk|dmabuf|found|fd=8 target=/dmabuf:G:pid [22503]
vk|dmabuf|external_props|features=0x6 export=0x201 compatible=0x201
vk|dmabuf|buffer|create VkResult=0 ptr=0x55921e9710
vk|dmabuf|requirements|size=32768 align=64 bits=0x7
vk|dmabuf|import|vkAllocateMemory VkResult=0 type=2 size=32768 fd=8
vk|dmabuf|bind|VkResult=0
vk|compute|PASS|shader words=98
vk|compute|PASS|pipeline
vk|compute|PASS|descriptor
vk|compute|PASS|commands
vk|compute|PASS|queue_submit+fence
vk|compute|PASS|readback=0x4855534b
vk|dmabuf|gpu_access|PASS|compute wrote and read 0x4855534b
vk|dmabuf|teardown|memory/buffer/fds released
vk|teardown|PASS|device/instance/loader released
```

## app 原始日志（bridge 段）

```text
I Husk : ahb|allocate|rc=0 ptr=0xb400007aa5448800
I Husk : ahb|describe|64x64x1 format=1 usage=0x300 stride=128
I Husk : ahb|allocated|handle version=12 fds=3 ints=76
I Husk : ahb|allocated|fd[0]=105 valid=true target=anon_inode:gralloc_extra
I Husk : ahb|allocated|fd[1]=106 valid=true target=/dmabuf:G:pid [22503]
I Husk : ahb|allocated|fd[2]=107 valid=true target=/dev/ashmemc9cae759-c33d-400e-8b08-d93f8623edaa
I Husk : ahb|bridge|LISTEN|/data/user/0/com.linuxhub.husk/files/ahb-bridge.sock
I Husk : ahb|bridge|SEND|rc=0
```

同轮 `vulkan-probe` 的 compute、离屏 graphics、GPU 像素回读也全部 PASS；本条新增的是外部
dma-buf 导入和导入后的 GPU 访问，不是重复之前的本地 buffer 测试。
