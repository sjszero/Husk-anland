# Evidence 43 — M1：DRM 的 BO 语义能落在 kbase 显存上

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-23 02:1x（CST）
工具：`runtime/crates/drm-probe selfcheck` + 新 crate `runtime/crates/drm-shim`（纯 Rust）

> M1 的目标不是"跑 Mesa"，而是先把 **DRM 语义 → kbase** 这一层的正确性钉死：
> `CREATE_BO` 出来的东西，必须真的是一块能 CPU 读写的显存，且 offset 必须是 GPU VA。

## 1. 实测（FACT，一次跑完）

```
drm|selfcheck/ctx|PASS|props=83 GET_PARAM 可用
drm|selfcheck/create_bo|PASS|size=4096 handle=1 offset(=gpu_va)=0x41000
drm|selfcheck/mmap_bo|PASS|handle=1 offset=0x41000
drm|selfcheck/mmap|PASS|va=0x41000 写入并读回 64 字节
drm|selfcheck/paramGPU_PROD_ID|PASS|0x9080
drm|selfcheck/paramJS_PRESENT|PASS|0x7
drm|selfcheck/paramMAX_THREADS|PASS|0x400 (1024)
drm|selfcheck/get_bo_offset|PASS|0x41000（与 MMAP_BO 一致）
drm|selfcheck/verdict|PASS|M1 语义层：DRM BO == kbase 显存，可 CPU 读写
```

要点：

- `CREATE_BO.size=4096` → kbase `MEM_ALLOC(va_pages=1, flags=0xf)` → **VA `0x41000`**；
- DRM 的 `offset` 直接用 kbase 回写的 **GPU VA**（不另造句柄号），这样 Mesa 拿到的地址就是 GPU 真地址；
- 真 `mmap(fd=/dev/mali0, offset=0x41000, prot=RW)` 后写 16 个 u32 再读回，**完全一致**；
- `GET_PARAM` 走 evidence 42 的表，三条关键值都对（`0x9080` / `0x7` / `1024`）。

## 2. 实现位置

`runtime/crates/drm-shim/src/lib.rs`：`DrmCtx`（kbase context + props 缓存 + BO 表）+ `dispatch(request, arg)`。
ioctl 编号按 `_IOC(dir,'d',0x40+nr,size)` 现算，`SUBMIT` 用 `_IOW`，其余 `_IOWR`。

`dispatch` 现在覆盖：

| DRM ioctl | 状态 |
|---|---|
| `CREATE_BO` | ✅ kbase `MEM_ALLOC`，offset = GPU VA |
| `MMAP_BO` / `GET_BO_OFFSET` | ✅ 回 GPU VA |
| `GET_PARAM` | ✅ evidence 42 的表 |
| `SUBMIT` | 已实现（构造 72 字节 atom，`core_req` = FS/CS），但**未验证**（M3） |
| `WAIT_BO` | 暂直接放行（真实等待要等 KINSTR 记录，M3） |
| `MADVISE` / `SET_LABEL_BO` / `SYNC_BO` / `JM_CTX_*` / `QUERY_BO_INFO` | 簿记或 0 |
| `PERFCNT_*` | `-ENOSYS` |

## 3. 下一步（M1 收尾 → M2）

1. 把 `dispatch` 接到 path-shim 的 interpose：`open("/dev/dri/renderD128")` 返回假 fd；
   该 fd 上的 `ioctl` 进 `dispatch`，`mmap` 用 `MMAP_BO` 回的 offset 转到 `/dev/mali0`。
2. 然后跑 `vainfo`/`eglinfo` 看 panfrost probe 能不能过（M2）。

## 4. 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
cargo build --release --target aarch64-unknown-linux-gnu -p drm-probe   # 交叉编译参数同 evidence 41
adb push target/aarch64-unknown-linux-gnu/release/drm-probe /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/drm-probe files/rootfs/usr/bin/'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/drm-probe selfcheck'
```
