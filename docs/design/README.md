# 设计文档索引

Husk 运行时的设计与证据。产品介绍见 [../../README.md](../../README.md)，
机制说明见 [../ARCHITECTURE.md](../ARCHITECTURE.md)，实测数字见 [../RESULTS.md](../RESULTS.md)。

| 文件 | 内容 |
|---|---|
| [../PROJECT-MEMORY.md](../PROJECT-MEMORY.md) | 长期接手手册：目标、硬约束、路线变更、参考仓库与操作纪律 |
| [runtime.md](runtime.md) | 运行时架构：分层、性能预算、验收清单、剩余工作 |
| [capability-baseline.md](capability-baseline.md) | 设备能力与权限边界实测（RMX3357 / Android 13 / 4.19） |
| [device-matrix.md](device-matrix.md) | Android 16 / GKI 6.6 实测矩阵 |
| [proot-comparison.md](proot-comparison.md) | 与 PRoot 的同条件对照（syscall / 路径 / exec 三张表） |
| [evidence/](evidence/) | 原始设备输出 01–76，编号被方案与其他文档引用 |
| [reference-request-20261122.md](reference-request-20261122.md) | 用户提供的参考仓库原始清单（12 类、120 条，未审计） |
| [references.md](references.md) | 参考仓库真实路径、可用性、已拉取 commit 与借鉴结论 |
| [reference-audit-matrix.md](reference-audit-matrix.md) | 原始 12 类 / 120 条逐项审计矩阵（完整、保留重复项） |
| [reference-audit-20261123.json](reference-audit-20261123.json) | 与矩阵同源的机器可读审计快照 |
| [acquire-fence.md](acquire-fence.md) | producer / acquire fence 的已通过链路、fallback 与剩余 WSI 待办 |
| [audio-bridge.md](audio-bridge.md) | AAudio guest→app 音频桥、持续 PulseAudio 路径、610 秒长稳与实测延迟口径 |
| [mali-kbase-prior-art.md](mali-kbase-prior-art.md) | Mali kbase / PanVK / Panfrost 前人实现的源码级规格 |
| [history/](history/) | 定案前的探索记录（候选枚举、原型、决策日志），仅存档 |

## 写作规范

- 结论必须标 FACT / INFERENCE / UNKNOWN，未实测不写 PASS。
- 每个数字都要能指回 `evidence/` 里的设备输出。
- 与 [runtime.md](runtime.md) 冲突的描述一律作废，先改那份。
