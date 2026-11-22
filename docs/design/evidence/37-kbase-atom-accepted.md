# evidence 37 — JOB_SUBMIT 接受了一个 atom（ioctl 返回 0）

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-22 22:47（CST）
上下文：guest（glibc，`untrusted_app`），`gpu-probe submit1`

## 1. 结果（FACT）

```
gpu|job_submit|ANSWERED|nr_atoms=1 stride=72 errno=0 (ok)
gpu|job_submit|ANSWERED|nr_atoms=0 stride=72 errno=0 (ok)
```

- 提交结构 `{u64 addr; u32 nr_atoms; u32 stride}`，`addr` 指向**用户态** 72 字节全零数组
  （说明 kbase 的 `kbase_pointer` 收的是用户指针，不需要先把 atom 放进显存）。
- 与 evidence 35 的 `submit0`（addr=0, nr_atoms=0, **stride=0** → EINVAL）对比：
  **stride 是必填项**，`stride=0` 会被拒；`stride=72` 被接受。

## 2. 这证明了什么、没证明什么

证明（FACT）：命令数字、结构布局、stride 三者都对得上，内核**进入并完成了**
`kbase_ioctl_job_submit()` 的校验路径，返回 0。

**没有证明**（UNKNOWN）：atom 真的被执行。jc=0 的 atom 大概率在 GPU 侧 fault，
而 kbase 的完成/错误是通过事件（atom 的 udata / 事件通道）异步回传的，不是 ioctl 返回值。
我们现在还没有接事件通道，所以"提交成功"≠"跑完了"。

**下一步**：接完成通道（atom 的 udata 写回 / kbase 事件），再用一个能证明"GPU 读了我们的内存"
的最小 job（例如往 BO 里写字）来升级这条证据。

## 3. stride = 72 的来源（FACT，来自厂商 .so 反汇编）

- `0x8811fc`：`w9 = 0x48`（72），`stp w2, w9, [sp, #0x10]` → `{addr, nr_atoms, stride=72}`；
- `0x859e04`：`add x22, x22, #0x48`（逐 atom 前进 72 字节），与上面一致。

## 4. mem_query 仍未对齐（FACT）

按公开 kbase UAPI 的 `union kbase_ioctl_mem_query`（in `{u64 gpu_addr; u64 query}`，
query 取 1/2/3）打，三种都返回 **EINVAL**：

```
mem_query COMMIT_SIZE errno=22 value=0x41000
mem_query VA_SIZE     errno=22 value=0x41000
mem_query FLAGS       errno=22 value=0x41000
```

说明这台内核的 query 语义与公开头不一致（MTK 改过，或需要先带别的字段）。
**不猜**：下一步从厂商 .so 的 `0x881be4` 调用点再读一次它实际填的第二个字段
（现在看到它填 `1`，与我们一致，所以差异在别处 —— 待查是否要求 gpu_addr 与区域起点严格相等，
或 query 编号不同）。

## 5. 复现

    cd runtime && <NDK/gccroot 交叉编译命令见 evidence 31>
    adb push target/aarch64-unknown-linux-gnu/release/gpu-probe /data/local/tmp/
    adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/gpu-probe files/rootfs/usr/bin/'
    adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe submit1'
