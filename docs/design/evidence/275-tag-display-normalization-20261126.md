# 275 · Git tag 显示名统一与全量审计

日期：2026-11-26（CST）。

## 0 · 结论

FACT：修复前远端 `v0.0.1`–`v0.0.32` 共 32 个 tag，其中 30 个是 annotated tag，
`v0.0.11`、`v0.0.12` 是轻量 tag。21 个 tag 的显示首行不是严格的 `Husk X.Y.Z`，
存在主题后缀、冒号、`release:` 前缀和作者前缀。GitHub Release 标题本身已经全部正确，
不一致只出现在 git tag 页面。

FACT：本次把 32 个 tag 全部统一为 annotated tag，说明首行严格等于 `Husk X.Y.Z`。
已逐项对比 `^{commit}`，32 个提交目标全部未变；tag 名称、GitHub Release 和附件也未变。

FACT：本地重写后的 32 个 tag 显示名全部通过检查：

```text
MISMATCH_COUNT=0
local tag subjects: PASS
```

## 1 · 修复前的不合规类型

| 类型 | 示例 | 数量 |
|---|---|---:|
| 轻量 tag，首行直接显示提交主题 | `release: 0.0.11——…` | 2 |
| 缺少 `Husk` 前缀 | `0.0.5：…` | 3 |
| 用冒号、破折号或空格拼主题 | `Husk 0.0.13 - full-chain audit` | 15 |
| 多出作者前缀 | `戏山 Husk 0.0.26` | 1 |

上表合计 21 个不合规 tag。`Husk 0.0.2`、`Husk 0.0.4`、`Husk 0.0.15` 等 11 个原本已符合格式。

## 2 · 不可变边界

本次只替换 annotated tag 对象的说明文字，不修改 tag 名，也不移动 tag 指向的 commit。
每个 tag 都保留原 tagger 时间和身份；轻量 tag 转为 annotated tag 时使用目标 commit 的
committer 时间和身份。旧 tag 对象保存在本地 `refs/tags-backup/20261126/*`，用于出错时恢复。

这与“已发布 tag 不移动”的发布规则不冲突：可下载的源码树、Release 和附件均未变化，
变化的只是 tag 页显示的说明首行。

## 3 · 防回归

`tools/ci/audit-release-names.sh` 不再只检查最新 tag，改为逐个检查所有远端 tag：

- 必须是 `vX.Y.Z`，三段均只能为 `0..99`；
- 必须是 annotated tag，不能退回提交主题；
- annotation 首行必须严格等于 `Husk X.Y.Z`；
- 版本序列必须从 `v0.0.1` 连续按 100 进位；
- GitHub Release 标题必须是 `Husk X.Y.Z`，正文首行不得重复版本标题；
- tag 与 GitHub Release 必须一一对应。

## 4 · README 首屏

FACT：README 首个可见元素仍是 `Husk` 一级标题，下一段是六边形图标，再下一段是徽章；
项目介绍从 `## 概览` 开始。`tools/ci/audit-readme-header.sh` 当前输出：

```text
README header: PASS
```

## 5 · 100 进位

FACT：`tools/ci/version-bump.sh` 的边界已经由 `tools/ci/test-version-bump.sh` 覆盖：

```text
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
reject 0.0.100
```

不存在 `0.0.100`，下一版仍从当前 `VERSION=0.0.32` 推进到 `0.0.33`。

