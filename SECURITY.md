# 安全策略

## 范围

Husk 在 Android 应用沙箱里跑一段「系统本身不信任」的 Linux 用户态（`untrusted_app`，无
root）。真正要守住的边界是 Android 那条：**运行时不得把它放宽一格**。最在意的报告：

- 让 guest 读到或写到它 rootfs 之外的路径；
- 让 loader 或 shim 拿到 Android 沙箱拒绝的特权（capabilities、切到别的 uid、mount、ptrace）；
- 让 guest 通过重新 `exec` 一个 shim 没覆盖的二进制，跑到宿主 Android 的路径空间里；
- 让一个应用的容器读到另一个应用的数据。

**guest 发行版自身的漏洞**（某个 Ubuntu 包、GNOME 之类）归上游，请报给上游。只有 guest 把
自己弄坏才触发的崩溃，是 bug 报告，不是漏洞。

## 怎么报

用本仓库的 GitHub Security Advisories 私密提交（Security → Report a vulnerability）。
疑似沙箱逃逸**不要**开公开 issue。

附上：设备与 Android 版本、`assets.version`、完整命令、观察到的现象。如果复现步骤能扛过
重启，请说明。

## 我们会做什么

- 7 天内确认收到。
- 给出定性：范围内（沙箱逃逸、路径逃逸、特权放大）或范围外（guest 内部崩溃、性能、缺功能）。
- 修复随 release notes 的「Security」一节发布；包含修复的版本发布后公开 advisory。

## 已知且接受

[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) 里列的一些行为看着像安全问题，其实是写明的设计
边界：静态二进制绕过 shim、`/proc` 是合成的、`setuid` 是模拟而不是真给权限。这些不是漏洞，
它们正是 Husk 不需要 root 的原因。
