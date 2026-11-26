# 318 · 会话 bundle 隔离、P11 收口与当前 loader 指纹（2026-11-27）

## 0 · 结论

- **FACT**：当 `rootfs` 是通过软链接暴露时，旧 `husk-session.sh` 用字面父目录查找
  `.husk-install-state` 和 bundle 的 `libc.so.6`。它会把新树与旧 bundle 的恢复数据拼在一起，
  甚至用旧 libc 覆盖新树。
- **FACT**：新增的主机负向测试在修前失败：
  `session rootfs bundle test: new rootfs libc was overwritten from the legacy bundle`。
- **FACT**：把 `rootfs` 解析为物理路径、并从同一个物理 bundle 取 loader/state/libc 后，
  同一测试输出 `session rootfs bundle: PASS`。
- **FACT**：设备默认入口现为
  `files/linux/distributions/ubuntu-24.04/rootfs -> ../../../release-tree-libc812-20261127/root`。
  该树的 `dpkg -C` 为 0 行，Tree Integrity 为 `PASS=23 FAIL=0`，门禁 A 为
  `PASS=34 FAIL=0 SKIP=1`。
- **FACT**：本轮修复前的旧 `rootfs-pre-p1-legacy-20261127` 已删除；删除前新树已通过上述检查。
- **FACT**：`9263c0e` 改动 `drm-shim` 后，当前源码重建的 loader 指纹变为
  `4216ad0c...` / `3e770c0b...`；`docs/ENGINEERING.md` 的旧指纹已同步。
- **UNVERIFIED**：apt 在线索引、gcc/strace 覆盖探针仍因该发行树未携带对应工具而 SKIP；
  不能把本轮结果说成所有系统调用或所有硬件已验证。

## 1 · P11 的实际状态

历史缺陷仍然成立：旧出厂树是 P1 修复前的 `nr-tools` 产物，862 installed / 92 个 `.list`，
`dpkg -C` 1722 行。处理方式不是修改旧树，而是把稳定路径切到已重建的 807 包树，并在验证后删除
旧树。

设备上的最终事实：

```text
rootfs -> ../../../release-tree-libc812-20261127/root
dpkg -C lines=0
installed=807
.list=807
TREE_INTEGRITY PASS=23 FAIL=0
Gate A PASS=34 FAIL=0 SKIP=1
```

因此 P11 指的是“旧树不能发行”；当前默认发行入口已不再指向它。旧树删除不是证据本身，删除前
完成的默认路径复验才是关闭依据。

## 2 · 新发现的 bundle 混用缺陷（P19）

复现结构：

```text
legacy/rootfs -> ../../../bundle/root
legacy/.husk-install-state      # 旧状态
legacy/libc.so.6                # 旧恢复源
bundle/.husk-install-state      # 新状态，记录 2ed8771f...
bundle/libc.so.6                # 新恢复源
```

修前 `husk-session.sh` 使用 `dirname "$ROOTFS"` 而没有先物理解析 `rootfs`，于是拿的是
`legacy/` 下的状态和 libc；新树里的 libc 被覆盖。修法是：

- `husk-session.sh`、`husk-desktop.sh` 都用 `pwd -P` 解析 `rootfs`；
- 从解析后的物理目录取 `BUNDLE`、`NR_LD`、`.husk-install-state` 和 `libc.so.6`。

主机回归测试：

```text
sh tools/ci/test-session-rootfs-bundle.sh
session rootfs bundle: PASS
```

设备复验使用修复后的会话脚本和 `4216ad0c...` loader，默认软链接入口执行
`/usr/bin/dpkg -C`，返回 0 行；三份 libc（bundle、`usr/lib`、`lib`）和 install state
都保持 `2ed8771f220ba55ed0c224e114c83a30cb9d34fb333d375fc690f7267bd138bb`。

## 3 · 当前 loader 指纹

`9263c0e` 改了嵌入到 loader 的 `drm-shim`，所以 316 记录的 `96b96bbe...` 已经不能代表当前
源码交付物；文档 316 保留为历史证据，当前主干以本文件为准：

```text
4216ad0cd98f41e8f7b8f366658fe80b9231d60d49a94f90382cf26f6ae8d768  tools/loader/ld-embedded.so
4216ad0cd98f41e8f7b8f366658fe80b9231d60d49a94f90382cf26f6ae8d768  app/app/src/main/jniLibs/arm64-v8a/libld.so
3e770c0b9a37554dcfdecb5d202881ef222859bac7754a1f8064fce4489d73e7  runtime/target/aarch64-unknown-linux-gnu/release/libpath_shim.so
```

重建命令和关键断言：

```text
NDK=<ndk>/toolchains/llvm/prebuilt/darwin-x86_64 make loader
"interposed": 209
"shim_span": 589824
"out_size": 869888
```

loader 指纹只对当前源码有意义；发行审计仍必须以 tag 内 `dist/SHA256SUMS` 为准。

## 4 · 门禁结果与未完成边界

```text
make apk                 BUILD SUCCESSFUL; libhuskapp.so + libhuskguard.so + libld.so present
run-fd-identity-probe    PASS (21 assertions)
20-tree-integrity.sh     PASS=23 FAIL=0
make gate-a              PASS=34 FAIL=0 SKIP=1; supplementary/locale/guard PASS
```

SKIP 项是环境缺少 guest `gcc` / `strace`，不是已通过项。独立 Husk X11 桌面、Adreno/KGSL、
16/64 KiB 真机仍未在本证据中验证。
