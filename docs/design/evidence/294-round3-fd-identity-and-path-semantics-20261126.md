# 294 · 第 3 轮：假 fd 身份单一真源 + 路径语义复核

日期：2026-11-26（CST）。机器：macOS arm64 开发机 + 真机
RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页，
序列号 `UKHMMB8PYTGECMNV`。版本 `VERSION=0.0.36`（未发版，最新 tag `v0.0.35`）。

本轮两件事：把"假 fd 身份"的三份重复实现收成一个带单测的 crate；把路径语义
（符号链接深度、越界、dirfd）在真机上重跑一遍留证。中途挖出三个**验证方法本身是错的**
的缺陷（S3/S4/S5），这一轮真正的时间花在那里。

## 0 · 结论

| 编号 | 缺陷 | 状态 | 判据（真机） |
|---|---|---|---|
| S1 | 假 fd 身份有三份实现，规则各不相同 | **FIXED** | 新 crate `runtime/crates/fd-identity`（no_std，5 单测）；`v4l2-shim` / `drm-preload` / `path-shim` 三处全改成调它 |
| S2 | `path-shim` 对"无状态假 DRM fd"回 `EBADF(9)`——fd 是好的，说"描述符无效"是假话 | **FIXED** | 探针 `pathshim-drm-fake-fd-identity-lost` → `rc=-1 errno=95`（`memfd` 与 `dup` 各一次） |
| S3 | 设备上三份 `libld.so`，探针默认用的不是刚构建的那份 | **FIXED** | 见 §1：探针现在先比对 sha256，不一致直接 `rc=1`；正向 `matches local build`，负向实测 `rc=1` |
| S4 | 探针用"errno≠ENOTTY 即被认领"当判据，本机内核在 memfd 上回 `EACCES(13)` | **FIXED** | 改成与同进程对照 memfd 比对；§3 有对照数据 |
| S5 | `pathshim-drm-fake-fd-identity-lost` 用 `open("/dev/dri/renderD128")` + `/proc/self/fd/N` 复现"身份丢失"，而 shim 会重新建立身份 | **FIXED** | 改成原生 `memfd:husk-drm` + `dup`；旧写法在本机（无 renderD128）也能打印结果，但它测的是 DRM 模拟层 |
| S6 | 同一条件两层答案不一致：无 preload → `EOPNOTSUPP(95)`；preload 在场 → `ENOSYS(38)` | **OPEN**（已量化） | 探针新增 `pathshim-drm-fake-fd-identity-lost-preload`，断言只用"显式拒绝、不是 ENOTTY" |

- **FACT**：`tools/device-tests/run-fd-identity-probe.sh` 现在是 `--- fd-identity-probe: PASS`（§2）。
- **FACT**：路径语义 `path-probe` **20/20**、`04-path-reliability.sh` **14/14 + 1 SKIP**（§4）。
- **FACT**：`make check` 全绿：fmt / `cargo check --workspace` / 单测（gpu-env 6 · nr-tools 1 ·
  session-env 4 · usermgr 3）/ `cargo clippy --workspace` / 320 篇文档链接 / 版本 `0.0.36`。
- **FACT**：`make loader` 两次构建逐字节相同（`sha256=b96b7860…`），即"重建能不能复现"这一条成立。

## 1 · S3：三份 `libld.so`，探针读的那份不是新构建

| 位置 | 谁在用 | 21:11 时的 sha256 |
|---|---|---|
| `/data/app/~~LX…/lib/arm64/libld.so` | **真实会话**（`app-rs/src/lib.rs:169` 用 `files/libdir` 拼路径） | `658f97e1…`（19:04 的旧件） |
| `files/linux/libld.so` | 没人（我当成了部署位） | `b96b7860…`（21:11 新件） |
| `files/linux/husk/libld.so` | **探针**（`run-fd-identity-probe.sh` 的 `LD=` 默认值） | `658f97e1…`（19:04 的旧件） |

后果：把新 loader 推到中间那一格、以为修好了，探针却一直跑旧件，连续几轮
"改了没变化"。这不是运行时缺陷，是**验证路径与部署路径不一致**，比缺陷本身更费时间。
判据：把同一份新件写进 `files/linux/husk/libld.so` 之后，同一条用例的答案从
`errno=38/13` 变成 `errno=95`，因为代码没动过。

修法（已落）：`run-fd-identity-probe.sh` 在跑之前比对设备上 `$LD` 与
`tools/loader/ld-embedded.so` 的 sha256，不一致就拒绝运行。两个方向都测了：

