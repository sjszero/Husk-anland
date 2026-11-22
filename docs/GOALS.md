# 目标（Husk）

> 这份文件是目标的**权威规格**。会话里的 goal 对象文本不可编辑，所以每次
> 恢复/重建 goal 时以本文件为准。改动目标就改这里。

最后更新：2026-11-22

## 总目标

把 Husk（纯 Rust 原生 Linux 运行时）在 RMX3357（Android 13 / kernel 4.19.191 /
4 KiB 页 / arm64）上端到端跑通，并推进到能替代 LinuxHub 2.0 的产品化程度，
同时把"原生直连 SoC"做成正式目标。

约束：不 root、不解 BL、不开虚拟化；运行时零 syscall 拦截（shim 只在必要处
interpose）；能 Rust 就 Rust（零 Java / 零 Python）；不改动用户已有数据。

## A. 运行时核心

1. **SIGSYS 路径打通**：捕获并仿真被拦 syscall。已定位 `kill(2)` 属于不可捕获档
   （连 handler + ptrace 都接不住），只能靠 interpose 让调用不落内核。
   验收：该 ROM 的 `untrusted_app` 域里 guest `bash -i` 能交互运行 10 分钟不死。
2. **壳应用生命周期与渲染**：黑屏（`laid_out` 一直 0、`onNativeWindow` 未回调）修好；
   旋转 / 软键盘 / 切后台 / Surface 重建不重启会话；`vim` 全屏可用；僵尸与重启风暴零复现。
3. **页大小无关**：构建期 over-align（`max-page-size ≥ 0x4000`），运行期读 `AT_PAGESZ`，
   `nr-tools check-elf` 体检，禁止硬编码 4096。见 [design/page-size.md](design/page-size.md)。

## B. 真实场景验证

4. 用 **LinuxHub 2.0 的同一份 Ubuntu 24.04 rootfs** 验证，且**只读**它的数据目录：
   从仓库可复现地产出，或由用户导出一次；统一复制成 Husk 自己的副本再跑。
   验收：GNOME/常用工具能起，而不只是 bash。

## C. 原生直连 SoC（与 A/B 同级）

5. **GPU**：不额外加一层加速层，直接从 guest 打开厂商内核节点（kgsl / mali / drm），
   把 `/dev/dri` 与 dma-buf 接到 Mesa。验收：guest 里 `eglinfo`/`glxinfo` 报出真 GPU，
   GUI 走硬件渲染（对比软件渲染帧率）。
6. **显示**：guest X11/Wayland 面 ↔ Android Surface 的零拷贝桥（Rust 重写现有 doro/X11 方案
   或更好方案）。验收：GNOME 窗口可交互、拖动不掉帧、旋转正确。
7. **音频**：guest ALSA/PulseAudio ↔ Android AudioFlinger/AAudio 的最小桥，低延迟优先。
   验收：声卡枚举可见、能出声、音量可控。
8. 以上桥接全部 Rust 实现；天玑 / 骁龙两类 SoC 的探测与回退策略写成**代码里的表**，
   不是口头约定。

## D. 工程纪律

9. 每步有可复现命令与证据文件（`docs/design/evidence/`、`docs/STATUS.md`）；
   结论只按实测下；单台旧设备的问题不外推到所有设备。

## 附：怎么拿到 2.0 的 rootfs（不碰它的数据）

FACT：`com.linuxhub.myth` 不是 debuggable 包，`run-as` 直接拒绝（`package not debuggable`），
设备无 root，所以**读不到** `/data/user/0/com.linuxhub.myth/files/...`。这与签名无关，
是 Android 的沙箱。

不需要读它：2.0 的 rootfs 是可复现的（`linuxhub-repo/OFFLINE_ASSETS.md`）：

- `app/src/main/assets/rootfs/ubuntu-base-24.04.4-base-arm64.tar`（Ubuntu 官方 base）
- `app/src/main/assets/offline/bootstrap-arm64.tar`
  （348,477,440 B，sha256 `776e5ffe4d8225c6c67aca0728799058a2c8826f4f4865f528ec315950ec8db9`，
  598 个包，锁在 `tools/offline/offline-bootstrap.lock.json`）
- 生成器：`linuxhub-repo/tools/offline/build_offline_bootstrap.py`

两条路：
1. 在这些脚本旁边跑一遍 → 得到**字节等同**的产物，复制进 Husk 私有目录（推荐，零风险）；
2. 用户在 2.0 里导出一次 tar → 我只读解包，原目录不写。
