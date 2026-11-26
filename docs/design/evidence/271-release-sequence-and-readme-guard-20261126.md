# 271 · 发布序列连续性与仓库首屏防回归

日期：2026-11-26（CST）。

## 0 · 结论

FACT：远端 `v0.0.1`–`v0.0.32` 共 32 个正式 tag，版本连续、无跳号、无前导零；
32 个 Release 标题均为 `Husk X.Y.Z`。最新 `VERSION` 与最新 Release 都是 `0.0.32`。

FACT：`make release-audit` 在本次复核中通过：

```text
README header: PASS
release naming audit: PASS (tags=32 releases=32 latest=Husk 0.0.32)
```

FACT：`tools/ci/version-bump.sh` 在临时副本上实测，按 100 进位：

```text
0.0.32 -> 0.0.33
0.0.98 -> 0.0.99
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

因此下一版是 `0.0.33`，不存在 `0.0.100`。

## 1 · 首屏结构

FACT：GitHub API 回读的当前 `README.md` 首个可见元素是 `<h1>Husk</h1>`，其后是六边形图标和
徽章；第一条项目介绍从 `## 概览` 开始。

新增 `tools/ci/audit-readme-header.sh`，检查：

- 第一个可见元素必须是 `Husk` 一级标题；
- 标题下必须有六边形图标和至少一个徽章；
- `## 概览` 之前不能出现普通段落、标题或其它正文；
- `## 概览` 必须存在。

FACT：负对照测试在标题区插入一段普通介绍后，脚本报错并拒绝通过；当前 README 通过。

该检查已接入 `make check`，`make release-audit` 也会先执行一次，防止介绍重新挤回标题区。

## 2 · 发布序列防回归

FACT：`tools/ci/audit-release-names.sh` 现在同时校验：

- tag 必须从 `v0.0.1` 开始；
- 版本按 `X.Y.Z` 每分量 `0..99` 的顺序连续递增；
- 最高 tag 必须等于 `VERSION`；
- Release 标题只能是 `Husk X.Y.Z`，正文首行不能重复版本标题。

这样既会拒绝 `0.0.100`，也会拒绝跳号、孤立 tag 和发布页标题漂移。

## 3 · 边界

- 本次只改发布审计与文档门禁，不改运行时、不改已发布 tag/附件。
- `UNKNOWN`：GitHub 网页缓存可能短时间显示旧标题；以 Release 页面重新加载后的标题和
  `make release-audit` 的 API 回读结果为准。
