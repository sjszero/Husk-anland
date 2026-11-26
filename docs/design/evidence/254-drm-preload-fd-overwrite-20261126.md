# 254 · DRM preload 假 fd 被普通文件覆盖后的身份误判

日期：2026-11-26 12:45（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。探针：`runtime/crates/fd-identity-probe`。

## 0 · 修前事实（FACT）

`libdrm_preload.so` 旧实现把假 fd 号码抄进私有 `FAKE: [i32; 8]`。`is_fake()` 先查号码表，
号码命中就直接返回真。`dup2(oldfd, fake_fd)` 会先关闭 `fake_fd`，再把普通文件装到同一号码，
但 preload 不经过自己的 `close`，号码表没有任何机会更新。

用 `0.0.26` 的旧 `libdrm_preload.so` 动态加载到当前探针，结果：

```text
fdidentity|drm-preload-dup2-ordinary-over-fake|FAIL|before=1 after=1 expected=1->0
fdidentity|drm-preload-dup2-fake-preserved|PASS|is_fake_after_source_close=1 expected=1
```

FACT：目标号码已经指向普通文件，`is_fake()` 仍返回 1。后续 DRM ioctl 会被错误送进
`DRM_CTX`，而不是作为普通文件失败。这不是 V4L2 或 netlink 的同类推断，而是本次动态负对照。

## 1 · 修法

1. 删掉 `FAKE` 数组和 `register_fake()`，不再把 fd 号码当身份。
2. `is_fake()` 每次读取 `/proc/self/fd/<n>` 的内核目标。只认两种目标：
   loader 创建的 `memfd:husk-*`，以及真实 kbase 节点目标中的 `mali`。
3. `fd-identity-probe` 动态 `dlopen("/root/libhusk_drm.so")`，直接调用导出的
   `husk_drm_fake_open` 与 `drm_shim_is_fake`，同时验证覆盖与复制方向。
4. `run-fd-identity-probe.sh` 自行构建并推送当前 `libdrm_preload.so`，避免使用设备上残留的旧文件。
5. `drm-preload` 加入 `make check`、Clippy 和 CI 的 crate 清单。

## 2 · 修后真机结果（FACT）

```text
fdidentity|drm-preload-dup2-ordinary-over-fake|PASS|before=1 after=0 expected=1->0
fdidentity|drm-preload-dup2-fake-preserved|PASS|is_fake_after_source_close=1 expected=1
```

同一轮 `fd-identity-probe` 的 V4L2 / netlink 既有断言也全部 PASS：

```text
fdidentity|v4l2-dup2-ordinary-over-fake|PASS|QUERYCAP=Err(13) expected=negative after ordinary overwrite
fdidentity|v4l2-dup3-ordinary-over-fake|PASS|QUERYCAP=Err(13) expected=negative after ordinary overwrite
fdidentity|v4l2-dup2-fake-preserved|PASS|QUERYCAP=Ok("husk") after source close
fdidentity|netlink-dup2-ordinary-over-fake|PASS|family_before=Ok(16) family_after=Ok(1)
fdidentity|netlink-dup3-ordinary-over-fake|PASS|family_before=Ok(16) family_after=Ok(1)
fdidentity|netlink-dup2-fake-preserved|PASS|family_after_source_close=Ok(16)
```

## 3 · 边界

- FACT：本轮动态验证的是 Mali-G77 / Android 13 / 4 KiB 上的 DRM preload 身份检查。
- UNKNOWN：Adreno/KGSL 真机上的同路径；本机没有 Adreno 设备。
- UNKNOWN：`SCM_RIGHTS` 跨进程传假 DRM fd 后的状态恢复。
- UNKNOWN：多线程同时 `dup2` / `dup3` / `close` 同一 fd 的长时压力；本探针只验证顺序语义。
- FACT：旧版复现只替换 `libdrm_preload.so`，探针与 loader 使用当前构建，因此证明的是 preload
  自身号码表错误，不是 loader 路径翻译错误。

## 4 · 回归（FACT）

```text
门禁 A：PASS=38 FAIL=0
coverage-probe：PASS
coverage-sweep：PASS（1400 条候选路径）
V4L2_REF：PASS
DRM fd：PASS
fd-identity-probe：PASS（8/8）
make check：PASS（格式化、aarch64 检查、Clippy、72 个 shell、279 个 Markdown、版本 0.0.27）
make dist：PASS
```

发布包 `dist/husk-arm64.tar.gz`：

```text
SHA256 a8b9189b1ec516ffcf125a575274e32aa1014684ae953d3b335d0bf574ec0353
VERSION 0.0.27
loader SHA256 3f4189e2eff1e94a1068484916e5088f95b9f21aa27e12c0518f8e31d6abf495
guest libc SHA256 69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c
```

FACT：loader 与 guest libc 的哈希和 0.0.26 相同，说明本次没有改动路径层或 libc；
发布包哈希变化来自版本与文档，以及 `VERSION=0.0.27`。DRM preload 的修复是本仓库源码与
动态门禁的修复，当前 `make dist` 的 launcher bundle 不包含独立 `libdrm_preload.so`。
