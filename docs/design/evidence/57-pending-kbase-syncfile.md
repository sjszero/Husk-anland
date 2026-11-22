# Evidence 57 — r32p1 kbase 生成真正的 pending sync_file

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191+ / 4 KiB 页  
时间：2026-11-23  
结论：**PASS（内核层 producer fence）**。guest 进程直接向 `/dev/mali0` 提交两个 64-byte
`base_jd_atom_v2`，内核回写一个真实 `sync_file` fd。该 fd 在事件未设置时保持 pending，
`SOFT_EVENT_UPDATE` 后同一个 fd 变 readable，`SYNC_IOC_FILE_INFO.status` 从 0 变为 1。
Vulkan、SurfaceFlinger 接入仍是下一步，不能把本证据写成完整 WSI 已完成。

## 1. 根因与修正（FACT）

两个错误叠加导致此前的 soft-fence 实验成为假阴性：

1. **atom 布局混合**：r32p1 有两套结构，必须整体二选一：
   - 64-byte `base_jd_atom_v2`：`atom_number` u8 @ `0x28`，`core_req` u32 @ `0x2c`；
   - 72-byte `base_jd_atom`：`seq_nr` @ `0x00`，`jc` @ `0x08`，`atom_number` @ `0x30`，
     `core_req` @ `0x34`。
   把 72-byte stride 与 64-byte 字段偏移混用会得到 `JOB_CONFIG_FAULT (0x40)`。
2. **`SAME_VA` 的返回值含义**：`MEM_ALLOC(flags=0x300f)` 在 `mmap` 前返回的是 cookie
   （本次为 `0x41000`），只用于 `mmap` 的 offset。`mmap` 成功后，映射地址本身才是 GPU VA。
   把 pre-mmap cookie 交给 `SOFT_EVENT_WAIT/SOFT_EVENT_UPDATE` 会让 `kbase_vmap()` 找不到
   region，表现为 `JOB_CANCELLED (0x4002)` 或 `ENOENT`。

源码依据（MTK r32p1 GPL）：

- `mali_base_jm_kernel.h`：`struct base_jd_atom_v2` / `struct base_jd_atom` 字段定义；
- `softjobs.c`：`SOFT_EVENT_WAIT` 用 `kbase_vmap(kctx, katom->jc, ...)` 读取事件字节；
- `mem_linux.c`：`SAME_VA` 分配先返回 cookie，`mmap` 时才建立最终 VA；
- `sync_file.c`：`SOFT_FENCE_TRIGGER` 通过 `sync_file_create()` 生成 fd 并回写到 `base_fence.fd`。

## 2. 最小提交

探针 `gpu-probe fence11`：

```text
atom 1: 64-byte v2, core_req=0x205 SOFT_EVENT_WAIT, jc=<mmap 地址>
atom 2: 64-byte v2, core_req=0x202 SOFT_FENCE_TRIGGER,
        pre_dep[0]={atom_id=1,type=ORDER}, jc=&base_fence
JOB_SUBMIT: nr_atoms=2, stride=64
```

事件页使用 `MEM_ALLOC(flags=0x300f)`：`CACHED_CPU | SAME_VA | CPU_RD | CPU_WR |
GPU_RD | GPU_WR`。先写入 0，再提交；250 ms 后调用 `SOFT_EVENT_UPDATE(event=<mmap 地址>,
new_status=1)`，然后轮询同一个 `base_fence.fd`。

## 3. 真机结果

连续 3 次最终验证均为同一结果；日志原文在
[`57-pending-kbase-syncfile-guest.log`](57-pending-kbase-syncfile-guest.log)。

```text
gpu|fence11/event_bo|PASS|cookie=0x41000 mapped=... gpu_va=... flags=0x300f value=0
gpu|fence11/job_submit|ACCEPTED|errno=0 (ok) layout=v2-64 atoms=2 fence_after_submit={fd:6,stream_fd:5}
gpu|fence11/sync_file(pending)|PASS|errno=0 name="mali-mali.timeline..." status=0 num_fences=1 flags=0x0
gpu|fence11/poll(before set)|TIMEOUT(expected pending)|fd=6 poll=0 revents=0x0 after 251ms
gpu|fence11/soft_event_set|PASS|event=... errno=0 (ok)
gpu|fence11/poll(after set)|READABLE|fd=6 poll=1 revents=0x1 after 0ms
gpu|fence11/sync_file(signaled)|PASS|errno=0 name="mali-mali.timeline..." status=1 num_fences=1 flags=0x0
gpu|fence11/event|DONE|#0 event_code=0x1 atom_number=1
gpu|fence11/event|DONE|#1 event_code=0x1 atom_number=2
```

验收点逐项满足：

1. `JOB_SUBMIT` 接受两个 atom，内核回写 `fence.fd=6`；
2. `SYNC_IOC_FILE_INFO` 证明 fd 是 `mali-mali.timeline*` 的 sync_file，不是 eventfd；
3. set 前 `status=0` 且 `poll` 250 ms 超时；
4. set 后同一 fd `poll` readable，`status=1`；
5. 两个 atom 的完成事件均为 `DONE (0x1)`，没有 `JOB_CANCELLED` / `JOB_CONFIG_FAULT`。

## 4. 边界（UNKNOWN / 未完成）

- **FACT**：本证据只证明 r32p1 kbase 可以生成并 signal 一个真实的 pending `sync_file`。
- **UNKNOWN / 未完成**：把这个 fd 接入 FristOneRR PanVK 的 `vkGetFenceFdKHR` / `SYNC_FD` 路径；
- **UNKNOWN / 未完成**：SurfaceFlinger 在 GPU 尚未完成时实际等待该 sync_file；
- **UNKNOWN / 未完成**：持久资源池、WSI / swapchain、Wayland / X11、多窗口与 10 分钟门禁；
- 当前 eventfd bridge 仍是可用 fallback，不能因为本证据就删除。

## 5. 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime

NDK=/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64/bin
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$NDK/clang \
RUSTFLAGS="-C link-arg=--target=aarch64-linux-gnu \
-C link-arg=--sysroot=/tmp/gccroot \
-C link-arg=-B/tmp/gccroot/usr/lib/gcc/aarch64-linux-gnu/13 \
-C link-arg=-L/tmp/gccroot/usr/lib/aarch64-linux-gnu" \
cargo build --release --target aarch64-unknown-linux-gnu -p gpu-probe

adb -s UKHMMB8PYTGECMNV push \
  target/aarch64-unknown-linux-gnu/release/gpu-probe /data/local/tmp/gpu-probe
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk cp /data/local/tmp/gpu-probe files/rootfs/usr/bin/gpu-probe'
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "cd files && timeout 20 sh gexec2.sh rootfs usr/bin/gpu-probe fence11"'
```

## 6. 产物哈希

```text
guest probe sha256: c043e256ceecb34d90ed1b21041aca230905286c4fdf77f2df90f5e32bf954c3
guest log sha256  : 987f02a9c82dc06ccebefe6aac5b55804c2fa35a742d23537712fd576fa780b3
```
