# 270 · 发布命名一致性复核与异常标签清理

日期：2026-11-26（CST）。

## 0 · 结论

FACT：32 个正式 tag 均为 `vX.Y.Z`，32 个 GitHub Release 标题均为 `Husk X.Y.Z`，
`VERSION`、最新 CHANGELOG 小节和最新 Release 均为 `0.0.32`。

FACT：远端额外存在一个非版本号轻量标签
`untagged-f5a0d2a05f35ca6c3dfd`。它和 `v0.0.11` 指向同一个提交
`ad8f99d2450a84f9103ba96fcd09650ccea147af`，没有独立 Release，属于重复别名
而非版本。

FACT：已删除本地与远端的该重复别名。`v0.0.11` 及其提交、Release、附件
未改。

FACT：`README.md` 首屏仍只有 `Husk` 标题、六边形 H 图标和徽章；介绍从
`## 概览` 开始。GitHub API 回读仓库 README 时未再发现标题区介绍。

## 1 · 防回归

新增 `tools/ci/audit-release-names.sh` 和 `make release-audit`，检查：

- 远端 tag 只能是 `vX.Y.Z`，每个分量 `0..99`，不允许前导零；
- 每个 tag 必须对应一个 GitHub Release，不能有孤立 tag；
- Release 标题必须严格等于 `Husk X.Y.Z`；
- Release 正文第一行不能重复版本 Markdown 标题或裸版本标题；
- 最新 `VERSION` 必须已经对应 GitHub Release。

## 2 · 版本进位

规则仍是 100 进位，下一版为 `0.0.33`：

```text
0.0.32 -> 0.0.33
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

## 3 · 边界

- 不重命名、不移动任何已发布正式 tag。
- 不改 Release 附件与正文内容，只审计命名。
- Adreno/KGSL、桌面 GPU 崩溃和硬件适配状态不在本证据范围内。
