# 268 · 发布版本序列与 Release 标题规范化

日期：2026-11-26（CST）。

## 0 · 结论

FACT：GitHub 上 `v0.0.1`–`v0.0.23` 的 Release 标题均为 `Husk X.Y.Z`；`v0.0.24` 和
`v0.0.26`–`v0.0.32` 把主题拼进了标题，例如
`Husk 0.0.32 — 文件创建模式、虚构所有权与 access 主体`。同一个仓库出现了两种命名方式。

FACT：已用 `gh release edit` 将上述 8 个 Release 的标题统一改为 `Husk 0.0.N`，并逐个回读
确认。只改标题；tag、正文、附件和 release 状态均未改动。

## 1 · 100 进位规则

Husk 的版本号形式仍为 `X.Y.Z`，但发布序号按 100 进位：

```text
0.0.98 -> 0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

不存在 `0.0.100`。新增 `tools/ci/version-bump.sh` 和 `make version-bump` 执行这条规则；
`make version-check` 同时拒绝任何分量大于 99 的版本号，避免手改绕过。

FACT：在临时 VERSION 副本上实测：

```text
0.0.32 -> 0.0.33
0.0.99 -> 0.1.0
0.99.99 -> 1.0.0
```

## 2 · 发布标题约束

GitHub Release 标题只写 `Husk X.Y.Z`。版本主题、修复内容、验证结果、资产与边界全部放正文，
不拼进标题。规则已写入 `docs/GOALS.md` C6。
