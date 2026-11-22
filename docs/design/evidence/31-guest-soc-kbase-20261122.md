# evidence 31 — guest 侧 SoC 直连能力实测（kbase / 节点 / 厂商 EGL）

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB 页
时间：2026-11-22 21:26–21:30（CST）
进程上下文：**guest**（glibc 用户态，经 Husk loader 启动，`untrusted_app` 域、app uid u0_a302）
工具：`runtime/crates/gpu-probe`（纯 Rust，无依赖，aarch64-linux-gnu，`nr-tools check-elf` 4K/16K/64K 全过）

复现命令：

    # 构建（主机）
    cd runtime && NDK=… 
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
    RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu -C link-arg=--sysroot=/tmp/gccroot \
      -C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
      -C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
    cargo build --release --target aarch64-unknown-linux-gnu -p gpu-probe

    # 进 guest（注意：参数用相对路径，见下「已知坑」）
    adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe nodes'
    adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe enc'
    adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe ver'
    adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe nrscan'

## 1. 设备节点（guest 视角，FACT）

| 节点 | 结果 |
|---|---|
| `/dev/mali0` | **rw 打开成功**（fd=4） |
| `/dev/kgsl-3d0` | 不存在（ENOENT） |
| `/dev/dri/card0` | EACCES(13)；目录可见 |
| `/dev/dri/renderD128` | 不存在 |
| `/dev/ion` | **EACCES(13)**（注意：与 evidence 26 的 app 侧结论冲突，见下） |
| `/dev/dma_heap/*` | 目录不存在 |
| `/dev/snd/controlC0`、`/dev/snd/pcmC0D0p` | EACCES(13)，目录不可列 |
| `/dev/binder` | rw 打开成功 |
| `/dev/ptmx` | rw 打开成功 |

`/dev/ion` 的冲突要先讲清：evidence 26 是 **app 自己（bionic 进程）** 打开成功，
本表是 **guest 子进程（glibc，同 uid 同域）** 打开失败。两者 uid 和 SELinux 域相同，
所以差异只能来自 open 的 flag 或该次探测的写权限要求 —— 目前是 **UNKNOWN**，
需要一次固定 flag（O_RDONLY / O_RDWR 各一次）的对照实验才能定论。

## 2. kbase（Mali 内核驱动）能从 guest 里问到什么（FACT）

### 2.1 命令字编码必须逐字节对

kbase 按**完整命令字**（含 size 与方向位）匹配，不是只按 (type, nr)。
实测（同一进程、同一 fd）：

    cmd=0xc0008000 dir=RW size=0  ask 11.13 -> errno=1   (驱动不认)
    cmd=0xc0048000 dir=RW size=4  ask 11.13 -> 0         (认)
    cmd=0xc0088000 dir=RW size=8  ask 11.13 -> errno=25  (ENOTTY)

所以 `VERSION_CHECK` = `_IOC(_IOC_READ|_IOC_WRITE, 0x80, 0, 4)`。
**size=0（若干公开的 kbase 头文件是这么写的）在本机是 ENOTTY。**

### 2.2 内核 kbase UAPI 版本 = 11.38

一次探测一个进程（kbase 对同一 fd 只允许一次 VERSION_CHECK），把 0.0 传进去，
驱动把 out 半边填成自己的版本：

    asked 0.0    -> errno=0 out=11.38
    asked 11.13  -> errno=0 out=11.13   （回显，说明这台内核不校验版本号）
    asked 11.38  -> errno=0 out=11.38

结论：**UAPI 11.38**，且此 vendor 内核不校验请求的版本号（连 13.40 也返回 0）。
这点很重要：不能靠“版本号被拒”来判断驱动能力，只能靠真实命令逐条试。

### 2.3 设置进程标志：guest 里能建 kctx（FACT）

    cmd=0x40048001 dir=W size=4 (SET_FLAGS) -> 0

