# 127 · GPU 家族计划表落地：真节点不遮蔽、无适配器就明说原因并降级

日期：2026-11-23　设备：RMX3357 / Mali-G77 / Android 13 / 4 KiB（`adb` serial `UKHMMB8PYTGECMNV`）
被测物：`tools/loader/ld-embedded.so`（新增 `path-shim/src/plan.rs`），设备上为 `files/libld5.so`

## 1. 改了什么

`docs/design/device-adapters.md` 早就写了"探测一次、给原因、失败降级、不按机型特判"的规则，
但代码里没有这个决策点：`open("/dev/dri/renderD128")` 一律交给 Mali 的 kbase 假节点，
在别的芯片上就是一条**静默 ENOENT**，而且会把真实存在的渲染节点遮蔽掉。

新增 `plan.rs`（no_std，raw syscall，缓存一次，与 `rootfd()` 同样的 CAS 纪律）：

| 判据（按顺序，全部只用节点） | family | 之后的动作 |
|---|---|---|
| `NR_FORCE_SOFTWARE_GPU` 存在（**测试钩子**） | `software` | 拒绝 DRM 节点，Mesa 用 llvmpipe |
| `/dev/mali0` 可 rw 打开 | `mali-kbase` | 走我们的 DRM 语义层（现有实现） |
| `/dev/kgsl-3d0` 可 rw 打开 | `adreno-kgsl` | 适配器未实现 → 拒绝 + 日志 |
| `/dev/dri/renderD128` 可 rw 打开 | `drm-node` | **透传真节点**，不再遮蔽 |
| 都打不开 | `software` | 拒绝 + 日志，`reason` 里带每个节点的 errno |

只有 `mali-kbase` 才把 `/dev/dri` 目录换成我们的假目录、才接受 `renderD128`/`card0` 的 open；
其它家族一律走真实路径。降级时输出**一条**（每进程一次）：

```
nr-plan: drm node -> gpu=software reason=NR_FORCE_SOFTWARE_GPU
nr-plan: drm node -> gpu=software reason=mali0=13 kgsl=2 dri=2 no gpu node openable
```

## 2. 实测（FACT）

设备上 `run-as com.linuxhub.husk`，`NR_LD=files/libld5.so`：

| 场景 | 命令 | 结果 |
|---|---|---|
| 本机正常（Mali） | `gl_probe` | `gl\|open_render\|PASS\|fd=4`（假节点仍在） |
| 本机正常（Mali） | `eglinfo -B`（Zink/PanVK） | `panvk (Mali-G77 MC9)` + `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，与 evidence 47 一致，无回归 |
| 强制降级 | `NR_FORCE_SOFTWARE_GPU=1 gl_probe` | `nr-plan: drm node -> gpu=software reason=NR_FORCE_SOFTWARE_GPU`，`gl\|open_render\|FAIL\|errno=2`，`GL_RENDERER=llvmpipe`，**不崩** |
| 强制降级 + panfrost 覆盖 | `MESA_LOADER_DRIVER_OVERRIDE=panfrost NR_FORCE_SOFTWARE_GPU=1 gl_probe` | 同上（dri2 路径也被同一决策点挡住） |
| `/dev/dri` 列表 | `ls /dev/dri` | `card0` `renderD128`（Mali 下仍是我们的假目录） |
| 路径回归 | `04-path-reliability.sh` | `#checks=31 fail=0` + `#checks=15 fail=0` |
| 功能回归 | bash 矩阵 | `cat /etc/os-release`、877 个二进制、嵌套 exec、`/tmp` 写读、`ls -l` stderr 0 字节、`readlink -v` errno 文本正确 |

## 3. 仍未验证（UNKNOWN，不写成功）

- **Adreno(kgsl)**：本机没有 `/dev/kgsl-3d0`，`adreno-kgsl` 这一行只有代码路径，没有实机。
- **真实 DRM 节点透传**：本机 `/dev/dri/renderD128` 在 app 域打不开，`drm-node` 分支没有实机。
- PanVK 是**直接** open `/dev/mali0` 的（evidence 47 的 `Found kbase device`），不经过
  `/dev/dri`，所以本计划表对它不产生影响——也就是说本表管的是 Mesa 的 `dri2`/DRM 语义路径。
  这一点必须写清楚，否则会被误解成"GPU 全路径都被这张表管住了"。
