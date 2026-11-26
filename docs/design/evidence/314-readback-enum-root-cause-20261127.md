# 314 · 整块 `glReadPixels` 的真正根因：PixelStore 常量写反（2026-11-27）

设备：`UKHMMB8PYTGECMNV`（RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页）。
运行树：`files/rootfs-full`。
loader：`files/husk-dist-final2/libld.so`。

## 0 · 结论（FACT）

evidence 162、179、289 里的“整块 `glReadPixels` 行距恒 16 字节 / Mesa 打包层缺陷”结论**不成立**。
真正原因是测试探针把两个 OpenGL ES PixelStore 枚举值写反了：

```text
GL_PACK_ALIGNMENT   = 0x0D05
GL_PACK_ROW_LENGTH  = 0x0D02
```

旧探针写成了：

```text
GL_PACK_ALIGNMENT   = 0x0D02  // 实际是 PACK_ROW_LENGTH
GL_PACK_ROW_LENGTH  = 0x0D05  // 实际是 PACK_ALIGNMENT
```

于是 `glPixelStorei(GL_PACK_ROW_LENGTH, 0)` 实际上没有把 RowLength 归零，默认 RowLength 留成了 4；
`16 × 4 bytes = 64 bytes`，而错误的 RowLength=4 让 Mesa 按 `4 × 4 = 16 bytes` 推进目的行。
这不是 Husk、path-shim、Mali/PanVK、llvmpipe 或 Mesa 的运行时缺陷，而是验证代码的 ABI/常量错误。

影响文件已修正：

- `tools/device-tests/probes/readback.py`
- `tools/device-tests/probes/readback-suite.py`
- `runtime/crates/gl-probe/src/main.rs`

## 1 · 修前负向结果（FACT）

旧版测试树：

```sh
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c \
   "sh files/rootfs-full/tmp/husk-exec.sh files/rootfs-full \
     /data/data/com.linuxhub.husk/files/husk-dist-final2/libld.so \
     usr/bin/python3 /tmp/readback.py 16 16 current compute"'
```

关键状态和整块读：

```text
STATE default            PACK_ALIGNMENT=4 PACK_ROW_LENGTH=4
STATE rowlen=32          PACK_ALIGNMENT=4 PACK_ROW_LENGTH=4
STATE align=8            PACK_ALIGNMENT=8 PACK_ROW_LENGTH=4
BLOCK default              written=76/256 correct=4 err=0x0 dup=0 overrun=0
dest_pitch_bytes=16 (=4 px)  row0_px=4  rows_present=16  rows=0..15
```

`PACK_ROW_LENGTH` 设置为 0/16/32 都不改变状态，证明测试读的不是它想读的枚举。旧读回套仍会打印：

```text
T3 whole-block read: must not look correct (negative test)
INFO W=16 H=16: whole-block read wrong in 907/1024 bytes ...
INFO W=65 H=4: whole-block read wrong in 951/1040 bytes ...
PASS=17 FAIL=0
```

这个 `PASS=17` 不是整块读通过：旧脚本故意把 T3 失败当作“期望结果”，没有给 T3 正向断言。

## 2 · 修后正向结果（FACT）

修正常量后，同一设备、同一 guest 树、同一 loader：

```text
TAG corrected  GL_RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
GL_VERSION=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
SOURCE 1x1-read map: 0/256 pixels differ from uploaded pattern
STATE default            PACK_ALIGNMENT=4 PACK_ROW_LENGTH=0
STATE rowlen=32          PACK_ALIGNMENT=4 PACK_ROW_LENGTH=32
STATE align=8            PACK_ALIGNMENT=8 PACK_ROW_LENGTH=0
BLOCK default              written=256/256 correct=256 err=0x0 dup=0 overrun=0
dest_pitch_bytes=64 (=16 px)  row0_px=16  rows_present=16  rows=0..15
BLOCK PBO                  written=256/256 correct=256 err=0x0 dest_pitch_bytes=64
ROW-AT-A-TIME 256/256 correct
```

完整读回套现在把 T3 改成“整块读必须正确 + 尾部金丝雀必须不变 + `glGetError()=0`”的正向断言：

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
T1 宽度边界 13 组: PASS
T2 行式读取 3 组: PASS
T3 整块读 2 组: PASS
T4 240×135 截图路径: PASS
PASS=19 FAIL=0
```

## 3 · Rust 探针交叉验证（FACT）

修正后的 `gl-probe` 也重新交叉编译并在同一设备运行：

```text
renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
=== A 整块 16x16（256 像素，预填 0xAA）
  被写 256/256（期望 256），值正确 256/256
=== PACK_ROW_LENGTH=W：被写 256/256，值正确 256/256
=== PACK_ALIGNMENT=1：被写 256/256，值正确 256/256
=== B 逐行读（16x1 × 16 次）：被写 256/256，值正确 256/256
=== C 逐列读（1x16 × 16 次）：被写 256/256，值正确 256/256，越界写到缓冲后段 0/16 次
=== D 单点 1x1 采样：5/5 正确
```

Python/ctypes 探针和 Rust/dlopen 探针现在给出同一结论；两条路径都不再显示 16 字节行距。

## 4 · 复现命令

```sh
# 主机侧修正后的探针
sed -n '29,36p' tools/device-tests/probes/readback.py
sed -n '31,38p' tools/device-tests/probes/readback-suite.py
sed -n '113,119p' runtime/crates/gl-probe/src/main.rs

# 设备侧：推入探针后运行
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c \
   "sh files/rootfs-full/tmp/husk-exec.sh files/rootfs-full \
     /data/data/com.linuxhub.husk/files/husk-dist-final2/libld.so \
     usr/bin/python3 /tmp/readback-suite.py"'
```

预期最后一行：`PASS=19 FAIL=0`。如果人为把常量换回旧值，状态应出现
`PACK_ROW_LENGTH=4`，T3 应因整块读错误而失败。

## 5 · 边界

- 本次验证的是 surfaceless FBO 和截图形状的行循环；默认帧缓冲 / X11 窗口表面整块读仍未单独验证。
- 本次只验证 Mali-G77 / Android 13 / 4 KiB 页；Adreno、16/64 KiB 页仍为 UNKNOWN。
- “整块读正确”不等于“窗口内容到 X server 零拷贝”；DRI3 的宿主侧限制仍是独立问题。

## 6 · 提交前复跑（FACT）

2026-11-27 提交前，在同一台 RMX3357 上重新推入修正后的探针并复跑：

```text
T1 宽度边界 13 组: PASS
T2 行式读取 3 组: PASS
T3 整块读 2 组: PASS
T4 240×135 截图路径: PASS
PASS=19 FAIL=0
```

诊断探针同时报告：

```text
STATE default  PACK_ALIGNMENT=4 PACK_ROW_LENGTH=0
BLOCK default  written=256/256 correct=256 err=0x0 overrun=0
BLOCK PBO      written=256/256 correct=256 err=0x0
ROW-AT-A-TIME  256/256 correct
```
