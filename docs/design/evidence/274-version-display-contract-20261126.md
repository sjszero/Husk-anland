# 274 · 版本显示契约与 100 进位防回归

日期：2026-11-26（CST）。

## 0 · 结论

FACT：远端现有 `v0.0.1`–`v0.0.32` 共 32 个 tag，32 个 GitHub Release 标题均为
`Husk X.Y.Z`，没有 `v` 前缀，也没有把主题拼进标题。最新 Release 是
`Husk 0.0.32`。

FACT：历史上部分 annotated tag 的内部说明并不统一，例如 `v0.0.15` 是
`Husk 0.0.15`，`v0.0.26` 带作者前缀，`v0.0.32` 带主题后缀。它们不是 GitHub Release
标题；已经发布的 tag 不移动、不改写。本次只约束新 tag，避免把历史元数据继续复制到下一版。

FACT：`tools/ci/version-bump.sh` 在隔离临时目录中实测：

```text
0.0.32 -> 0.0.33
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
reject 0.0.100
reject 0.0.1.2
```

新增 `tools/ci/test-version-bump.sh`，已接入 `make check` 和 GitHub Actions。它不修改
仓库根的 `VERSION`，只在临时目录验证下一版和进位边界。

## 1 · 固定写法

| 位置 | 固定格式 | 当前示例 |
|---|---|---|
| `VERSION`、`husk --version`、正文 | `X.Y.Z` | `0.0.32` |
| annotated git tag | `vX.Y.Z` | `v0.0.32` |
| GitHub Release 标题 | `Husk X.Y.Z` | `Husk 0.0.32` |
| `CHANGELOG.md` 小节 | `[X.Y.Z] - YYYY-MM-DD` | `[0.0.32] - 2026-11-26` |

`v` 只属于 git tag，Release 标题不写 `Husk vX.Y.Z`。主题、摘要和项目介绍只放正文，
不拼进 Release 标题。

## 2 · 100 进位

每个版本分量只能是 `0..99`，按 100 进位：

```text
0.0.32 -> 0.0.33 -> ... -> 0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

不存在 `0.0.100`。从 `0.0.33` 起，`make release-audit` 还要求最新 annotated tag 的
说明首行严格等于 `Husk X.Y.Z`，防止 tag 页和 Release 页再次出现两套名字。

## 3 · 首屏

FACT：`README.md` 第一个可见元素仍是 `<h1 align="center">Husk</h1>`，随后是六边形图标和
徽章；项目介绍从 `## 概览` 开始，没有回到标题区。`make release-audit` 回读结果为：

```text
README header: PASS
release naming audit: PASS (tags=32 releases=32 latest=Husk 0.0.32)
```

## 4 · 边界

- 本次不改运行时、不改已发布 tag/附件、不重写历史 Release 正文。
- `UNKNOWN`：GitHub 网页缓存可能短时间显示旧内容；以 API 回读和重新加载后的页面为准。

## 5 · 后续更正

FACT：本记录中“已经发布的 tag 不移动、不改写”的边界，已在同日由
[evidence 275](275-tag-display-normalization-20261126.md) 补充。历史 tag 的提交目标、Release
和附件仍不移动；仅把 annotation 的显示首行统一为 `Husk X.Y.Z`，并把审计范围从最新 tag
扩大到全部 tag。
