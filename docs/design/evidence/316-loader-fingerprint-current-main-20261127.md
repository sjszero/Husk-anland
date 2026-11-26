# 316 · 当前主干的 loader 指纹核对（2026-11-27）

## 0 · 结论

- **FACT**：`docs/ENGINEERING.md` 的产物表停留在旧主干：`ld-embedded.so` 写
  `3f4189e2...`、`libpath_shim.so` 写 `455665eb...`。
- **FACT**：在干净工作树上用当前源码重新执行 loader 构建，实际产物为
  `ld-embedded.so = 96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b`、
  `libpath_shim.so = 5319d12893bdf783fd540805319825a77938fdf7472512f59ea5c7bf5b0abed6`。
- **INFERENCE**：差异来自 0.0.26 之后 `path-shim` 源码继续变化，而文档只在提交
  `98284df` 更新过一次；不是同源码重复构建的随机漂移。
- **UNVERIFIED**：本次没有重跑真机门禁；这里只核对构建产物与文档一致性，不代表运行时能力
  重新验收。

## 1 · 复现

构建输入：

```text
base commit: d2c6262
NDK: /Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
command: make loader
```

关键输出：

```text
"loader": "glibc 2.39 aarch64"
"shim_span": 589824
"hook_vaddr": 1179648
"interposed": 209
"file_offset": 262144
"vaddr": 589824
"total_load": 606208
"out_size": 869888
```

产物指纹：

```text
5319d12893bdf783fd540805319825a77938fdf7472512f59ea5c7bf5b0abed6  runtime/target/aarch64-unknown-linux-gnu/release/libpath_shim.so
96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b  tools/loader/ld-embedded.so
96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b  app/app/src/main/jniLibs/arm64-v8a/libld.so
2ed8771f220ba55ed0c224e114c83a30cb9d34fb333d375fc690f7267bd138bb  tools/loader/libc-nop.so.6
```

## 2 · 文档验证

```text
make docs
checked 343 markdown files, 0 broken link(s)
git diff --check
```

## 3 · 边界

- 本改动只更新当前主干的产物指纹和证据，不改 `VERSION`、已发布 `v0.0.36`、tag 或 Release。
- 发布审计仍以 tag 内 `dist/SHA256SUMS` 为准；当前指纹只对当前主干构建有效。
