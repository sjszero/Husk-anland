# 251 · 假 DRM / Adreno fd 被 dup2、dup3 覆盖后的记账清理

日期：2026-11-26 11:20（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 /
kernel 4.19.191 / 4 KiB。探针：`runtime/crates/drm-probe/src/bin/drmfake.rs`。

## 0 · 修前事实

`dup2(oldfd, newfd)` 和 `dup3(oldfd, newfd, flags)` 会先关闭 `newfd` 当前指向的文件描述，
再把 `oldfd` 的文件描述安装到 `newfd`。旧实现只在 `oldfd` 本身是假 DRM fd 时才清理
`newfd` 的 DRM 表项：

```rust
drm_inherit_fake(old, new) {
    if is_fake_fd_locked(old) {
        drm_forget_locked(new);
        register_fake_fd_locked(new);
    }
}
```

FACT：当 `oldfd` 是普通文件、`newfd` 原来是假 DRM fd 时，`newfd` 已经不再指向 DRM，
但 `FAKE_FDS` 仍保留旧号码。之后对普通文件 `newfd` 调 DRM ioctl，会错误进入
`DRM_CTX.dispatch()`，得到伪成功或错误语义，而不是普通文件应有的失败。
Adreno 的 `ADRENO_FDS` 也有同样结构：`adreno_inherit()` 没有先清掉目标 fd。

## 1 · 修法

1. `drm_inherit_fake()` 无论源 fd 是不是假 DRM，都先执行 `drm_forget_locked(new)`。
2. `adreno_inherit()` 先执行 `adreno_forget(new)`；只有源 fd 通过 KGSL 设备校验后，
   才把新 fd 重新登记。
3. 保留原有正方向语义：`dup2(fake, new)` 后新 fd 仍是假 DRM，源 fd 关闭也不影响副本。
4. 新增门禁探针：普通文件覆盖假 fd、假 fd 复制到新 fd 两个方向都跑；旧 loader 用作负对照。

## 2 · 真机动态结果（FACT）

修前 loader SHA256：`a4350b5f2e2657e2f89a0eebbf10f149f1767bf250702e8ee7bf13d73449a9e8`。
修后 loader SHA256：`c81f2ece22c3d130dccfb9afa70f8ff01397d5544fbc84abf60243a8a364b207`。

旧 loader 的负对照：

```text
drmfake|dup2-ordinary-over-fake|FAIL|ioctl=Ok(()) expected=non-DRM fallthrough (EACCES/ENOTTY)
drmfake|dup3-ordinary-over-fake|FAIL|ioctl=Ok(()) expected=non-DRM fallthrough (EACCES/ENOTTY)
drmfake|dup2-fake-preserved|PASS|ioctl=Ok(()) after source close
```

修后 loader：

```text
drmfake|dup2-ordinary-over-fake|PASS|ioctl=Err(13) expected=non-DRM fallthrough (EACCES/ENOTTY)
drmfake|dup3-ordinary-over-fake|PASS|ioctl=Err(13) expected=non-DRM fallthrough (EACCES/ENOTTY)
drmfake|dup2-fake-preserved|PASS|ioctl=Ok(()) after source close
```

`Err(13)` 是 Android app 域 seccomp 对普通文件 ioctl 的实际返回；内核上的普通文件通常返回
`ENOTTY(25)`。两种结果都证明 ioctl 已离开 DRM shim；判据不接受 `Ok(())`，因为它只能由残留的
假 fd 记账产生。

## 3 · 回归

```text
门禁 A：PASS=38 FAIL=0
coverage-probe：PASS
coverage-sweep：PASS（1401 条候选路径）
V4L2_REF：PASS
DRM fd：PASS（4/4）
make check：PASS
```

`run-drm-fd-probe.sh` 已接入 `make gate-a`；脚本会自行推送当前 `husk-session.sh`，
不依赖上一次设备测试留下的 `/data/local/tmp` 文件。

## 4 · 未验证 / 边界

- FACT：Mali-G77 真机上，假 DRM fd 的 `dup2`/`dup3` 覆盖路径已动态验证。
- UNKNOWN：Adreno/KGSL 真机上的同等路径。代码已经同步清理 `ADRENO_FDS`，但没有高通设备，
  不能把静态修改写成动态通过。
- UNKNOWN：`SCM_RIGHTS` 跨进程传假 DRM fd 后的表身份恢复；当前仍不是跨进程内核级设备语义。
- UNKNOWN：多线程同时对同一 fd 做 `dup2`/`dup3`/`close` 的长时间压力；本探针验证的是记账
  覆盖顺序，不冒充并发压力通过。
