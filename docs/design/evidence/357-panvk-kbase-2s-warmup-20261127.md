# 357 · PanVK 的 kbase「2 秒热身」：桌面冷启动少 4 秒的根因与补丁

日期：2026-11-27 21:20（CST）。
设备 `UKHMMB8PYTGECMNV`（RMX3357 / MT6893 / **Mali-G77 MC9** / Android 13 / kernel 4.19.191 /
4 KiB 页），测试宿主 `com.linuxhub.myth.husk`，guest 树
`files/linux/distributions/ubuntu-24.04/rootfs`。

本文件回答三件事：**为什么每个 GL 进程要空等 4 秒**、**补丁改的是哪 16 个字节**、
**改完拿什么证据证明没有变坏**。全部数字都是本轮在这台机器上重测的。

## 0 · 结论

- **FACT**：随包携带的 FristOneRR PanVK 1.2.0（Mesa 26.3.0-devel 源码线）在
  `src/panfrost/lib/kmod/kbase_kmod.c` 里，**每建立一个 kbase JM context 就无条件
  `nanosleep(2s)`**。这条补丁是为 Mali-G57 打的「第一个 job 会被终止」绕行。
- **FACT**：一个 GL/Vulkan 进程在这条链上会建**两次** context，所以每次启动白等 **4 秒**；
  `glxinfo -B` 恒定 4.3 s、桌面冷启动首帧前那段「0% CPU 空转」就是它。
- **FACT（A/B 同会话实测）**：只把 ICD 的 `library_path` 指回未打补丁的原件，
  `vulkan-probe` 4075/4061 ms → **68/75 ms**，`glxinfo -B` 4306/4273 ms → **289/271 ms**。
- **FACT**：补丁后 19 项 readback 门禁 `PASS=19 FAIL=0`、`READBACK_AUDIT PASS=6 FAIL=0`，
  renderer 仍是 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。
- **FACT**：冷启动（点「打开桌面」→ `SurfaceView` 出现）13.88 / 10.08 / 10.55 s，
  中位 **10.55 s**；补丁前 5 次为 14.52–15.37 s（evidence 353）。**净省约 4.1 s**。

## 1 · 根因

上游源码（`fristonerr-1.2.0`，commit `0c9dce35094dbe89c412b79bf57c6067c7b97e59`）：

```c
/* Empirically, this Mali-G57 kbase JM backend needs ~2s of real
 * wall-clock time after context creation before the GPU power domain
 * is ready for job-slot submissions. Submitting before this elapses
 * reliably TERMINATES the first atom regardless of retry count or
 * backoff length (both were tested extensively and never helped).
 * Sleep once here, at context creation, instead of retrying the
 * first job. */
if (!is_csf) {
   struct timespec warmup_ts = { .tv_sec = 2, .tv_nsec = 0 };
   nanosleep(&warmup_ts, NULL);
}
```

这段是**为 Mali-G57 的 JM 后端**加的，但被无条件继承到所有 kbase 设备。本机是
Mali-G77 MC9，不需要它。

对上了 evidence 353 / defect H41 里那条一直没解释的现象：`gnome-shell` 创建到首帧之间
有 **4.1 s 纯空转（utime/stime 增量为 0）**，随后才满核算出首帧。4.1 s ≈ 2 次 context × 2 s。

## 2 · 补丁

不改逻辑、不重编 Mesa，只把 `.rodata` 里那个 16 字节 `timespec` 清零：

| 项 | 值 |
|---|---|
| 上游驱动 sha256 | `599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c` |
| 补丁后 sha256 | `f73600da109ea301ad77663cdab5fcf2b1931ecd2e623a9d30b1b72136e25ee5` |
| 偏移 | `0x53a570`（十进制 5481840） |
| 长度 | 16 字节 |
| 改动 | `02 00 00 00` + 12×`00` → 16×`00`（`tv_sec 2 → 0`） |

设备上直接按字节核对（`run-as`，不经过 shim）：

```text
$ od -An -tx1 -j 5481840 -N 16 .../libvulkan_panfrost_husk.so
 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
$ od -An -tx1 -j 5481840 -N 16 .../.libvulkan_panfrost_husk.so.bak-orig
 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
$ sha256sum .../libvulkan_panfrost_husk.so
 f73600da109ea301ad77663cdab5fcf2b1931ecd2e623a9d30b1b72136e25ee5
```

- 安装器：`tools/guest/install-husk.sh` 的 `_panvk_warmup_off()` 先核对上游 SHA，
  再按字节改，再用 `f73600da…` 复核；`HUSK_PANVK_KEEP_WARMUP=1` 保留上游原样。
- 源码等价补丁：`docs/design/patches/fristonerr-kbase-warmup-skip.patch`
  （改成 `PANVK_KBASE_WARMUP_MS` 可配，默认 2000 保持历史行为），机械描述在同名 `.meta`。
- 独立审计：`sh tools/ci/audit-panvk-bundle.sh` → 从上游 `.gz` 复算出补丁后的字节，
  断言等于 `f73600da…`（即安装器产出的就是本文件验证过的那颗）。

## 3 · A/B（同一会话，唯一变量是 ICD 的 `library_path`）

