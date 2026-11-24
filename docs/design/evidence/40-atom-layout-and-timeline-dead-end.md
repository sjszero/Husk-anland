# Evidence 40 — atom 布局钉死了一半；timeline fd **不是**完成信号（推翻旧结论）

> **后续更正（evidence 57）**：本文件只得到了“72-byte stride 可被接受”，并没有证明 64-byte
> v2 不可用。后来确认 r32p1 同时支持 64 B `base_jd_atom_v2` 与 72 B `base_jd_atom`；本文件
> 的 soft-job 结论受字段偏移混用影响，不是内核不支持。原文保留，作为当时的实测记录。

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-22 23:5x–2026-11-23 00:3x（CST）
工具：`runtime/crates/gpu-probe`（纯 Rust，guest 内运行，经 Husk loader 启动）

> 本轮的目的只有一个：**"提交被接受"之后，怎么知道 job 跑完了**。
> 结果：证伪了一个假阳性，钉死了三项 ABI 事实，把软作业（soft job）这条路走死。

## 1. 内核自己报的 DDK 版本（FACT）

```
gpu|ddk_version|PASS|returned 20 bytes: "K:r32p1-00bet5(GPL)"
```

`KBASE_IOCTL_GET_DDK_VERSION` = `0x4010800d`（nr 13，结构 `{u64 buf; u32 size; u32 pad}`）。
**内核 kbase = r32p1**，与厂商 `libGLES_mali.so`（r32p1）同代。
⇒ 之前"厂商 .so 的 atom 布局可能比内核新"的担心不成立：两者是同代。

## 2. atom 数组确实被内核拷进去了（FACT）

```
gpu|badptr|REFUSED|addr=0xdead0000 nr_atoms=1 stride=72 errno=14 (EFAULT)
gpu|badptr|REFUSED|addr=0x1        nr_atoms=1 stride=72 errno=14 (EFAULT)
gpu|badptr|REFUSED|addr=0x0        nr_atoms=1 stride=72 errno=14 (EFAULT)
```

⇒ `JOB_SUBMIT` 不是空转：它 `copy_from_user` 了我们给的地址。这条把"errno 0 可能是啥也没干"排除掉一半。

## 3. stride 必须是 72（FACT）

```
gpu|stride|ACCEPTED|stride=72 errno=0 (ok)
gpu|stride|REFUSED |stride=56 errno=22 (EINVAL)
```

厂商 .so 里也是这个数（`mov w9, #0x48` @0x88120c；`madd x8, x9, #0x48, x8` @0x7c4cb0）。
⇒ 本内核 `sizeof(base_jd_atom_v2) == 72`，不是公开头（rockchip 4.19）的 56。

## 4. timeline fd **不是**完成信号（FACT，推翻 evidence 38 §6 与一次假阳性）

```
gpu|stream_create 0x40208018|PASS|timeline fd=5
gpu|poll(timeline)|revents=0x0                      ← 新建、未提交
gpu|fence0/control/job_submit|PASS|core_req@0x40=0x0
gpu|fence0/control/poll(timeline)|READABLE|revents=0x1 after 0ms
gpu|fence1/vendor/job_submit|PASS|core_req@0x40=0x202
gpu|fence1/vendor/poll(timeline)|PASS|revents=0x1 after 0ms
gpu|fence4/wait/job_submit|PASS|EVENT_WAIT on 0x41000
gpu|fence4/wait/poll(before)|READABLE|revents=0x1 after 0ms   ← 本该阻塞的 atom 也让 fd 可读
gpu|rd/read|FAIL|errno=22 (EINVAL)                            ← 这个 fd 不支持 read()
```

- 只要有一次**被接受**的 submit，fd 就变可读；`core_req = 0` 的纯依赖 atom 也一样。
- 一个"应当永远阻塞"的 `SOFT_EVENT_WAIT` atom 也是立刻可读。
- `read()` 直接 EINVAL（没有 read 实现）。

⇒ **`STREAM_CREATE` 的 fd 不能当完成通道**。本轮开始时我自己先报过一次 `fence1 ... PASS`，
那是假阳性，已被 `fence0` 对照打掉。证据 38 §6 里"可 poll = 完成通道"的结论一并作废。

## 5. 内核不会把 sync_file fd 写回 `base_fence`（FACT）

```
gpu|fence6/job_submit|PASS|errno=0 fence after submit = {fd: -1, stream_fd: 5}
gpu|fence6/sync_file|INFO|kernel did not write an fd back
```

`base_jd_fence_trigger_setup_v2` 要求提交前 `basep.fd == -1`，
`base_jd_fence_wait_setup_v2` 要求 `basep.fd >= 0` —— 说明 fd 是触发 atom 的**输出**。
本内核没有写回，这条也不通。

## 6. atom 里哪个 4/8 字节是 `core_req`：扫到边界就扫不动了（FACT）

零填充 atom + `jc` = 合法 GPU VA（0x41000），只在一个偏移写值：

