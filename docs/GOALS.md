# 目标（Husk）

> 本文件是目标的**权威规格**。会话里的 goal 对象不可编辑，重建 goal 时以本文件为准；
> 改目标就改这里，再把下面的「粘贴块」整段贴进去。

最后更新：2026-11-22（v2：补齐原生直连 SoC，写入 MT6893 实测节点表）

---

## 粘贴块（可直接复制进 goal 字段）

总目标：Husk（纯 Rust 原生 Linux 运行时，仓库 /Users/l/Documents/Codex/2026-08-10/husk）在 RMX3357（MT6893 / Dimensity 1200 / Mali-G77，Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64）上端到端跑通，做到能替代 LinuxHub 2.0 的产品化程度；原生直连 SoC（GPU / 显示 / 音频）与运行时核心同级，必须做完。

硬约束：不 root、不解 BL、不开虚拟化；运行时零 syscall 拦截（shim 只在必要处 interpose，不再扩大拦截面）；能 Rust 就 Rust（零 Java、零 Python、零 C，只有内核 ABI 强制的裸汇编 stub 例外）；绝不读写 com.linuxhub.myth 的任何数据，只用自己私有目录里的副本；单台旧设备的问题不外推到所有设备；结论只按实测下，推断必须标注。

【A. 运行时核心】
1）SIGSYS 全链路打通。已实测：openat2(437) 是可捕获 TRAP；kill(2)/tkill/tgkill 属不可捕获档（进程内 handler 与 ptrace 都接不住），只能 interpose 让调用不落内核。现状 kill interpose 已生效（interposed +3），但退出码从 signal 31 变成 159，说明死在 handler 的 frame 定位分支；4.19 上 si_call_addr 不填，最后一个 pending entry 是 exit_group(94)。要求：handler 入口无条件写固定路径日志（先分清"handler 没被调用"还是"帧没找到"），按真实 ucontext 偏移定位帧，不再靠特征扫描猜。验收：untrusted_app 域里 guest bash -i 连续交互 10 分钟不死，日志里能看到被仿真成 ENOSYS 的具体 syscall 号。
2）壳应用（app-rs，纯 Rust NativeActivity）生命周期与渲染完整。黑屏必须定位并修好（laid_out 恒 0、onNativeWindow 未回调）；旋转 / 软键盘 / 切后台 / Surface 重建不重启会话；vim、htop 全屏可用；僵尸进程与重启风暴零复现（唯一 reaper + PR_SET_CHILD_SUBREAPER + 点了才重启，已验证零僵尸，需持续守住）；终端渲染补齐完整 ANSI（提示符、光标、滚回），删掉开发期自检代码。
3）页大小无关。构建期 over-align（max-page-size ≥ 0x4000），运行期读 AT_PAGESZ，nr-tools check-elf 体检，代码禁止字面量 4096。验收：4K 与 16K 两种内核假设下都能通过体检，guest 侧 ELF 对齐有表可查。

【B. 真实场景验证】
4）用 LinuxHub 2.0 的完整 rootfs 验证，只读源、只写自己的副本。本机现成资产在 /Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/app/src/main/assets/：offline/gnome-24.04-arm64.tar（508 MB）、rootfs/ubuntu-base-24.04.4-base-arm64.tar（30 MB）、offline/bootstrap-arm64.tar（333 MB）、offline/mesa-for-android-container_27.0.0-devel-20261025-r2_ubuntu_noble_arm64.tar（12 MB）、runtime/arm64-v8a/{loader,proot,libandroid-shmem.so,libgetifaddrs_bridge.so,libtalloc.so.2}；另有完整包 ~/Downloads/LinuxHub-1.2.0-MediaTek-X11-compositor.apk（856 MB，assets/rootfs/ubuntu-26.04-x11-plasma6-arm64.tar 557 MB）。解到 /data/data/com.linuxhub.husk/files/rootfs-*，绝不读写 com.linuxhub.myth。验收：不只是 bash —— Plasma 或 GNOME 桌面能起，浏览器 / 文件管理器 / 终端可用。

