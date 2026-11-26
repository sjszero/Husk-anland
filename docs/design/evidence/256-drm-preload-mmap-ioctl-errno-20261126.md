# 256 · DRM preload 的 `mmap` 失败返回值与 `ioctl` errno

日期：2026-11-26 13:40（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。探针：`runtime/crates/fd-identity-probe`。

## 0 · 修前事实（FACT）

`libdrm_preload.so` 的 `mmap` 和普通 fd `ioctl` 都使用 glibc `syscall()`。这个包装失败时
返回 `-1` 并写 errno；旧代码又把它当原始 `-errno` 计算，同时 `mmap` 还返回 `NULL`：

```text
fdidentity|drm-preload-mmap-failure-errno|FAIL|rc=0 errno=1 expected=-1/9
fdidentity|drm-preload-ioctl-failure-errno|FAIL|rc=-1 errno=1 expected=-1/25
```

FACT：`mmap(-1)` 返回 `NULL` 而不是 `MAP_FAILED`，调用方按 mmap ABI 检查
`MAP_FAILED` 时会把失败当成成功地址；真实 `EBADF(9)` 被改写成 `EPERM(1)`。普通
`/dev/null` ioctl 的真实 `ENOTTY(25)` 同样被改写成 `EPERM(1)`。

同一轮还确认，DRM `DrmCtx` 的 ioctl 回调收到的是 glibc `-1` 而不是原始 `-errno`，这会把
kbase 错误统一误读成 `EPERM`。

## 1 · 修法

1. `mmap` 失败路径返回 `usize::MAX as *mut u8`，即 `MAP_FAILED`，并保留 glibc 设置的 errno。
2. 普通 fd 的 `ioctl` 透传不再二次改写 errno。
3. 新增 `raw_syscall3()`，只给 `DrmCtx` 的 ioctl 回调用，保持原始负 errno 语义。
4. `fd-identity-probe` 新增两项失败 ABI 断言并接入门禁 A。

## 2 · 修后真机结果（FACT）

```text
fdidentity|drm-preload-mmap-failure-errno|PASS|rc=-1 errno=9 expected=-1/9
fdidentity|drm-preload-ioctl-failure-errno|PASS|rc=-1 errno=25 expected=-1/25
```

## 3 · 边界

- FACT：本轮动态验证的是 Mali-G77 / Android 13 / 4 KiB 上的失败 ABI。
- UNKNOWN：非 DRM 路径在 preload 绕过路径 shim 时的完整翻译与隔离。
- UNKNOWN：Adreno/KGSL 真机上的失败路径。
- FACT：成功路径和 GPU 后端未在本版改动；最终仍以完整门禁和既有 DRM 探针为准。

## 4 · 回归（FACT）

执行时间：2026-11-26 11:46–11:47（CST）。设备仍在线的条件如下：

```text
DEV=UKHMMB8PYTGECMNV
RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB
```

### 4.1 `make check`

```text
cargo fmt --all --check                 PASS
aarch64 cargo check                     PASS
shell syntax: 72 script(s) checked      PASS
checked 281 markdown files, 0 broken    PASS
version ok: 0.0.29                      PASS
```

### 4.2 门禁 A

```text
PASS=38 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: PASS  (没有发现未翻译的 guest 绝对路径)
--- drm-fd-probe: PASS
--- fd-identity-probe: PASS
```

`coverage-sweep` 从四组真实工作负载抓到 1400 条候选路径，设备侧判定未发现未翻译的 guest
绝对路径。`fd-identity-probe` 的 13 项断言全部通过，包括本 evidence 的两项：

```text
fdidentity|drm-preload-mmap-failure-errno|PASS|rc=-1 errno=9 expected=-1/9
fdidentity|drm-preload-ioctl-failure-errno|PASS|rc=-1 errno=25 expected=-1/25
```

### 4.3 发布包

`make dist` 成功，包内 `VERSION=0.0.29`：

```text
a5a05e8614827cbe983c5bd3a84c1f8088405f5802c7c2364e39930c0d5e188a  husk-arm64.tar.gz
3f4189e2eff1e94a1068484916e5088f95b9f21aa27e12c0518f8e31d6abf495  libld.so
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  libc.so.6
```

以上是 RMX3357 上的结果。跨机型、Adreno/KGSL 和非 DRM 路径隔离仍是 UNKNOWN，不能由本轮
回归外推。
