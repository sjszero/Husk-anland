# 目标（Husk）

> 本文件是目标的**权威规格**。会话里的 goal 对象不可编辑，重建 goal 时以本文件为准；
> 改目标就改这里，再把下面的「粘贴块」整段贴进去。

最后更新：2026-11-22（v4：B 段只认 LinuxHub 2.0 的资产，删掉 1.2.0 引用；C 段按 guest 侧实测重写）

---

## 粘贴块（可直接复制进 goal 字段）

总目标：Husk（纯 Rust 原生 Linux 运行时，仓库 /Users/l/Documents/Codex/2026-08-10/husk）在 RMX3357（MT6893 / Dimensity 1200 / Mali-G77，Android 13 / kernel 4.19.191 / 4 KiB 页 / arm64）上端到端跑通，做到能替代 LinuxHub 2.0 的产品化程度；原生直连 SoC（GPU / 显示 / 音频）与运行时核心同级，必须做完。

硬约束：不 root、不解 BL、不开虚拟化；运行时零 syscall 拦截（shim 只在必要处 interpose，不再扩大拦截面）；能 Rust 就 Rust（零 Java、零 Python、零 C，只有内核 ABI 强制的裸汇编 stub 例外）；绝不读写 com.linuxhub.myth 的任何数据，只用自己私有目录里的副本；单台旧设备的问题不外推到所有设备；结论只按实测下，推断必须标注。每次正式改动都提交本地并推 GitHub。

【A. 运行时核心】
1）SIGSYS 全链路打通。已实测：openat2(437) 是可捕获 TRAP；kill(2)/tkill/tgkill 属不可捕获档，只能 interpose 让调用不落内核（interpose 已生效，但当前死在 handler 的 frame 定位分支；4.19 上 si_call_addr 不填）。要求：handler 入口无条件写固定路径日志，按真实 ucontext 偏移定位帧，不靠特征扫描猜。验收：untrusted_app 域里 guest bash -i 连续交互 10 分钟不死，日志里能看到被仿真成 ENOSYS 的具体 syscall 号。
2）壳应用（app-rs，纯 Rust NativeActivity）生命周期与渲染完整。黑屏已定位并修好（原地覆盖平台回调表）；旋转 / 软键盘 / 切后台 / Surface 重建不重启会话；vim、htop 全屏可用；僵尸与重启风暴零复现（唯一 reaper + PR_SET_CHILD_SUBREAPER + 点了才重启）。终端渲染补齐完整 ANSI（提示符、光标、滚动回看、SGR 颜色、UTF-8 宽字符），删掉开发期自检代码。
3）页大小无关。构建期 over-align（max-page-size ≥ 0x4000）、运行期读 auxv AT_PAGESZ、nr-tools check-elf 体检；代码禁止字面量 4096。验收：4K 与 16K 两种内核假设下都通过体检，guest 侧 ELF 对齐有表可查。

【B. 真实场景验证】
4）用 **LinuxHub 2.0** 的完整 rootfs 验证，唯一来源是 /Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/app/src/main/assets/：rootfs/ubuntu-base-24.04.4-base-arm64.tar（31 MB）→ offline/bootstrap-arm64.tar（349 MB）→ offline/gnome-24.04-arm64.tar（532 MB）→ offline/vendor-apps-arm64.tar（403 MB），外加 offline/mesa-for-android-container_27.0.0-devel-20261025-r2_ubuntu_noble_arm64.tar（12 MB）、desktop/（GNOME 默认值、图标、壁纸、会话总线配置）、x11/xkb-data.tar。**不引入任何 1.2.0 的资产**（那份 557 MB 的 plasma6 包不在本项目范围内）。解到 Husk 私有目录的独立副本（files/rootfs-full），绝不读写 com.linuxhub.myth。验收：不只是 bash——GNOME 桌面能起，浏览器、文件管理器、终端可用。同时打通 libc 内部调用（构建期槽位方案），否则 apt/dpkg/locale 仍读宿主文件。

【C. 原生直连 SoC】本机已实测（2026-11-22，guest 侧，见 evidence 31）：/dev/mali0 可从 guest rw 打开；kbase UAPI = 11.38；VERSION_CHECK 用 _IOC(RW,0x80,0,4) 编码才被识别；SET_FLAGS _IOC(W,0x80,1,4) 在 guest 里返回 0（kctx 建成）；MEM_ALLOC/MEM_QUERY 等命令存在；/dev/kgsl-3d0 不存在、/dev/dri 打不开、/dev/dma_heap 不存在、/dev/snd 打不开、/dev/ion 在 guest 侧 EACCES（待复核）。厂商 libGLES_mali.so 有 25 个 bionic DT_NEEDED（libc.so/libc++/libhidlbase/gralloc…），**不能在 glibc guest 内加载**，此路线已排除，不再尝试。
5）GPU：唯一路线 = guest 进程自己打开 /dev/mali0，走 Mesa panfrost/panvk 的 kbase 后端，不经任何中转服务、不留独立 GPU 进程。先查清这台内核的提交接口（教科书 nr=2 JOB_SUBMIT 实测为 ENOTTY，需确认是否为 CSF 或 MTK 改号），再出最小可测实现。验收：guest 内 eglinfo/glxinfo 报出 Mali-G77 真实 renderer；同一场景硬件 vs llvmpipe 帧率对比表；不崩、可降级。
6）显示：guest 渲染面与 Android Surface 零拷贝。dma-buf 来源按节点表实测选择（/dev/ion 访问权复核；无 /dev/dri），包成 AHardwareBuffer → ANativeWindow queueBuffer + render-done fence 交 SurfaceFlinger，全部 Rust。验收：窗口可交互、拖动不掉帧、旋转与 DPI 变化正确、多窗口正常、连续 10 分钟无泄漏。
7）音频：/dev/snd 打不开，只能在 app 进程内用 AAudio 开流；guest ↔ app 用独立 socketpair 传 PCM（不与输入/剪贴板争通道），Rust 实现。验收：guest 内可枚举输出设备、能出声、音量可控、延迟有实测数字并写明测量方法。
8）探测与降级：SoC 与节点能力写成代码里的表（按 /dev/mali0、/dev/kgsl-3d0、/dev/dri/renderD128、ro.hardware.egl、ro.hardware.vulkan、ro.board.platform 判定），探测失败必须有明确降级路径（软件渲染 + 日志说明原因），任何一条都不能崩。

