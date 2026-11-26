# 285 · 第 3 轮：两个 LD_PRELOAD 层根本构建不出来，以及它们的导出契约

日期：2026-11-26（CST）。设备：无（源码 + 宿主构建 + 产物符号检查）。
对应缺陷编号：A-15（LD_PRELOAD 层无构建路径、不进发行包、导出无断言）。

## 1 · 修前：v4l2-preload 在当前树上链接失败

FACT，在 `31d7228` 的树上执行：

```sh
cd runtime && cargo build --release -p v4l2-preload --target aarch64-unknown-linux-gnu
```

结果：

```
rust-lld: error: ...cgu.0.rcgu.o: symbol open64@GLIBC_2.17 has undefined version GLIBC_2.17
rust-lld: error: ... symbol ioctl@GLIBC_2.17 has undefined version GLIBC_2.17
rust-lld: error: ... symbol stat64@GLIBC_2.33 has undefined version GLIBC_2.33
（共 7 条）
```

根因（FACT）：`crates/v4l2-preload/src/lib.rs` 用 `.symver` 造了带版本的第二份定义
（`open64@GLIBC_2.17`、`stat64@GLIBC_2.33` …），版本节点必须由链接期的 version script 提供。
仓库里确实有 `crates/v4l2-preload/exports.map`，定义了 `GLIBC_2.17` / `GLIBC_2.33` 两个节点——
但**全仓库没有任何构建路径引用它**：

```sh
grep -rn 'exports.map' --include='*.sh' --include='Makefile' --include='*.toml' .
# 0 命中（evidence 目录除外）
```

所以那个 map 是死配置，`-p v4l2-preload` 的 release 构建从来跑不通。
`cargo check` 不链接，`make check` 也不会发现——这和 §284 的死代码是同一类问题：
**编译期绿，产物根本不存在**。

修后（FACT）：把 map 放到链接行就正常：

```sh
cd runtime && RUSTFLAGS="... -C link-arg=--version-script=$PWD/../runtime/crates/v4l2-preload/exports.map" \
  cargo build --release -p v4l2-preload --target aarch64-unknown-linux-gnu
# Finished; libhusk_v4l2.so 200424 字节
```

产物里确认 10 个带版本符号：`open@/open64@/openat@/ioctl@/mmap@/mmap64@/close@/read@/readv@GLIBC_2.17`、
`stat64@GLIBC_2.33`。

## 2 · 新增 `make preloads`

FACT，`Makefile`：

- `make preloads` 构建 `drm-preload`（→ `libdrm_preload.so`）与 `v4l2-preload`（→ `libhusk_v4l2.so`），
  v4l2 那一步带 `--version-script`。
- 构建完立即跑 `tools/ci/audit-preload-exports.sh`。
- 配方里必须 `cd runtime` 再跑 cargo：`runtime/.cargo/config.toml` 把
  `aarch64-unknown-linux-gnu` 的链接器指定为 `rust-lld`，在仓库根目录跑 cargo 读不到这份配置，
  会退回宿主 `cc`（macOS 上直接是 `ld64.lld: unknown argument '--version-script'`）。

实测（`make preloads`）：`preload exports OK`。

## 3 · 导出契约（`tools/ci/audit-preload-exports.sh`）

为什么这两个层特别需要：它们都是 `#[no_mangle]` 的 cdylib，**树里没有任何东西链接它们**，
全靠运行时 `dlopen`（Mesa 的 libgbm/libdrm、GStreamer 的 libgstvideo4linux2）。
所以丢一个 `#[no_mangle]`（例如把新 item 插到属性和 fn 之间）、改名、进 cfg，
`cargo check`、`clippy`、单测全绿，层却已经不拦截了 —— 284 就是这么发生的。

断言内容（FACT，本机通过）：

| 层 | 必须导出 |
|---|---|
| `libdrm_preload.so` | `open open64 openat ioctl mmap husk_drm_is_fake husk_drm_ioctl husk_drm_fake_open husk_drm_fake_open_flags drm_shim_is_fake` |
| `libhusk_v4l2.so` | `open@GLIBC_2.17 open64@ openat@ ioctl@ mmap64@ close@ read@ readv@GLIBC_2.17 stat64@GLIBC_2.33`、`husk_v4l2_open/ioctl/is_fake/mmap/close/nonblock`、`v4l2_read` |

两处按实测修正过：`close` **不在** DRM 层的导出里（假节点是真的 `/dev/mali0` fd，
close 不需要拦截）；`husk_v4l2_read` **不存在**，read 走版本化的 `read`/`readv` 与 `v4l2_read`。
断言不能凭想象列符号。

CI 新增 `preloads` job 跑同一条命令。

## 4 · 未修（明确留下的缺口）

**发行包里没有这两个层。** FACT：

```sh
tar tzf dist/husk-arm64.tar.gz
# 21 项，只有 libld.so / libc.so.6 / husk-* / guest* / VERSION，没有 libhusk_v4l2.so 或 libdrm_preload.so
```

而 `tools/device-tests/run-fd-identity-probe.sh:63` 是从
`runtime/target/.../libdrm_preload.so` 手动 push 进 rootfs 的。也就是说：

- GStreamer 走 `/dev/videoN`（evidence 281 的 `v4l2src` PASS）
- Mesa 的 libgbm/libdrm 走假 DRM 节点

这两条能力在当前发行包里**对最终用户不可用**，只有开发者手工 push 才有。
把层打进 tarball 需要先定安装位置与 `install-husk.sh` 的落点（`/usr/lib/husk/`？
以及 `LD_PRELOAD` 由谁设），那是发行面改动，不在本轮。

状态：UNVERIFIED（未上机）。本轮只保证「能构建、导出正确」，
不保证「装到设备上就生效」。
