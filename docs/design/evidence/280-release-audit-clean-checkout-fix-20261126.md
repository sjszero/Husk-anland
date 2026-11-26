# 280 · Git 引用审计的干净检出修复

- 日期：2026-11-26
- 状态：**PASS（静态门禁）**
- 范围：`tools/ci/audit-git-refs.sh`

## 问题

FACT：`0.0.34` 提交前运行 `make check` 时，`git grep` 默认不搜索未跟踪文件，因此没有看到
evidence 279 中引用的 `v0.1.0` 反例。提交后该文件进入 Git 索引，干净检出再运行同一门禁就会
失败：

```text
git reference: docs/design/evidence/279-version-reference-audit-20261126.md:13
points at missing tag v0.1.0
```

FACT：这不是运行时回归，而是发布门禁自身的覆盖缺口。

## 修复

- 使用 `git grep --untracked`，让新增但尚未提交的文档也进入检查。
- 排除 `docs/design/evidence/`。证据文件记录历史失败原文，不能因为引用已删除对象而让门禁
  永远失败。
- 已验证脚本本身、当前文档和新的未跟踪文件都能被同一套检查覆盖。

## 验证

```text
$ sh tools/ci/audit-git-refs.sh
git references: PASS

$ printf 'git show v0.1.0:tools/loader/nr-embed.py\n' > .husk-audit-probe.tmp
$ sh tools/ci/audit-git-refs.sh
git reference: .husk-audit-probe.tmp:1 points at missing tag v0.1.0
exit=1
```

负对照证明未跟踪文件已经进入审计；删除探针后，干净工作树再次返回 `PASS`。

`make check` 随后通过；发布后由 `make release-audit` 再回读全部远端 tag 与 Release。

## 边界

- FACT：本修复只影响仓库静态检查，不改变发布资产中的运行时。
- FACT：已发布的 `v0.0.34` 保持原提交和原资产，不回写、不移动。