即 `SET_FLAGS` = `_IOC(_IOC_WRITE, 0x80, 1, 4)`，**从 guest 进程调用返回 0**，
说明 kbase 上下文（kctx）在 guest 里创建成功 —— 这是“Mesa 直连 kbase”路线的第一道门，
已经过了。

### 2.4 存在的命令面（nr / 方向 / size）

用零ed buffer 全量扫 nr 0..=40 × 多种 size × 三种方向，只列不是 ENOTTY 的：

| nr | 方向 | size | 结果 | 推测 |
|---|---|---|---|---|
| 0 | RW | 4 | EPERM（二次调用）/ 填 11.38 | VERSION_CHECK |
| 1 | W | 4 | **0** | SET_FLAGS |
| 5 | RW | 32 | ENOMEM | MEM_ALLOC（参数全 0 → 分配失败） |
| 6 | RW | 16 | EINVAL | MEM_QUERY（handle=0） |
| 16 | RW | 24 | EINVAL | 待定 |
| 21 | RW | 32 | EINVAL | 待定 |
| 22 | RW | 24 | ENOMEM | 待定（结构像 MEM_IMPORT） |
| 31 | RW | 16 | EINVAL | 待定 |

**未出现**：nr=2（教科书上的 JOB_SUBMIT，W/8）在所有 size 下都是 ENOTTY。
这是下一个必须查清的点：要么这台内核是 CSF 模型（提交走 QUEUE_* 系列），
要么 MTK 改了编号。在查清之前，“能建 kctx”还不能等同于“能提 job”。

## 3. 厂商 EGL 不能在 glibc guest 里加载（FACT + 结论）

    adb pull /vendor/lib64/egl/mt6893/libGLES_mali.so   (38 051 640 B, md5 e6a8bff9…)

`DT_NEEDED` 共 25 项，全是 bionic 侧的东西：

    libnativewindow libutils liblog libdmabufheap libion_mtk libion libged libgpu_aux
    libgpud libgralloc_extra libladder libgralloc_metadata libgralloctypes_mtk libcutils
    libz arm.graphics-V1-ndk_platform android.hardware.graphics.mapper@4.0 libgralloctypes
    libhidlbase libhardware libc++ libc libm libdl

其中 `libc.so` 是 bionic 的 libc（不是 guest 的 `libc.so.6`），`libc++.so`、`libhidlbase.so`
也都绑在 bionic 的运行模型上。**结论（判断，依据是上面的依赖表）：**
在一个 glibc 进程里 dlopen 厂商 EGL，等于同时装两套 libc 运行模型，不成立。
所以“guest 内直接加载厂商 libGLES_mali.so”这条路线**关闭**，不再尝试。
厂商 EGL 只可能活在 bionic 进程里，而那就是 2.0 的 virgl 式桥，与目标 C5 冲突。

## 4. 已知坑（下次别再踩）

* `gexec.sh` 的目标必须写**相对路径**（`usr/bin/foo`），不能写绝对路径：loader 打开主程序
  走的是宿主路径，`/usr/bin/foo` 在宿主不存在 → `cannot open shared object file`。
* `errno` 在 ioctl **成功**之后仍有上一次失败的值，日志会显示 `OK … errno=1`。
  探测工具里已改成调用前清零（`__errno_location()`）。
* 一个 fd 只能做一次 VERSION_CHECK；同一 fd 再问别的版本会 EPERM，看起来像“版本不支持”。

## 5. 对目标的影响

1. C5 的 GPU 路线**只能**是：guest 进程 → `/dev/mali0` → Mesa panfrost/panvk（kbase 后端）。
   已经证明这条链的第一段（打开设备、建 kctx）在 guest 里可行。
2. 原文把联发科写成 `/dev/kgsl-3d0` 是错的（那是高通）；本机实测只有 `/dev/mali0`。
3. 原文写“把 /dev/dri 接给 guest Mesa”在本机不成立（无 renderD128，card0 打不开）；
   dma-buf 的来源要按实测（`/dev/ion` 的访问权待复核，`/dev/dma_heap` 不存在）。
