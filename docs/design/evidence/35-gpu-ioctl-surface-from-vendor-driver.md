# evidence 35 — 从厂商驱动反读 kbase ioctl 命令面，纠正 evidence 31 的假阴性

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-22 22:09–22:12（CST）
结论先行：**`JOB_SUBMIT` 在 guest（untrusted_app）里是可达的，返回 EINVAL 而不是 ENOTTY。
evidence 31 记录的"nr=2 全 size ENOTTY → 提交接口待查"是测量错误，C5 的阻塞点不存在。**

## 1. 方法：不猜 header，直接读厂商自己的代码

     adb pull /vendor/lib64/egl/libGLES_mali.so      # 38 MB，指向 mt6893/libGLES_mali.so
     strings -a libGLES_mali.so | grep r32p1          # "1.4 Valhall-r32p1-01eac0"
     llvm-objdump -d --no-show-raw-insn libGLES_mali.so > dis.txt   # 721 万行
     # 找所有 ioctl@plt 调用点（53 处），回溯 w1 的 movz/movk 立即数

为什么用这个方法：厂商用户态驱动就是这份内核驱动唯一的真实客户端，它发的命令字必然被接受。
猜 header 常量的做法已经在 VERSION_CHECK 的编码上出过一次错（size 必须 4，不是 0）。

## 2. 反读结果：命令字全集（FACT，来自二进制）

| 命令字 | dir/size | nr | 说明 |
|---|---|---|---|
| `0xc0048000` | RW,4 | 0 | VERSION_CHECK（实测回填 11.38） |
| `0x40048001` | W,4 | 1 | SET_FLAGS（实测 errno=0，kctx 建成） |
| **`0x40108002`** | W,16 | **2** | **JOB_SUBMIT** — 结构 `{u64 addr; u32 nr_atoms; u32 stride}` |
| `0x40108003` | W,16 | 3 | |
| `0xc0208005` | RW,32 | 5 | 内存分配类 |
| `0xc0108006` | RW,16 | 6 | |
| `0x40088007` | W,8 | 7 | |
| `0x8004800c` | R,4 | 12 | |
| `0x4010800d` | W,16 | 13 | |
| `0x4018800e` | W,24 | 14 | |
| `0x4020800f` | W,32 | 15 | |
| `0x80048011` | R,4 | 17 | |
| `0x40048012` | W,4 | 18 | |
| `0x40108014` | W,16 | 20 | |
| `0xc0208015` | RW,32 | 21 | |
| `0xc0188016` | RW,24 | 22 | |
| `0x40188017` | W,24 | 23 | |
| `0x40208018` | W,32 | 24 | |
| `0x40048019` | W,4 | 25 | |
| `0x4010801b` | W,16 | 27 | |
| `0x4010801c` | W,16 | 28 | |
| `0x4010801d` | W,16 | 29 | |
| `0x4010801e` | W,16 | 30 | |
| `0x40088026` | W,8 | 38 | |
| `0xc0208032` | RW,32 | 50 | |
| `0xc0018036` | RW,1 | 54 | |

（`0x2400/0x2401/0x2403/0x8004` 是 TTY/其它库的 ioctl，不属于 kbase。）

## 3. 复现与实测（FACT）

     runtime/crates/gpu-probe submit0        # 新增模式，按厂商顺序打三条

    gpu|version_check(0,0) 0xc0048000|PASS|errno=0 -> 11.38
    gpu|set_flags 0x40048001|PASS|errno=0
    gpu|job_submit 0x40108002 nr_atoms=0|ANSWERED|errno=22 (EINVAL)

`nr_atoms=0` 让驱动在校验阶段就拒绝（EINVAL），因此这条命令**没有向硬件提交任何 job**，
21:30 那次内核 panic 的风险路径没有被触碰。

判定依据：ENOTTY(25) = 命令字不在驱动的 switch 表里；EINVAL(22) = 命令字对、参数被拒。
这次是后者，说明 guest 进程确实走进了 `kbase_ioctl_job_submit()`。

## 4. evidence 31 为什么错（INFERENCE，未验证）

evidence 31 的 `nrscan` 用"猜的 (nr,size) 组合"扫，得到的"已知命令面"
（0/1/5/6/16/21/22/31 + 尺寸）与本表对不上：厂商实际用的是 0/1/2/3/5/6/7/12/13/14/15/17/18/20/21/22/23/24/25/27/28/29/30/38/50/54。
两者交集只有 0/1/5/6/21/22 —— 说明那次扫描用的 nr→size 映射整体是错的，
`nr=2` 很可能从没被用 `size=16` 试过（或用了 RW 方向）。

**纪律修正**：以后判定"内核不支持某命令"前，必须先从真实客户端（厂商 .so / 内核字符串）取命令字，
再用最小参数打一次；不允许用猜的组合宣布"不支持"。

## 5. 下一步（C5 真正的工作）

命令面已经确认存在，剩下的是构造合法 atom，而不是打通内核：

1. 用 nr=5（RW,32）拿到 GPU VA 区间 → 得到 `gpu_va`；
2. 在可用内存里写最小 jc（job chain），先做"空 job / 单 atom 立即完成"；
3. `JOB_SUBMIT` 传 `{addr=atoms_array, nr_atoms=1, stride=sizeof(base_jd_atom_v2)}`，
   stride 必须是 8 的倍数，atom 里的 core_req 先只填 `BASE_JD_REQ_CS`；
4. 用 nr=4/… 的完成路径或直接看 dmesg 之外的返回值验证 atom 完成。

风险与边界：JM 模型的 atom 结构（core_req / atom_number / jc / udata）必须逐字段对齐，
否则驱动会回 EINVAL 或直接 hang kctx（不是 panic，但会拖死进程）。一次只改一个字段。
