# Evidence 42 — kbase 能给出 panfrost 需要的全部硬件参数（GET_PARAM 生死线通过）

设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191
时间：2026-11-23 01:4x（CST）
工具：`runtime/crates/drm-probe`（新建，纯 Rust）

> 目的：Mesa panfrost 起来第一件事就是 `GET_PARAM` 问硬件参数，答错就退回 swrast。
> 本轮验证：这些参数能不能从 kbase 拿到，还是必须我们自己合成。

## 1. 一处纠错（FACT，直接影响能不能问）

`KBASE_IOCTL_GET_GPUPROPS` 是 **`_IOW` 不是 `_IOWR`**：

```
0xc0108003（我按 _IOWR 编的） → errno=25 ENOTTY
0x40108003（GPL 头 + 厂商 libGLES_mali.so 里的） → PASS, returned 723 bytes
```

## 2. 返回值的格式（FACT，源码 `kbase_gpuprops_populate_user_buffer`）

不是「(id, value) 对」，是**变长记录流**：

```
每条 = u32 header + 值（小端）
header = (prop_id << 2) | size_code
size_code: 0=u8, 1=u16, 2=u32, 3=u64
```

723 字节 = 全部 100+ 个属性；按上面的规则解出来，序号和 `KBASE_GPUPROP_*` 完全对上。

## 3. 覆盖度：24/24 主表 + 16/16 JS_FEATURES（FACT）

`drm|coverage|PASS|24/24 主表 + 16/16 JS_FEATURES；缺失: -`

panfrost 要的 `DRM_PANFROST_PARAM_*` 逐条对应 kbase 的 `RAW_*`：

| panfrost 要的 | kbase prop | 实测值 |
|---|---|---|
| GPU_PROD_ID | PRODUCT_ID | `0x9080`（G77 的 product id） |
| GPU_REVISION | MAJOR/MINOR_REVISION | `major=0 minor=1` → `0x1`（组装方式 INFERENCE） |
| SHADER_PRESENT | RAW_SHADER_PRESENT | `0x660067`（9 核的位图） |
| TILER_PRESENT / L2_PRESENT / STACK_PRESENT | RAW_*_PRESENT | `0x1` / `0x1` / `0x67` |
| AS_PRESENT | RAW_AS_PRESENT | `0xff` |
| JS_PRESENT | RAW_JS_PRESENT | `0x7`（3 个 job slot，与 `kbase_js_get_slot` 的 0/1/2 一致） |
| L2_FEATURES / CORE_FEATURES / TILER_FEATURES | RAW_*_FEATURES | `0x7120206` / `0x0` / `0x809` |
| MEM_FEATURES / MMU_FEATURES | RAW_*_FEATURES | `0x301` / `0x2830` |
| THREAD_FEATURES | RAW_THREAD_FEATURES | `0x48000` |
| MAX_THREADS / WG / BARRIER | RAW_THREAD_MAX_* | `1024` / `512` / `512` |
| COHERENCY_FEATURES | RAW_COHERENCY_MODE | `0x1f` |
| TEXTURE_FEATURES0..3 | RAW_TEXTURE_FEATURES_0..3 | `0xf7fffffe` / `0xc3fff7ff` / `0xbfe1ff9f` / `0x10f6` |
| JS_FEATURES0..15 | RAW_JS_FEATURES_0..15 | `0x20e` / `0x1c9e` / `0x1e` / 其余 0 |
| NR_CORE_GROUPS | COHERENCY_NUM_CORE_GROUPS | `1` |
| THREAD_TLS_ALLOC | RAW_THREAD_TLS_ALLOC | `0x0` |

数值自洽：3 个 job slot、单 core group、Valhall 级纹理/线程特性 → 这就是 G77。

## 4. 结论

**FACT**：panfrost 需要的硬件参数，kbase 一个不少地给得出来，不需要我们伪造寄存器值。
**INFERENCE**：panfrost probe 阶段不会因为参数缺失而 fallback 到 swrast；M2 的最大风险消掉了。

剩下的风险前移到 M1（假节点 + BO 的 DRM 语义）和 M3（提交后能不能出 START）。

## 5. 复现命令

```sh
cd /Users/l/Documents/Codex/2026-08-10/husk/runtime
# 交叉编译（同 evidence 41 的 RUSTFLAGS）
cargo build --release --target aarch64-unknown-linux-gnu -p drm-probe
adb push target/aarch64-unknown-linux-gnu/release/drm-probe /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/drm-probe files/rootfs/usr/bin/'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/drm-probe param'
adb shell 'run-as com.linuxhub.husk sh files/gexec.sh usr/bin/drm-probe props'   # 原始记录流
```