| 写入位置 | 值 | 结果 |
|---|---|---|
| 0x40（厂商 .so 的 core_req 位置） | 0x202 / 0x206 / 0xffff | 全部 ACCEPTED |
| 0x34（公开头 56 字节布局的 core_req） | 0x202 / 0x206 / 0xffff | 全部 ACCEPTED |
| 0x30 / 0x24 | 0x206 | ACCEPTED |
| **0x38 或 0x3c（8 字节字段的高/低半）** | 任何非零 | **EINVAL(22)** |
| 0x38 | 0x0 | ACCEPTED |

- `0xffff` 这种不可能合法的位模式在 0x34/0x40 都被接受 ⇒ 这两个位置**不是**被校验的 `core_req`。
- 0x38 有个被校验的 8 字节字段，必须为 0 —— 与厂商 .so 在 0x38 写 0 的行为一致
  （`stp x23, xzr, [x8, #0x30]`：0x30 写指针、0x38 写 0；`str w9, [x8, #0x40]` 写 0x202）。
  推断（INFERENCE）：0x38 起是 r32 新增字段（JIT id 一类），非零要 JIT 支持，否则拒。

## 7. 软作业在本机跑不起来（FACT）

让内核自己的 soft-job 处理器往显存写 1 字节，然后 CPU 读回（这是唯一不依赖完成 API 的可观测副作用）：

```
gpu|sevt/bo|INFO|gpu_va=0x41000 pre=[0x00]
gpu|sevt/submit@0x40|ACCEPTED|core_req=0x206 errno=0
gpu|sevt/read +10/+40/+150ms|UNCHANGED|event word = 0
（同样 UNCHANGED：@0x34、@0x30、@0x24、以及 compat_core_req@0x22 + 0x40/0x34 双写、
  以及先 MEM_JIT_INIT 再提交）
```

对照：`MEM_JIT_INIT`（`0x4018800e`）本身返回 0，说明它能被调用；atom 仍然不动。

再把"内核够不着我们的 BO"这个漏洞堵上——换分配标志重跑：

```
gpu|sevtf/submit@0x40 flags=0x100f|ACCEPTED|gpu_va=0x41000     (BASE_MEM_CACHED_CPU)
gpu|sevtf/read +200ms|UNCHANGED|event word = 0
gpu|sevtf/submit@0x40 flags=0x40f|ACCEPTED|gpu_va=0x41000      (BASE_MEM_COHERENT_SYSTEM)
gpu|sevtf/read +200ms|UNCHANGED|event word = 0
gpu|sevtf/alloc|FAIL|flags=0x400f                              (COHERENT_SYSTEM_REQUIRED 需要 coherent group 信息)
```

⇒ 换成 cached CPU 映射和 coherent 映射都读到 0，`0x400f` 连分配都不给。软作业无效不是映射类型的问题。
`MEM_SYNC`（`0x4020800f`，`{handle,user_addr,size,type}`）返回 EINVAL，缓存维护这条路也没打通
——不过分配时未带 `BASE_MEM_CACHED_CPU`（0x1000），CPU 映射按未缓存处理，所以"读到 0"大概率是真的 0。

⇒ **r32p1 的 soft job 在这台机器上不产生任何效果**。GPU 完成通道必须靠真正的硬件 job
把值写进显存，或者找到 r32 的 sync 机制。

## 8. 结论与下一步（按优先级）

FACT：内核 r32p1；atom 72 字节、数组被拷贝；stride 72；timeline fd 不可读、POLLIN 无意义；
`base_fence` 无 fd 回写；软作业无效。

UNKNOWN：0x34/0x40 都不被校验，那 `core_req` 到底在哪；atom 是否真的进队列。

下一步（每条都可复现、都不碰"盲扫 ioctl"）：

1. **拿到 r32p1 的 `base_jd_atom_v2` 定义**（MTK MT6893 GPL 内核源 / r32p1 DDK 头），
   不再靠偏移试探。这是最快解。
2. 若拿不到：用"**GPU 必须写显存才能算跑完**"作为唯一口径，做一个最小硬件 job
   （Valhall job chain：先只求把常量写进 BO），一次只验证一个字段。
3. 排除分配标志的影响：同一实验用 `0x100F`（CACHED_CPU）、`0x40F`（COHERENT_SYSTEM）再跑一次，
   确认"软作业无效"不是 kcpu 映射缺失导致的。
4. 若 1–3 全部指向"这台机器的 kbase 不允许第三方提交"，则目标第 2 条要改成
   厂商 EGL 的零拷贝方案（app 进程内驱动 + guest 侧 Surface 直投），并在 evidence 里写明原因。

## 9. 复现命令

```sh
cd $REPO
adb push runtime/target/aarch64-unknown-linux-gnu/release/gpu-probe /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/gpu-probe files/rootfs/usr/bin/'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe ddkver'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe badptr'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe stride_probe'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe stream1'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe fence0'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe fence4'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe fence6'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe cr 0x40 0xffff'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe cr 0x38 0x202'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe sevt 0x40'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe sevt2 0x34'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/gpu-probe sevtjit 0x40'
```
