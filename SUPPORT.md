# 支持

## 先读这几份

| 问题 | 文档 |
|---|---|
| 我这台机器能不能跑？ | [`docs/RUNNING.md`](docs/RUNNING.md) §2 的三条硬前提 |
| 每个运行器能跑什么？ | [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) |
| 怎么装、怎么起会话？ | [`docs/RUNNING.md`](docs/RUNNING.md) |
| 为什么快、快多少？ | [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) |
| 什么做不到？ | [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) |
| loader 是怎么工作的？ | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) |

## 怎么提问

开 issue 用 bug 模板。一个能处理的报告包含：设备属性、完整命令、完整输出、
`files/linux/husk/assets.version` 的内容。只写「跑不起来」的报告无法复现，也无法处理。

## 不在支持范围

- 三条硬前提不满足的设备：那不是 bug，是这台机器跑不了。
- `root`、解 Bootloader、脱离应用沙箱运行。
- 参考发行版之外的发行版：loader 的补丁按 glibc 版本断言，换树要重新验证。
- Termux：缺依赖，不是脚本能补的（[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) §3）。
