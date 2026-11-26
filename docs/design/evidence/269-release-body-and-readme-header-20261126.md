# 269 · Release 正文与仓库首屏规范化

日期：2026-11-26（CST）。

## 0 · 结论

FACT：GitHub 上 32 个 Release 的名称在 evidence 268 后已经是 `Husk X.Y.Z`，但正文首行仍有
三种历史写法：`# Husk X.Y.Z`、`## Husk X.Y.Z`、`## [X.Y.Z] - 日期`。较早页面则直接以介绍
文字开头，因此 Release 页面看上去仍是两套格式。

FACT：本轮移除了 15 个页面正文首行的重复版本标题；`v0.0.19` 原正文为空，本轮按
`CHANGELOG.md` 对应小节补齐。其余 16 个页面原本已经是正文开头，内容未改。

FACT：回读 32 个 Release 后，名称不匹配数为 0，正文以 Markdown 标题开头的页面数为 0，
正文为空的页面数为 0。tag、附件、发布时间和 release 状态未改。

FACT：仓库首屏原来在图标和徽章之间放定位语，徽章后立刻接项目介绍。现在首屏只保留
`Husk` 标题、图标和徽章；定位语与项目介绍从 `## 概览` 开始。

## 1 · 仓库改动

| 文件 | 改动 |
|---|---|
| `README.md` | 首屏只留标题、图标、徽章；新增 `## 概览` 承载介绍与定位语 |
| `CONTRIBUTING.md` | 写清 100 进位规则，发版改用 `make version-bump`，并规定 Release 标题与正文格式 |
| `docs/GOALS.md` | 把首屏边界和 Release 正文规则写进仓库标准 |
| `tools/ci/release-notes.sh` | 从 CHANGELOG 提取指定版本正文，并去掉 `## [X.Y.Z]` 标题 |

## 2 · 验证

FACT：`sh tools/ci/check-shell.sh` 通过，检查 85 个脚本。

FACT：`make version-check` 通过，当前版本为 `0.0.32`。

FACT：`make docs` 检查 294 个 Markdown 文件，0 个坏链。

FACT：`tools/ci/release-notes.sh 0.0.32` 的首行是正文主题，不再重复版本标题；输入
`0.0.100` 返回 65，并报 `version component exceeds 99`。

FACT：在临时目录实测 100 进位：

```text
0.0.32 -> 0.0.33
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

FACT：重读 GitHub Releases API 的结构化校验结果：

```text
releases=32
name_mismatches=0
body_starts_with_heading=0
empty_bodies=0
```

## 3 · 边界

- 本轮只改发布页面元数据、README 和发布规范，不改 tag、附件或二进制。
- 本轮不产生新 APK 或 rootfs，不改变运行时行为。
