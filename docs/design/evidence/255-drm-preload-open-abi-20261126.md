# 255 · DRM preload 的 `openat` errno 改写与创建 mode 丢失

日期：2026-11-26 13:05（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。探针：`runtime/crates/fd-identity-probe`。

## 0 · 修前事实（FACT）

`libdrm_preload.so` 的 `openat` 调用 glibc `syscall()` 后，把 `-1` 当成原始 `-errno` 再算一次：

```rust
if r < 0 && r > -4096 {
    set_errno((-r) as i32);
    return -1;
}
```

`syscall()` 已经把真实的 errno 写进线程局部状态，因此不存在的路径从 `ENOENT(2)` 被改写为
`EPERM(1)`。同一个入口还把 `mode` 固定传 0，`O_CREAT` 创建的文件变成 mode `000`（再套 umask）。

旧 0.0.27 `.so` 的动态负对照：

```text
fdidentity|drm-preload-open-failure-abi|PASS|rc=-1 errno=2 expected=-1/2
fdidentity|drm-preload-openat-failure-errno|FAIL|rc=-1 errno=1 expected=-1/2
fdidentity|drm-preload-openat-create-mode|FAIL|stat_rc=0 mode=0 expected=640 path=/data/data/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs/root/fdidentity-drm-openat-mode
```

`open` 本身使用 glibc 的 `open` 包装，失败 ABI 这一项已经 PASS；要修的是 `openat` 的二次
errno 转换，以及所有 open 入口的 `mode` 转发。

## 1 · 修法

1. `openat` 在 `syscall()` 返回负数时直接返回 `-1`，不覆盖 glibc 已设置的 errno。
2. `open` / `open64` / `openat` / `openat64` 都将调用方的 `mode` 原样传给内核。
3. `fd-identity-probe` 增加三项断言：`open` 失败 ABI、`openat` 失败 errno、`openat(O_CREAT)`
   创建 mode；三项都接入门禁 A。

## 2 · 修后真机结果（FACT）

```text
fdidentity|drm-preload-open-failure-abi|PASS|rc=-1 errno=2 expected=-1/2
fdidentity|drm-preload-openat-failure-errno|PASS|rc=-1 errno=2 expected=-1/2
fdidentity|drm-preload-openat-create-mode|PASS|stat_rc=0 mode=640 expected=640
```

同一轮此前的 DRM fd 身份、V4L2 fd 身份、netlink 覆盖与复制断言也全部 PASS。

## 3 · 边界

- FACT：本轮动态验证的是 Mali-G77 / Android 13 / 4 KiB 上的 preload open ABI。
- UNKNOWN：preload 在路径 shim 之外直接处理非 DRM 路径时的绝对路径翻译和隔离；这三项只证明
  open ABI，不证明完整路径语义。
- UNKNOWN：Adreno/KGSL 真机上的同路径。

## 4 · 回归（FACT）

```text
门禁 A：PASS=38 FAIL=0
coverage-probe：PASS
coverage-sweep：PASS（1400 条候选路径）
V4L2_REF：PASS
DRM fd：PASS
fd-identity-probe：PASS（11/11）
make check：PASS（格式化、aarch64 检查、Clippy、72 个 shell、280 个 Markdown、版本 0.0.28）
make dist：PASS
```

发布包 `dist/husk-arm64.tar.gz`：

```text
SHA256 bdf2d779f8885a7e7b191f77120a3a89f7c39b8ef97f13f240e662dc6d76e89e
VERSION 0.0.28
loader SHA256 3f4189e2eff1e94a1068484916e5088f95b9f21aa27e12c0518f8e31d6abf495
guest libc SHA256 69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c
```

FACT：loader 与 guest libc 哈希和 0.0.27 相同；本版只改 `drm-preload` 的打开 ABI 和动态探针。