【C. 原生直连 SoC】本机实测节点（2026-11-22，普通应用域，见 evidence/26）：/dev/mali0 0666 可打开（Mali kbase，唯一可用 GPU 入口）；/dev/ion 0666 system:graphics 可打开（dma-buf 分配）；/dev/dri/card0 0660 打不开且无 renderD128；/dev/snd/* 0660 system:audio 打不开；/dev/kgsl-3d0 不存在。
5）GPU：guest 进程直接打开 /dev/mali0，最终让 guest 内 eglinfo/glxinfo 报出 Mali-G77 真实 renderer，GUI 走硬件渲染。两条候选路线各出最小可测实现再择一：(a) Mesa panfrost/panvk 走 kbase 后端，先测 kbase 版本是否被 pan_kmod 接受、崩在哪一步；(b) 在 guest 里直接加载厂商 /vendor/lib64/egl/libGLES_mali.so，用自写 Rust EGL 平台 + /dev/ion dma-buf 喂 gralloc。禁止再引入独立 GPU 服务进程、virtio-gpu/virpipe/virgl 中转或任何额外加速层（2.0 本机走的正是 virgl，本次要拿掉并给出前后对比数据）。验收：eglinfo/glxinfo 报 Mali-G77；同一场景硬件路径 vs llvmpipe 帧率对比表；不崩、可降级。
6）显示：guest X11/Wayland 表面与 Android Surface 零拷贝。本机无 /dev/dri，所以走 guest 渲染到我们分配的 /dev/ion dma-buf → 包成 AHardwareBuffer → ANativeWindow queueBuffer + render-done fence 交 SurfaceFlinger，全部 Rust（ash / ndk / libc）。验收：窗口可交互、拖动不掉帧、旋转与 DPI 变化正确、多窗口正常、连续 10 分钟无泄漏。
7）音频：guest ALSA/PulseAudio 与 AAudio 的最小桥（/dev/snd 打不开，只能在 app 进程内开 AAudio）。Rust 侧开流后读回设备真实 rate/channels 再回报 guest，音频走独立 socketpair 不与输入/剪贴板争通道。验收：guest 内可枚举输出设备、能出声、音量可控、延迟有实测数字并写明测量方法。
8）探测与降级：SoC 与节点能力写成代码里的表（按 /dev/mali0、/dev/kgsl-3d0、/dev/dri/renderD128、ro.hardware.egl、ro.hardware.vulkan、ro.board.platform 判定），探测失败必须有明确降级路径（软件渲染 + 日志说明原因），任何一条都不能崩。

【D. 工程纪律】
9）每一步留可复现命令与证据（docs/design/evidence/、docs/STATUS.md、docs/GOALS.md），事实与推断分开标注；对外结论别人用同样命令能复现。一项做完先上机验证再写"完成"，未验证就明确写未验证。收尾时设备息屏、会话不留僵尸。

---

## 附一：怎么拿到 2.0 的 rootfs（不碰它的数据）

FACT：`com.linuxhub.myth` 不是 debuggable 包，`run-as` 直接拒绝（`package not debuggable`），
设备无 root，所以**读不到** `/data/user/0/com.linuxhub.myth/files/...`。这与签名无关，是 Android 沙箱。

**本工作区内就有完整资产（FACT，2026-11-22）**：

```
/Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/
  app/src/main/assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar          30 MB
  app/src/main/assets/rootfs/ubuntu-base-26.04.1-base-arm64.tar.gz       33 MB
  app/src/main/assets/offline/bootstrap-arm64.tar                       333 MB
  app/src/main/assets/offline/gnome-24.04-arm64.tar                     508 MB
  app/src/main/assets/offline/gnome-26.04-arm64.tar                     512 MB
  app/src/main/assets/offline/vendor-apps-arm64.tar                     385 MB
  app/src/main/assets/offline/mesa-for-android-container_*.tar           12 MB
  app/src/main/assets/runtime/arm64-v8a/{loader,proot,libandroid-shmem.so,
                                        libgetifaddrs_bridge.so,libtalloc.so.2}
  app/src/main/assets/desktop/profile/doro-gpu-run.sh                    ← 2.0 的 GPU 分派脚本
  artifacts/release/LinuxHub-Myth-Prometheus.apk                        989 MB
```

另有 `~/Downloads/LinuxHub-1.2.0-MediaTek-X11-compositor.apk`（856 MB，内含
`assets/rootfs/ubuntu-26.04-x11-plasma6-arm64.tar` 557 MB）。

用法：直接复制进 Husk 私有目录（例如 `/data/data/com.linuxhub.husk/files/rootfs-full`），
**不读也不写** `com.linuxhub.myth`。

## 附二：2.0 的 GPU 现状（作为对照基线，不是要照抄）

- 判定逻辑：`DeviceGpuProbe.kt` —— 高通 Adreno + `/dev/kgsl-3d0` 可读写 → `zink`（Turnip）；
  联发科 + guest 内有 `virtio_gpu_dri.so`/`virpipe_dri.so` → `virgl`；否则 `software`。
- 实际执行：`desktop/profile/doro-gpu-run.sh`，GNOME Shell 本体固定 llvmpipe，硬件只给应用进程。
- 本机（MT6893 / Mali）走的就是 `virgl`：guest → virpipe → 主机侧 virgl server → vendor EGL。
  这条链里有一个独立服务进程和一段 virtio 传输，正是 Husk C5 要拿掉的部分。
