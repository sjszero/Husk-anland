# 305 · guest 工具交叉链接找不到 GCC start files / libgcc_s

日期：2026-11-27（CST）
主机：macOS + Android NDK 28.2.13676358
设备：RMX3357 / MT6893 / Android 13

涉及文件：

- `tools/guest-config/build-guest-tools.sh`
- `Makefile`（调用方，只作为失败现场）

## 0. 结论

- **FACT**：`make dist` 在 guest 工具链接阶段失败：NDK clang + 一个 Debian/Ubuntu `--sysroot` 不会自动推导 `/usr/lib/gcc/<triple>/<version>`，因此找不到 `crtbeginS.o` / `crtendS.o` / `-lgcc_s`。
- **FACT**：补上显式 `-B$GCCLIB/` 与 `-L$GCCLIB` 后，9 个 guest 工具全部链接成功。
- **FACT**：`make dist` 最终成功，生成 `dist/husk-arm64.tar.gz`，大小 7,139,367 字节，sha256 `7f7083e68cc0b8706a3912faf61b54d69084d93331345343dae3e0db4f76a877`。
- **FACT**：发行 bundle 内含最新 loader 和系统总线配置；从 bundle 在空目录重新安装，`INSTALL_RC=0`，树体检 `PASS=13 FAIL=0`，门禁 A `PASS=34 FAIL=0 SKIP=1`。

## 1. 修前负向结果

第一次 `make dist`：

```text
tools/guest-config/build-guest-tools.sh: incomplete sysroot:
  /tmp/nr-toolchain/root/usr/lib/aarch64-linux-gnu/crt1.o missing
make: *** [dist] Error 1
```

补齐 `libc6-dev` 后：

```text
ld.lld: error: cannot open crtbeginS.o: No such file or directory
ld.lld: error: unable to find library -lgcc_s
ld.lld: error: cannot open crtendS.o: No such file or directory
error: could not compile `usermgr`
```

这不是 Rust 代码错误，而是交叉链接参数缺口：`--sysroot` 只决定目标根，NDK clang 不会自动搜索目标发行版的 GCC 支持目录。

## 2. 修复

`build-guest-tools.sh` 现在先找 staging sysroot 中唯一的 GCC 支持目录：

```sh
GCCLIB=
for d in "$SYSROOT"/usr/lib/gcc/*/*; do
    [ -d "$d" ] && GCCLIB=$d
done
```

并做显式预检：

```text
crtbeginS.o
crtendS.o
libgcc_s.so
```

链接参数增加：

```text
-C link-arg=-B$GCCLIB/
-C link-arg=-L$GCCLIB
```

这样 `-B` 能找到 start files，`-L` 能找到 `libgcc_s`，不再依赖 clang 对主机安装布局的猜测。

## 3. 正向结果（FACT）

9 个 guest 工具全部产生有效入口：

```text
husk-usermgr   entry 0x2A580
husk-ldconfig  entry 0x272C0
husk-rd        entry 0x29DC0
husk-su        entry 0x25640
husk-symlinks  entry 0x26000
husk-window-wait entry 0x27180
gl-probe       entry 0x28800
ipc-probe      entry 0x26900
gpu-probe      entry 0x2B640
```

发行 bundle：

```text
dist/husk-arm64.tar.gz
sha256 7f7083e68cc0b8706a3912faf61b54d69084d93331345343dae3e0db4f76a877
libld.so 96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b
guest-config/linuxhub-system-bus.conf present
```

从 bundle 干净安装：

```text
INSTALL_RC=0
TREE_INTEGRITY PASS=13 FAIL=0
PASS=34 FAIL=0 SKIP=1
```

## 4. 边界

- 该脚本仍要求调用方提供一个完整的 aarch64 Ubuntu sysroot（至少 `libc6-dev`、`libgcc-13-dev`、运行库和 loader）；它现在会在缺失关键文件时提前报错，不再等到晦涩的链接错误。
- 本次 `make dist` 的成功依赖本机 staging sysroot 补齐 `libc6-dev`、`libgcc-13-dev`、`libgcc-s1`、`libmvec.so.1`。这与“工具链可从干净状态一键复现”仍不是同一件事，发行轮要把 sysroot 准备步骤固化。