```
# 正向（当前部署位就是新件）
# loader under test: …/files/linux/husk/libld.so sha256=b96b7860… (matches local build)
--- fd-identity-probe: PASS

# 负向（故意指向另一个构建，sha 不同）
LD=/data/local/tmp/libld-dbg.so ./tools/device-tests/run-fd-identity-probe.sh <dev>
--- loader mismatch: refusing to run
    device … sha256=c49e06a8…   built … sha256=b96b7860…
rc=1
```

## 2 · 真机：fd 身份探针（修后）

```
fdidentity|pathshim-drm-fake-fd-identity-lost|PASS|memfd_fd=3 rc=-1 errno=95 | dup_fd=4 rc=-1 errno=95 (want -1/EOPNOTSUPP=95 both)
fdidentity|pathshim-drm-fake-fd-identity-lost-preload|PASS|rc=-1 errno=38 (want explicit refusal: rc=-1, errno!=ENOTTY=25)
fdidentity|v4l2-name-grammar-exact|PASS|junk names unclaimed (=control rc=-1 errno=13); husk-drm rc=-1 errno=38 (refused, not a camera)
...
--- fd-identity-probe: PASS
```

外加 18 条既有用例（V4L2 dup2/dup3 覆盖与保留、netlink 覆盖与保留、drm-preload 的
open/openat/mmap/ioctl ABI、`is_fake` 的精确名字判定）全 PASS。

## 3 · S4：这台 ROM 的内核在 memfd 上回的是 EACCES(13)，不是 ENOTTY(25)

为定位"改了没生效"，用原生 memfd 做了一次最小对照（探针之外的小程序，同一 loader）：

| memfd 名字 | ioctl(V4L2_QUERYCAP) | ioctl(0x64b1, `_IOC_TYPE=='d'`) |
|---|---|---|
| `plain` | `-1 / EACCES(13)` | `-1 / EACCES(13)` |
| `husk-drm` | `-1 / EACCES(13)` | **`-1 / EOPNOTSUPP(95)`** |
| `husk-v4l2-0` | `-1 / EACCES(13)` | `-1 / EACCES(13)` |
| `husk-v4l2-0x` | `-1 / EACCES(13)` | `-1 / EACCES(13)` |

**FACT**：未认领的 memfd 在这台机器上是 `EACCES`，不是 `ENOTTY`。所以旧断言
"errno≠25 就是被 shim 认领了"恒真，三层判定（`husk-v4l2-0x` / `-99` / `husk-drm`）
全部报"claimed=true"，看起来是代码缺陷，其实是**判据**错。`EACCES` 的来源是
SELinux 的 ioctl xperm（`untrusted_app` 域对 tmpfs 文件没有对应 xperm 授权），
与 shim 无关——同一份程序放到别的 ROM 上答案会变。

修法：判据改成"与同进程里名字中性的 memfd 完全同答案"。这样无论对端 ROM 回
`ENOTTY`、`EACCES` 还是别的，判据都成立。

## 4 · 路径语义复核（本轮只复核，不改代码）

`path-probe`（本轮现编，走 GOT 到 shim）：

```
PASS depth-final-16 / depth-mid-16            FAIL 无
PASS depth-final-17 / depth-mid-17
PASS depth-final-40 / depth-mid-40
PASS depth-final-41 / depth-mid-41
PASS depth-40-on-64k-stack / 128k / 256k / 1024k   errno=0
#checks=20 fail=0
```

`04-path-reliability.sh`（guest 内 shell）：`#checks=14 fail=0`，另有 1 条
`SKIP relative-dirfd-contained -- /dirfd-probe not installed`。构建 `dirfd-probe`
需要 `LIBC_SO` 指向 guest 运行时那份被 patch 过的 `libc.so.6`，本轮没做，
所以这条**未验证 UNVERIFIED**，不算通过。

## 5 · 顺带记录：这条链上"文件描述符"的既有口径不变

- 假 fd 的身份**只由内核的 readlink 目标决定**，不由 fd 号决定；所以 `dup2` 用普通
  文件盖掉同一个号之后，ioctl 必须离开模拟层（探针里 8 条 dup 用例覆盖）。
- `Path Shim` 仍然不是安全沙箱：静态 ELF、raw syscall、不经过 loader 的进程
  不受 GOT 改写控制。本轮没有改变这条边界，也没有在任何地方把它说成隔离容器。