```
WARMUP ON  (upstream 2s) vulkan-probe run1: 4075 ms
WARMUP ON  (upstream 2s) vulkan-probe run2: 4061 ms
WARMUP OFF (patched)     vulkan-probe run1:   68 ms
WARMUP OFF (patched)     vulkan-probe run2:   75 ms
WARMUP ON  glxinfo -B    run1: 4306 ms
WARMUP ON  glxinfo -B    run2: 4273 ms
WARMUP OFF glxinfo -B    run1:  289 ms
WARMUP OFF glxinfo -B    run2:  271 ms
```

canonical ICD（`husk_panvk_guest.json`）下另测 3 次 `glxinfo -B`：279 / 233 / 318 ms，
输出 `direct rendering: Yes`、`OpenGL renderer string: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`、
`OpenGL version string: 2.1 Mesa 27.0.0-devel (git-b84f02a451)`。

## 4 · 没变坏的证据

```text
$ DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs \
  LD=files/linux/husk/libld.so sh tools/device-tests/19-readback-suite.sh
PASS=19 FAIL=0
READBACK_AUDIT PASS=6 FAIL=0
STATE renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 27.0.0-devel (git-b84f02a451)
```

19 项覆盖：宽度边界 1/2/3/4/5/7/8/15/16/17/63/64/65 逐行精确读回、金丝雀尾部不被越界写、
整块读回、截图行距（240×135 = 129600 字节逐字节相同）。

冷启动（`/data/local/tmp/meas3.sh`，点按钮 → `SurfaceView`）：

| 轮次 | tap→gnome-shell | tap→Surface | 补丁前（evidence 353） |
|---|---:|---:|---:|
| 1 | 9.93 s | 13.88 s | 14.52–15.37 s |
| 2 | 6.86 s | 10.08 s | — |
| 3 | 6.97 s | 10.55 s | — |
| 中位 | 6.97 s | **10.55 s** | 14.7 s |

### 4.1 · 两颗驱动的 19 项读回对拍（本轮补做）

把上游驱动（从 `vendor/.../libvulkan_panfrost_frist370_execva.so.gz` 解出，sha256 `59914856…`）
放到树上 `/tmp`，用两份只差 `library_path` 的 ICD，同一段探针各跑一遍 19 项读回：

```
[on ] rc=0  PASS=19 FAIL=0
[off] rc=0  PASS=19 FAIL=0
== stdout diff (on vs off) ==
STDOUT IDENTICAL
```

这比「两张 PPM 的 md5 相同」更强：判据是每一项都等于期望值（宽度边界 1/2/3/4/5/7/8/15/16/17/
63/64/65 的逐行精确读回、金丝雀尾部不被越界写、整块读回、240×135 截图行距逐字节相同），
而且两颗驱动的完整 stdout 逐字节相同。复现：

```sh
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs LD=files/linux/husk/libld.so \
  sh tools/device-tests/run-guest.sh t-357-bytes.sh
```

> 早前轮次还做过「整屏抓帧 md5 逐字节相同（`4ff819c0…`）」；那一条**本轮没有复现**，
> 引用时按「早前轮次实测」标注。上面 4.1 是同一主张的更强版本。

## 5 · 风险边界

1. **只验证了 Mali-G77 MC9 + Android 13 + kernel 4.19.191 + 4 KiB 页这一台。** 其它 kbase
   设备（尤其注释点名的 Mali-G57）可能需要这段热身，去掉后表现为**第一个 GPU job 被终止**。
   那种机器上用 `HUSK_PANVK_KEEP_WARMUP=1`，或按源码补丁把 `PANVK_KBASE_WARMUP_MS` 调回 2000。
2. **这是二进制补丁，不是新驱动版本**：`glxinfo` 自报的 `Mesa 27.0.0-devel (git-b84f02a451)`
   仍是源码线信息，不因补丁变化；判断「装的是不是这颗」只能靠 sha256。
3. 偏移 `0x53a570` 与两段 sha256 绑定了这颗具体构建。上游一旦重编，安装器会先报上游 SHA
   不匹配并退出（exit 76），不会盲改。
4. 补丁不改变渲染语义：10 个字节之外的 `.so` 内容与上游逐字节相同（安装器按 head/tail 拼接）。

## 6 · 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.myth.husk \
ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs LD=files/linux/husk/libld.so \
  sh tools/device-tests/run-guest.sh t-357-ab.sh      # 同会话 A/B 耗时
sh tools/ci/audit-panvk-bundle.sh                      # 从 .gz 复算补丁后 SHA
adb -s UKHMMB8PYTGECMNV shell \
  'sh /data/local/tmp/meas3.sh com.linuxhub.myth.husk com.linuxhub.myth.husk/com.java.myapplication.MainActivity'
```

## 7 · 顺带发现（另立缺陷，不在本补丁范围）

设备 `usr/share/vulkan/icd.d/` 里还留着实验期的
`husk_panvk_nosleep_test.json`（指向 `libvulkan_panfrost_husk_nosleep.so`，sha256 与 canonical
驱动相同），`usr/lib/aarch64-linux-gnu/` 里还留着
`libvulkan_panfrost_x11.so`、`libvulkan_panfrost_frist370_execva.so`、`libvulkan_panfrost.so`
和 `.libvulkan_panfrost_husk.so.bak-orig`。这些是「过期 ICD/驱动副本」，0.0.37 的清理项要求
删掉并加负向测试（记 defect H43）。
