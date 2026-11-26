# 317 · S6：假 DRM fd 的 errno 统一（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
包：`com.linuxhub.husk`

## 0 · 结论

- **FACT**：修前同一条件下，没有 DRM preload 时 `path-shim` 返回
  `EOPNOTSUPP(95)`；dlopen 了 DRM preload 后，`drm-shim::DrmCtx::dispatch` 的未模拟
  请求返回 `ENOSYS(38)`。
- **FACT**：将 `drm-shim` 的未知请求统一为 `-EOPNOTSUPP(95)` 后，同一 probe 的最后一层
  输出为 `rc=-1 errno=95`。
- **FACT**：完整 `run-fd-identity-probe.sh` 在真机通过，21 项 `want` 全绿，最终
  `--- fd-identity-probe: PASS`。
- **UNVERIFIED**：这次验证的是 fd 语义和显式拒绝；没有重新跑真实 GPU 客户端的桌面长稳，
  不能把本结果扩大成“DRI3 / 零拷贝已经完成”。

## 1 · 修前真实输出

通过 Husk loader 直接运行 probe，修前关键行：

```text
fdidentity|pathshim-drm-fake-fd-identity-lost|PASS|memfd_fd=3 rc=-1 errno=95 | dup_fd=4 rc=-1 errno=95
fdidentity|pathshim-drm-fake-fd-identity-lost-preload|PASS|rc=-1 errno=38 (want explicit refusal: rc=-1, errno!=ENOTTY=25)
```

旧 probe 只断言“不是 ENOTTY”，所以 `38` 被当成通过；这正是 S6 没有被门禁抓住的原因。

## 2 · 修改

- `runtime/crates/drm-shim/src/lib.rs`：未模拟 DRM 请求从 `-38 (ENOSYS)` 改为
  `-95 (EOPNOTSUPP)`。
- `runtime/crates/fd-identity-probe/src/main.rs`：把断言收紧为必须
  `rc == -1 && errno == 95`，避免再次把别的拒绝码当通过。
- `tools/device-tests/run-fd-identity-probe.sh`：对应说明改为 EOPNOTSUPP(95)。
- `tools/guest/husk-session.sh`：系统总线启动改为直接用 `setsid -d sh -c ...`
  启动整个子进程，不再保留 `( ... & )` 外层后台子 shell，并从 `/dev/null` 读入，
  防止 `adb shell` 因测试会话的 dbus 进程一直等待而挂住。

## 3 · 修后真机结果

loader：

```text
96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b  libld.so
```

probe / preload：

```text
1aacce74781c0d9b29ba7cce736c3476c2bd15208a056619ff0c182f4c659bd6  fd-identity-probe
6a6e6dc771efc84b0bf9822db1c7e740a9bb15f21c5d893059418e269406d40f  libdrm_preload.so
```

关键行：

```text
fdidentity|pathshim-drm-fake-fd-identity-lost|PASS|memfd_fd=3 rc=-1 errno=95 | dup_fd=4 rc=-1 errno=95
fdidentity|pathshim-drm-fake-fd-identity-lost-preload|PASS|rc=-1 errno=95 (want -1/EOPNOTSUPP=95)
```

完整门禁：

```text
ok   preload 在场时同一条件统一回 EOPNOTSUPP(95)
--- fd-identity-probe: PASS
```

## 4 · 会话启动回归

修前，在 rootfs 还没有 `run/dbus/system_bus_socket` 时，直接跑
`run-fd-identity-probe.sh` 会让设备端 `husk-session.sh` 的后台 dbus 子进程卡住，
`adb shell` 超过 90 秒没有返回。

修后：

```text
run-as com.linuxhub.husk env NR_LD=... sh husk-session.sh ... -- /bin/true
real 0.22

run-as com.linuxhub.husk env NR_LD=... sh husk-session.sh ... -- /usr/bin/dpkg -C
lines=0
```

随后在正式入口 `release-tree-libc812-20261127/root` 上完整 `run-fd-identity-probe.sh` 正常返回，
证明修复的是会话启动路径，不是绕过 probe。

## 5 · 本地门禁

```text
make check                         PASS
cargo fmt --all --check            PASS
cargo check --target aarch64...    PASS
cargo test -p drm-shim             PASS (0 tests)
```

`cargo test -p fd-identity-probe` 在 macOS 宿主上无法编译：`libc` 不提供 Linux 的
`dup3`、`SOCK_CLOEXEC`、`SOCK_NONBLOCK` 和 memfd syscall 签名。这不是本次改动引入的问题，
该 probe 的权威运行环境是真机 ARM64 Linux，上面的完整真机门禁已经覆盖。

## 6 · 边界

- 本次没有修改 `VERSION`、已发布 `v0.0.36`、tag 或 Release。
- 本次没有把 `Path Shim` 宣称成安全沙箱；它仍只负责 Linux 用户态兼容和路径语义。
- 本次没有验证 Adreno/KGSL、16/64 KiB 真机或独立 X11 桌面。
