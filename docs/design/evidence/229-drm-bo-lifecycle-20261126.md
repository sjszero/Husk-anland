# 229 · GPU 直通的 DRM 层：BO 只建不释放，以及 dispatch 的 handle 竞态

日期：2026-11-26 00:40（CST）。设备 UKHMMB8PYTGECMNV，桌面在跑。
审计对象是 `runtime/crates/drm-shim`（把 panfrost 的 DRM 语义接到 `/dev/mali0` 的 kbase 上）
和 path-shim 里调它的那段。两条都修了，并且都有修前/修后同机对照。

---

## 1 · 先看结论

| 编号 | 症状 | 修法 | 修前 → 修后 |
|---|---|---|---|
| H-23 | 每建一个 BO 就永久占一次 kbase 分配：**第 64 个开始 `CREATE_BO` 一律 ENOMEM**，Mesa 关缓冲根本不释放 | 实现 `DRM_IOCTL_GEM_CLOSE` → kbase `MEM_FREE`（用我们自己的 kfd）+ 从 BO 表摘掉 | 5 轮 × 60 个 create+close：修前第 2 轮只能建 3 个；修后 300 个全成 |
| H-24 | `DRM_CTX.dispatch()` 没有锁：两个线程同时 `CREATE_BO` 会拿到**同一个 handle**，后写的覆盖前一条 | `DRM_LOCK` 串行化 dispatch | 8 线程 × 100 次：修前出现 2 个重复 handle；修后 8 线程 × 5 轮 × 64 个：0 重复 |

---

## 2 · H-23：BO 只建不释放

### 2.1 现象

`drmrace`（guest 里对 `/dev/dri/renderD128` 直接发 `DRM_IOCTL_PANFROST_CREATE_BO`）：

```
1 thread × 200, 4 KiB : ok=63 errs=137  first errno=12 (Cannot allocate memory)
1 thread × 200, 64 KiB: ok=63 errs=137  first errno=12
1 thread × 200, 1 MiB : ok=63 errs=137  first errno=12
```

**和 BO 大小无关、和线程数无关，永远第 64 个开始 ENOMEM。** 说明是"分配**次数**"的上限，
不是字节数——kbase 每个 context 有分配数上限，而我们在 `CREATE_BO` 里每次都 `MEM_ALLOC`
一次，却从来不发 `MEM_FREE`。

代码侧对得上：
- `KBASE_MEM_FREE` 在 `drm-shim` 里**定义了但没有任何调用点**；
- `DRM_IOCTL_GEM_CLOSE`（DRM 核心释放 GEM handle 的 ioctl，`struct drm_gem_close { u32 handle; u32 pad; }`）
  **没有实现**，落到 `_ => -38`（ENOSYS）。

Mesa 关闭缓冲走的就是 `GEM_CLOSE`。所以每画一帧临时缓冲，就漏一次 kbase 分配——桌面跑得越久，
越接近 ENOMEM。

### 2.2 为什么不能"随便找个 fd 释放"

先试了从 probe 自己打开的 `/dev/mali0` 发 `MEM_FREE`：

```
MEM_FREE(gpu_addr=0x41000) size=8  -> -1 EPERM
MEM_FREE(gpu_addr=0x41000) size=16 -> -1 EPERM
```

kbase 只允许**当初分配它的那个 context** 释放。shim 的分配用的是 `DRM_MALI_FD`，
所以释放也必须用同一个 fd——这也是为什么修法是"在 dispatch 里做"，而不是加一个导出函数。

### 2.3 修法与验证

`DRM_IOCTL_GEM_CLOSE`：按 handle 找到 BO → 用 `self.kfd` 发 `MEM_FREE(gpu_addr = bo.va)`
→ 把最后一条挪到空位、`n_bos -= 1`。未知 handle 回 `-EINVAL`（和真内核一致）；
kbase 释放失败时仍然摘表（留着只会更早撞上限，泄漏一次 VA 不影响正确性）。

修后同一个判据：

```
单线程：5 轮 × 60 个 create+close
round 0..4: created=60 failed=0 duplicates=0 closed=60     ← 300 次分配，零失败
```

修前是 `round 0: 60 ok`、`round 1: 只建成 3 个`、之后全失败。**永久泄漏没了。**

---

## 3 · H-24：`dispatch` 的 handle 竞态

`drm_fake_open()` 的初始化有 `once_begin` 串行化（上一轮 H-16 修的），但**每个 ioctl 的
`DRM_CTX.dispatch()` 没有锁**。`CREATE_BO` 里是"读 `next_handle` → 加一 → 写 `bos[n_bos]` → `n_bos+1`"，
两个线程同时进来就会：拿到同一个 handle、写同一个槽、丢一条 BO。

实测（修前，8 线程 × 100 次）：`ok=63 errs=737`，其中 **duplicate_handles=2**（另一轮 3 个）。
单线程对照：`duplicate_handles=0`——所以重复是并发造成的，不是 handle 生成逻辑本身。

修法：`DRM_LOCK`（同 `MNT_LOCK` 那套 CAS + spin）包住整段 `DRM_CTX.dispatch()`。
里面的 kbase ioctl 都是短调用（`MEM_ALLOC`/`MEM_FREE`/`SUBMIT` 立即返回），没有长阻塞。

修后（新 loader，8 线程 × 5 轮 × 每轮 64 个 create，然后统一 close）：

```
round 0..4: created=63 failed=1 duplicates=0 closed=63
```

`failed=1` 是**活着同时存在 64 个 BO** 撞上 kbase 的分配数上限——这是硬件侧真实语义，
不是竞态；把它 close 掉之后下一轮照样能建 63 个（泄漏已修）。4 线程 × 5 轮 × 60 个则是
`created=60 failed=0 duplicates=0`。

---

## 4 · 回归

- 门禁 A（新 loader）：`PASS=38 FAIL=0`。
- GPU 冒烟（新 loader，桌面在跑）：

```
glxinfo -B:
  MESA: warning: kbase: KBASE_IOCTL_MEM_EXEC_INIT failed: Operation not permitted
  direct rendering: Yes
  Vendor: Mesa (0x13b5)
  Device: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) (0x90800011)
  Accelerated: yes
glxgears: 能启动并打印 "Running synchronized to the vertical refresh"；这**只证明启动路径**，
         不代表长跑通过。后续同机测试在 PanVK/Bifrost 编译器稳定 SIGSEGV，见 evidence 232。
```

## 5 · 记录在案

| 项 | 说明 |
|---|---|
| kbase 每 context 约 64 个并发分配 | 真上限，不是 bug。Mesa 只要正常 `GEM_CLOSE` 就在上限内循环使用；H-23 修完不会累积。 |
| `KBASE_IOCTL_MEM_EXEC_INIT` → EPERM | 可执行 BO 分配被拒，Mesa 打警告后走非可执行路径。已知限制，这一轮不动。 |
| `DRM_LOCK` 会把 `SUBMIT` 也串起来 | 目前 `SUBMIT` 只是把 kbase job 投进去就返回，实测 glxinfo/gl-probe 正常；`glxgears` 只验证到启动，长跑在 Mesa 编译器崩溃（evidence 232）。如果以后出现多线程提交瓶颈，再把 job 提交拆出锁外。 |
