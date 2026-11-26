> **历史快照（已被 evidence 315 取代）**：本文的 `69ceef37` 是旧转换结果；当前 8.12 输出为 `2ed8771f`。

# 252 · 清洁工作树的 `make dist` 不再依赖遗留 libc 产物

日期：2026-11-26（CST）。验证机：Mac 开发机；目标包由同一份源码重新生成。

## 0 · 复现

清理 `app-rs/target`、`runtime/target`、Gradle 构建目录和本地旧产物后，执行：

```text
make dist NDK=<ndk-prebuilt>
cp tools/loader/libc-nop.so.6 dist/libc.so.6
cp: tools/loader/libc-nop.so.6: No such file or directory
make: *** [dist] Error 1
```

FACT：`libc-nop.so.6` 是 `.gitignore` 中的生成物，但 `dist` 没有在缺失时生成它，也没有在
前置检查里给出明确错误。新克隆或清理过工作树的发布者会得到一条复制失败，无法只靠
`NDK=... make dist` 重建发布包。

## 1 · 修法

1. `dist` 在开始前检查 `tools/loader/libc-nop.so.6`；不存在时必须提供
   `LIBC=<pristine glibc 2.39 libc.so.6>`，否则立刻给出明确错误。
2. `build-embedded-loader.sh` 在 `LIBC` 存在时执行完整变换并重新生成 `libc-nop.so.6`；
   生成后 `dist` 再检查一次，避免再次复制不存在的东西。
3. 发布文档和贡献文档写明 clean tree 需要 `LIBC`（或先在设备上运行 `make libc-patch`）。
4. `ENGINEERING.md` 的 loader / libc 指纹改为当前产物哈希，不再是过期值。

## 2 · 验证（FACT）

使用 Ubuntu ports 的固定输入：

```text
libc6_2.39-0ubuntu8.12_arm64.deb
sha256 2578bc27d4e077f5e93c283d2502bc693c516d9590a20a27840b548a4d436820
```

重建结果：

```text
100/105 hooks, 5 declared refusals
clone3 1->0, set_robust_list 2->0
sha256 69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c
```

随后：

```text
make dist NDK=... LIBC=<stock libc.so.6>
fafea0171123792067cc4ed726104bc029ca3bfdf4aa595f22b8af0146790074  husk-arm64.tar.gz
```

当前指纹：

```text
ld-noble2404-ld.so  124e3f166bdc0be365eaa6dc59e415b876b0841a40463edd1b0e084b518bb7cd
ld-embedded.so      c81f2ece22c3d130dccfb9afa70f8ff01397d5544fbc84abf60243a8a364b207
libc-nop.so.6       69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c
```

本修复只改发布入口和文档，不改变运行时行为；运行时探针与门禁 A 结果见 evidence 251。
