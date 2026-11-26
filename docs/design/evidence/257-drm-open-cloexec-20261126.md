# 257 · DRM 假节点的 `O_CLOEXEC` 与 kbase 初始化失败

日期：2026-11-26 11:51–11:56（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。探针：`runtime/crates/fd-identity-probe`。

## 0 · 修前事实（FACT）

真机通过 Husk loader 打开 `/dev/dri/renderD128`，请求 `O_RDWR | O_CLOEXEC`：

```text
fd 4 flags 0 target /memfd:husk-drm (deleted)
dup 100 flags 1 target /memfd:husk-drm (deleted)
```

FACT：`F_DUPFD_CLOEXEC` 能正确设置复制 fd 的标志，但最初 open 返回的 fd 是
`FD_CLOEXEC=0`。新增动态探针后，旧 0.0.29 loader / preload 的负对照为：

```text
fdidentity|pathshim-drm-open-cloexec|FAIL|fd=4 fdflags=0 expected_cloexec=1
fdidentity|drm-preload-open-cloexec|FAIL|fd=5 fdflags=0 expected_cloexec=1
```

同时从源码确认：path-shim 忽略 `DrmCtx::init` 的 `Err` 并仍把上下文标记 ready；preload
在初始化失败时既不关闭临时 mali fd，也不把负 errno 返回给 open 调用方。

## 1 · 修法

1. path-shim 的 `drm_fake_open(flags)` 在返回 memfd 前设置 `FD_CLOEXEC`；失败时关闭 fd，
   保留真实 errno。
2. preload 的 fake open 接收 flags；新 `husk_drm_fake_open_flags(flags)` 进入
   `HUSK_DRM_API` 表，旧无参符号保留为兼容包装。
3. path-shim 的 API 桥在 preload 返回后再次核对 `O_CLOEXEC`，避免旧 preload 忽略 flags。
4. 两层遇到 kbase 初始化失败时关闭 mali fd 并把 `DrmCtx::init` 的负 errno 返回。
5. `fd-identity-probe` 新增 path-shim / preload 两条 `O_CLOEXEC` 断言并接入门禁 A。

## 2 · 修后真机结果（FACT）

```text
fdidentity|pathshim-drm-open-cloexec|PASS|fd=4 fdflags=1 expected_cloexec=1
fdidentity|drm-preload-open-cloexec|PASS|fd=5 fdflags=1 expected_cloexec=1
```

`fd-identity-probe` 当前 15/15 条断言通过。

## 3 · 边界

- FACT：动态验证范围是 RMX3357 / Mali-G77 / Android 13 / 4 KiB。
- FACT：path-shim 的 `libld.so` 进入发布包；独立 `libdrm_preload.so` 仍不在发布 tar 中。
- UNVERIFIED：kbase 初始化失败分支没有在真实设备上注入故障。
- UNKNOWN：Adreno/KGSL 真机上的同一路径。

## 4 · 回归（FACT）

### 4.1 `make check`

```text
cargo fmt --all --check                 PASS
aarch64 cargo check                     PASS
shell syntax: 72 script(s) checked      PASS
checked 282 markdown files, 0 broken    PASS
version ok: 0.0.30                      PASS
```

### 4.2 完整门禁 A

```text
PASS=38 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: PASS（1400 条候选路径，没有发现未翻译的 guest 绝对路径）
V4L2_REF|PASS
--- drm-fd-probe: PASS
--- fd-identity-probe: PASS（15/15）
```

新增两项保留在完整输出中：

```text
fdidentity|pathshim-drm-open-cloexec|PASS|fd=4 fdflags=1 expected_cloexec=1
fdidentity|drm-preload-open-cloexec|PASS|fd=5 fdflags=1 expected_cloexec=1
```

### 4.3 发布包

`make dist` 成功，包内 `VERSION=0.0.30`：

```text
3fffb5e26eca04b5ff010e60da65a29e4123895719670d2c6b04c6a82ff0e513  husk-arm64.tar.gz
3141b4276b2e4c0e8686a17f3492ca3d3cc5750b74a3be2538cbd3ac4965a1d5  libld.so
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  libc.so.6
```

0.0.29 的 loader SHA256 是
`3f4189e2eff1e94a1068484916e5088f95b9f21aa27e12c0518f8e31d6abf495`；0.0.30 变为上面的
`3141b427...`。这证明 path-shim 修复确实进入了发布环境，不是只改桌面或测试文件。

以上动态结果仅适用于 RMX3357 / Mali-G77 / Android 13 / 4 KiB。Adreno/KGSL 真机和
kbase 初始化失败注入仍是 UNKNOWN / UNVERIFIED。
