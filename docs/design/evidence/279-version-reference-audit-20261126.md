# 279 · 版本引用与标题页审计

- 日期：2026-11-26
- 状态：**PASS（静态门禁）**
- 范围：版本号、历史 Git 引用、README 首屏、Release 正文契约

## 观察

FACT：公开 tag 与 GitHub Release 标题已统一为 `vX.Y.Z` 和 `Husk X.Y.Z`，但仓库正文仍有两类
分叉：

1. `docs/RUNNING.md` 写死了 `0.0.7`，与实际 `VERSION` 不一致；
2. 多处 `git show v0.1.0:<path>` 指向已删除的 tag，实际历史内容应从提交 `665ec31^` 取回。

README 首屏没有项目介绍：首屏只有 `Husk` 标题、六边形图标和徽章，正文从 `## 概览` 开始。
Release 标题只写 `Husk X.Y.Z`，正文从 `## 概要` 开始。

## 修复

- 运行文档不再复制版本号，统一以仓库根 `VERSION` 和 `./husk --version` 为准。
- 历史工具引用改为不可变提交 `665ec31^`。
- 新增 `tools/ci/audit-git-refs.sh`，接入 `make check`：文档中的 `git show vX.Y.Z:<path>`
  必须指向真实存在的 tag。

## 验证

```text
$ sh tools/ci/audit-git-refs.sh
git references: PASS

$ sh tools/ci/audit-readme-header.sh
README header: PASS

$ sh tools/ci/test-version-bump.sh
version-bump test: 0.0.98 -> 0.0.99
version-bump test: 0.0.99 -> 0.1.0
version-bump test: 0.99.99 -> 1.0.0
version-bump test: reject 0.0.100
version-bump: PASS

$ sh tools/ci/test-release-notes.sh
release-notes: PASS (0.0.34)
```

## 边界

- FACT：这是静态仓库契约验证，不改变运行时。
- FACT：100 进位规则是 `0.0.98 → 0.0.99 → 0.1.0`、`0.99.99 → 1.0.0`，不存在
  `0.0.100`。
- UNKNOWN：GitHub 网页缓存或客户端缓存可能短时间继续显示旧内容，以 `make release-audit`
  回读 API 的结果为准。
