# 构建依赖（我们要编译/链接的外部组件）

> 只登记**实际参与构建**的外部源码与版本。参考笔记、源码阅读记录不入库也不在此列。

| 组件 | 版本 / commit | 用途 |
|---|---|---|
| Mesa（PanVK / Panfrost / Zink） | 以 glibc 交叉构建的 PanVK 分支为准，commit 见 `runtime/` 构建脚本 | guest 内 GPU 用户态（Vulkan / EGL / GL） |
| glibc | Ubuntu 24.04，2.39 | guest 用户态与 loader 底座 |
| Ubuntu 24.04 base + GNOME 桌面资产 | LinuxHub 2.0 离线链（`assets/offline/*.tar`） | guest rootfs |
| Android NDK | r29 | 交叉编译 Rust 与 C ABI 垫片 |
| Rust | stable + `aarch64-unknown-linux-gnu` / `aarch64-linux-android` | 全部我们自己的代码 |

不改上游、不 vendoring：需要改动时以补丁形式放 `docs/design/patches/`，并写明对应提交。