【D. 工程纪律】
9）每一步留可复现命令与证据（docs/design/evidence/、docs/STATUS.md、docs/GOALS.md），事实与推断分开标注；对外结论别人用同样命令能复现。一项做完先上机验证再写"完成"，未验证就明确写未验证。收尾时设备息屏、会话不留僵尸。

---

## 附一：怎么拿到 2.0 的 rootfs（不碰它的数据）

FACT：`com.linuxhub.myth` 不是 debuggable 包，`run-as` 直接拒绝（`package not debuggable`），
设备无 root，所以**读不到** `/data/user/0/com.linuxhub.myth/files/...`。这与签名无关，是 Android 沙箱。

**唯一资产来源是 LinuxHub 2.0（FACT，2026-11-22 实测目录）**：

```
/Users/l/Documents/Codex/2026-08-10/linuxhub/LinuxHub-2.0/app/src/main/assets/
  rootfs/ubuntu-base-24.04.4-base-arm64.tar                    31 MB   ← 主用
  rootfs/ubuntu-base-26.04.1-base-arm64.tar.gz                 35 MB   ← 备选
  offline/bootstrap-arm64.tar                                 349 MB
  offline/gnome-24.04-arm64.tar                               532 MB   ← 主用
  offline/gnome-26.04-arm64.tar                               537 MB   ← 备选
  offline/vendor-apps-arm64.tar                               403 MB
  offline/mesa-for-android-container_27.0.0-devel-20261025-r2_ubuntu_noble_arm64.tar  12 MB
  offline/{bootstrap-packages.txt,gnome-packages.txt,vendor-packages.txt,vendor-dependencies.txt}
  desktop/            GNOME 默认值、图标、壁纸、zh_CN 翻译、会话总线配置、doro-fetch.sh
  desktop/profile/    2.0 的 GPU 分派脚本（doro-gpu-run.sh），作为对照基线
  x11/xkb-data.tar    X11 键盘数据
  runtime/arm64-v8a/{loader,proot,libandroid-shmem.so,libgetifaddrs_bridge.so,libtalloc.so.2}
  gpu/virgl/arm64-v8a/{libvirglrenderer.so,libepoxy.so,virgl_test_server_android}  ← 2.0 的 virgl 链，只作对照
```

**不使用 1.2.0 的任何资产**（`~/Downloads/LinuxHub-1.2.0-MediaTek-X11-compositor.apk`
里的 plasma6 包不在本项目范围）。

用法：直接复制进 Husk 私有目录（例如 `/data/data/com.linuxhub.husk/files/rootfs-full`），
**不读也不写** `com.linuxhub.myth`。

## 附二：2.0 的 GPU 现状（对照基线，不是要照抄）

- 判定逻辑：`DeviceGpuProbe.kt` —— 高通 Adreno + `/dev/kgsl-3d0` 可读写 → `zink`（Turnip）；
  联发科 + guest 内有 `virtio_gpu_dri.so`/`virpipe_dri.so` → `virgl`；否则 `software`。
- 实际执行：`desktop/profile/doro-gpu-run.sh`，GNOME Shell 本体固定 llvmpipe，硬件只给应用进程。
- 本机（MT6893 / Mali）走的就是 `virgl`：guest → virpipe → 主机侧 virgl server → vendor EGL。
  这条链里有一个独立服务进程和一段 virtio 传输，正是 Husk C5 要拿掉的部分。

## 附三：C 段的实测锚点（写代码前先看这张表）

| 事实 | 值 | 出处 |
|---|---|---|
| GPU 设备节点 | 只有 `/dev/mali0`（0666 root:root） | evidence 31 |
| kbase UAPI | 11.38（vendor 不校验请求版本） | evidence 31 |
| VERSION_CHECK 编码 | `_IOC(RW, 0x80, 0, 4)`；size=0 是 ENOTTY | evidence 31 |
| SET_FLAGS 编码 | `_IOC(W, 0x80, 1, 4)`，guest 内返回 0 | evidence 31 |
| 已知命令面 | nr 0(4,RW) 1(4,W) 5(32) 6(16) 16(24) 21(32) 22(24) 31(16) | evidence 31 |
| JOB_SUBMIT | nr=2 全 size ENOTTY —— 提交接口待查 | evidence 31 |
| `/dev/ion` | guest 侧 EACCES；app 侧曾成功 → 待复核 | evidence 26 / 31 |
| `/dev/dma_heap` | 不存在 | evidence 31 |
| `/dev/dri` | card0 EACCES，无 renderD128 | evidence 31 |
| `/dev/snd` | EACCES | evidence 31 |
| 厂商 EGL | 25 个 bionic DT_NEEDED → guest 内不可加载 | evidence 31 |
| `ro.hardware.egl` / `.vulkan` | `meow` / `mali` | evidence 26 |
