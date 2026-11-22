# 设计文档索引

Husk 运行时的设计与证据。产品介绍见 [../../README.md](../../README.md)，
机制说明见 [../ARCHITECTURE.md](../ARCHITECTURE.md)，实测数字见 [../RESULTS.md](../RESULTS.md)。

| 文件 | 内容 |
|---|---|
| [runtime.md](runtime.md) | **权威方案**：为什么只有这一条路、分层、性能预算、验收清单、剩余工作 |
| [capability-baseline.md](capability-baseline.md) | 设备能力与权限边界实测（RMX3357 / Android 13 / 4.19） |
| [device-matrix.md](device-matrix.md) | Android 16 / GKI 6.6 实测矩阵 |
| [proot-comparison.md](proot-comparison.md) | 与 PRoot 的同条件对照（syscall / 路径 / exec 三张表） |
| [evidence/](evidence/) | 原始设备输出 01–25，编号被方案与其他文档引用 |
| [history/](history/) | 定案前的探索记录（候选枚举、原型、决策日志），仅存档 |

## 写作规范

- 结论必须标 FACT / INFERENCE / UNKNOWN，未实测不写 PASS。
- 每个数字都要能指回 `evidence/` 里的设备输出。
- 与 [runtime.md](runtime.md) 冲突的描述一律作废，先改那份。
