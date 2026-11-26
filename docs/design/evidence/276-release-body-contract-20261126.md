# 276 · Release 正文统一为版本页格式

日期：2026-11-26（CST）。

## 0 · 结论

FACT：更新前，32 个 GitHub Release 的标题和 tag 已经统一，但正文仍有三套页头：

| 类型 | 数量 | 例子 |
|---|---:|---|
| 项目总介绍开头 | 16 | `Husk 在 Android 上跑一个真实的 Linux 用户态…` |
| `> 主题：**…**` 横幅 | 6 | `v0.0.19`–`v0.0.23`、`v0.0.25` |
| 直接进入本版变更 | 10 | `v0.0.18`、`v0.0.24`、`v0.0.26`–`v0.0.32` |

FACT：更新前另有 11 个页面混用 `###` 章节，`v0.0.2` 还在正文中重复了
`## [0.0.2] - 2026-11-24`。因此用户进入不同版本的 Release 页面时，会看到不同的标题层级和
页头写法。

FACT：本轮把 32 个 Release 正文统一为：

```text
## 概要

只属于本版的结论

## 修复 / 变更 / 验证 / 边界 / 产物
```

项目总介绍不再复制到 Release 页。若某个版本是首个版本，也只保留与本版直接相关的结论。

## 1 · 远端回读

FACT：更新后重新拉取 GitHub Releases API，32 个页面全部通过：

```text
first_line_is_## 概要  = 32 / 32
项目总介绍页头          = 0
> 主题 横幅             = 0
三级标题页面            = 0
重复版本标题            = 0
```

FACT：Release 的 ID、tag、标题、draft / prerelease 状态、发布时间和附件 ID、名称、大小、
下载地址均未变化。本轮 PATCH 只发送 `body` 字段。

## 2 · 仓库门禁

FACT：`tools/ci/release-notes.sh` 现在从 CHANGELOG 生成规范正文：

- 第一行固定输出 `## 概要`；
- 把 CHANGELOG 的 `###` 小节提升为 Release 的 `##`；
- 去掉重复版本标题、项目总介绍和 `> 主题：**…**` 页头。

FACT：新增 `tools/ci/test-release-notes.sh` 并接入 `make check`，覆盖首行、页头、标题层级、
重复版本标题和 `0.0.100` 拒绝规则。

FACT：`tools/ci/audit-release-names.sh` 现在逐个回读全部远端 Release，正文不以 `## 概要`
开头、混入项目总介绍 / 主题横幅、含三级标题或重复版本标题都会失败。

FACT：远端重跑结果：

```text
README header: PASS
release naming audit: PASS (tags=32 releases=32 latest=Husk 0.0.32)
```

## 3 · 标题页与版本进位

FACT：README 首屏顺序仍是 `Husk` 标题 → 六边形图标 → 徽章；项目介绍从 `## 概览` 开始，
不进入标题区。

FACT：版本号按 100 进位，不存在 `0.0.100`：

```text
0.0.32 -> 0.0.33
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

## 4 · 边界

- 本轮只改 Release 正文、仓库发布工具和文档，不修改运行时代码。
- 本轮不新增 tag，不移动任何 tag 的提交目标，不上传或替换附件。
- GitHub 网页可能短时间显示缓存旧正文；以 Releases API 回读和 `make release-audit` 为准。
