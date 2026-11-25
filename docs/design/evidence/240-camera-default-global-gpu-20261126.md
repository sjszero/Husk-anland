# 240 · 相机默认启用与初始终端 GPU 环境

日期：2026-11-26 02:25–02:34（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 /
4 KiB 页。验证包 `com.linuxhub.husk`，rootfs `files/rootfs-full`。
`com.linuxhub.myth.husk` 未启动、未停止、未替换资产。

## 1 · 本轮改动

- 相机从“测试 marker 才启动”改为默认启动。每次 App 启动都选择活动 rootfs
  （优先 `files/rootfs-full`，否则 `files/rootfs`），重建 V4L2 影子树，并监听
  `<rootfs>/run/husk/camera.sock`。
- 删除旧 `c81:N` udev 项，避免相机权限变化后留下过期节点。
- App 初始终端会话补上软件回退环境变量和 `MESA_GLES_VERSION_OVERRIDE=3.2`。
- 能力矩阵改写相机和输入的真实状态；本次重点核对相机与 GPU 初始环境。

## 2 · 相机不依赖 marker（FACT，真机）

测试前执行：

```text
run-as com.linuxhub.husk sh -c 'rm -f files/camera files/camera-guest'
```

随后 force-stop、冷启动 `com.linuxhub.husk/android.app.NativeActivity`。日志：

```text
camera|ROOTFS|/data/user/0/com.linuxhub.husk/files/rootfs-full
camera|list|id=0|facing=BACK|orientation=90|yuv=1920x1080,1872x864,1440x1080,1280x960,1600x720,1560x720|n=16
camera|list|id=1|facing=FRONT|orientation=90|yuv=1920x1080,1872x864,1440x1080,1280x960,1600x720,1560x720|n=16
camera|LISTEN|/data/user/0/com.linuxhub.husk/files/rootfs-full/run/husk/camera.sock
```

两个 marker 均不存在时 socket 仍出现：

```text
files/rootfs-full/run/husk/camera.sock
```

能力矩阵同轮输出：

```text
caps|camera|present=yes|husk=degraded|backend=V4L2 /dev/videoN -> Camera2 NDK|detail=ids=0,1
```

## 3 · 前后摄取帧（FACT，真机传输；画面内容 UNKNOWN）

在 guest 内用当前 App 初始终端会话的环境执行 `v4l2-test`。下面是最终 0.0.15 APK
（`versionName=0.0.15`，安装时间 02:32 CST）的结果：

```text
/dev/video0: rdev=81:0 driver=husk card=BACK
frames=5 bytes=2304000 fps=12.26
RESULT OK
```

```text
/dev/video1: rdev=81:1 driver=husk card=FRONT
frames=5 bytes=2304000 fps=12.79
RESULT OK
```

两者都使用 NV12、640×480、4 个 MMAP buffer，`DQBUF` 每次都拿到 460800 字节。
最终 0.0.15 构建复跑不止一次：先各取 5 帧得到 12.26 / 12.79 fps，随后各取 10 帧得到
16.44 / 17.42 fps。发布前同版构建另一轮 10 帧为 16.71 / 17.21 fps。这里记录
**12.26–17.42 fps** 的实测范围，不挑单次有利数字；传感器标称能力与端到端上限仍是 UNKNOWN。
这证明 socket → V4L2 shim → Camera2 的数据链路在工作。

主机侧把 PPM 拉回并检查：前后摄两张都是逐字节相同的全黑 640×480。设备当时处于
相机视野被遮挡的状态，因此**不能**用本轮图像判断传感器画面质量；画面内容仍是 UNKNOWN。
此前 evidence 91/93 已有真实非黑画面、JPEG 和录像记录，不能把本轮全黑图当成回归结论，也不能
把本次只验证传输写成“视觉验收通过”。

## 4 · 初始终端拿到 GPU 默认值（FACT，真机）

冷启动后 App 创建的初始 guest session：

```text
NR_ROOTFS=/data/user/0/com.linuxhub.husk/files/rootfs-full
__EGL_VENDOR_LIBRARY_FILENAMES=.../usr/share/glvnd/egl_vendor.d/50_mesa.json
VK_ICD_FILENAMES=.../usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
VK_DRIVER_FILES=.../usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
MESA_LOADER_DRIVER_OVERRIDE=zink
GALLIUM_DRIVER=zink
MESA_GLES_VERSION_OVERRIDE=3.2
```

这说明初始会话不再依赖登录 shell 才拿到 GPU 默认值。`HUSK_GPU=llvmpipe` 和
`NR_GPU_DEFAULT_OFF=1` 的代码路径也已在 App 初始终端实现。

## 5 · GPU 探针的意外结果（FACT，限定于独立测试树）

当前独立验证树 `files/rootfs-full` 里的 PanVK 不是 0.0.14 集成树里 evidence 232 使用的
`libvulkan_panfrost_x11.so` 版本，而是较早的 `libvulkan_panfrost_frist370_execva.so`。

- `gl-probe es3`，软件回退 `GALLIUM_DRIVER=llvmpipe`：**PASS**，renderer
  `llvmpipe (LLVM 20.1.2, 128 bits)`。
- `gl-probe es3`，zink/PanVK：SIGSEGV，未打印 renderer。
- `vulkan-probe`：SIGSEGV，未枚举到物理设备。
- 换成 `files/libld39.so`（旧 loader）后，zink/PanVK 同样 SIGSEGV。
- `gpu-probe nodes` 能打开 `/dev/mali0`，但不能证明 Vulkan/GL 探针通过。

因此本轮只能确认“默认环境变量确实已经注入初始会话”，**不能确认这个独立测试树的
OpenGL renderer 字符串或 Vulkan 枚举通过**。这不等同于 evidence 232 的集成树结果；
在独立树更新到同一套 PanVK 之前，状态记为 UNKNOWN。

## 6 · 口径更正

evidence 91/240 的源码和设备行为都表明：

- guest 的 `/dev/videoN` 是 V4L2 facade，`open()` 返回 shim 的 memfd；
- 帧来自 App 内 Camera2 `AImageReader`，经 socket 传给 shim；
- 这不是把真实 V4L2 节点 fd 直通给 guest。

所以硬件接口分类从旧的“直连 2 类（GPU、相机）/ 中转 2 类（音频、视频）”更正为：

```text
直连 1 类：GPU
中转 3 类：相机、音频、视频
窗口/显示：零拷贝链路单独描述
未做：传感器、GPS、USB、蓝牙、NFC；指纹不做
```

这是一次不利口径更正，不为了维持旧宣传而把 facade 写成真节点。

## 7 · 发布判断

- FACT：相机默认启动、前后摄取帧、能力矩阵、初始终端 GPU 环境均已在真机验证。
- FACT：本轮前后摄画面全黑，不能据此判断图像质量；GPU zink/PanVK 探针在独立树崩溃。
- INFERENCE：相机传输路径没有因为移除 marker 而失效；GPU 环境注入正常。
- UNKNOWN：独立测试树的 PanVK 版本为何与 evidence 232 不同、何时更新；相机全黑是否仅为
  物理遮挡；真实应用在 GLES 3.2 下的稳定性。
- 不能写：相机是真节点直连、GPU 在独立树已通过、相机本轮视觉验收通过。
