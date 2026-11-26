# 299 · 第 5 轮：读回测试套（宽度边界 / 金丝雀 / 截图路径）

设备：`UKHMMB8PYTGECMNV`（RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191），
树：`files/linux/distributions/ubuntu-24.04/rootfs`（862 包），loader `files/linux/husk/libld.so`。
探针：`tools/device-tests/probes/readback-suite.py`（纯 ctypes，`libEGL.so.1` / `libGLESv2.so.2`）。

## 0. 结果

```
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
PASS=17 FAIL=0
```

- **T1** 宽度边界 13 组（1/2/3/4/5/7/8/15/16/17/63/64/65）：逐行读**逐字节等于**逐像素真值，
  `glGetError()=0`。含非 4 倍数宽度，覆盖 `GL_PACK_ALIGNMENT=4` 会出问题的那些。
- **T2** 金丝雀：缓冲区尾部多留 4096 字节 `0xAA`，逐行读后尾部**一个字节都没动**。
- **T3** 负向测试：整块读仍然坏（`16×16` 错 907/1024 字节、`65×4` 错 951/1040 字节，
  越界 0 字节，`glGetError()=0`）。这份测试**特意要求它坏**：哪天它变好了，
  脚本会打 `INFO ... whole-block read is CORRECT now`，提示可以退役逐行方案。
- **T4** 截图路径（`240×135` 行循环写进 `W*H*4`）：129 600 字节**逐字节相同**。

## 1. 一个必须先踩掉的坑：sticky error 让 13 条全假红

第一版把 `glGetError()` 直接当断言，结果：

```
FAIL  W=1 H=3: 0 byte(s) differ, glGetError=0x501     ← 数据全对，但 GL_INVALID_VALUE 是粘的
```

`0x501` 是**建帧阶段**（texImage2D/FBO 那几步）就置上的，与读回无关；GL 错误是粘性的，
于是后面每次读都被判红。修法：建帧后先 `drain()` 把历史错误抽干并单独 `INFO` 打出来，
然后只对**读本身**要求 `glGetError()=0`。

剩下的疑问照实记：**建帧为什么会置 `0x501`，本轮没定位**（UNKNOWN）。它不影响读回结果，
但它是个真信号，第 6 轮 GPU 原生化时应该查一次。

## 2. 这条测试防的是什么

evidence 289 的结论是"整块读坏、逐行读好"，但它只测了一个 `16×16`。
如果哪天有人（包括我们自己）在截图、比对、录制里写回整块读，或者把行距算错一次，
只看"画面出得来"是发现不了的 —— 会得到一张**斜着错位**的图，而且 `glGetError()` 还是 0。
所以 T1/T2/T4 是**正**断言（必须对），T3 是**负**断言（必须错，直到它真的被修好）。

## 3. 复现

```sh
adb push tools/device-tests/probes/readback-suite.py /data/local/tmp/
adb shell 'run-as com.linuxhub.husk cp /data/local/tmp/readback-suite.py \
  files/linux/distributions/ubuntu-24.04/rootfs/tmp/'
adb shell 'run-as com.linuxhub.husk sh files/linux/husk-exec2.sh \
  files/linux/distributions/ubuntu-24.04/rootfs files/linux/husk/libld.so \
  usr/bin/bash -lc "python3 /tmp/readback-suite.py"'
```

> `husk-exec2.sh` 是临时启动器（`NR_ROOTFS` + `--library-path` + `cd rootfs`），
> 等价于 `tools/device-tests/14-gate-a-acceptance.sh` 里那份；不进发行包。

## 4. 边界

- 只测了 surfaceless FBO（无窗口）。默认帧缓冲 / X11 窗口表面的整块读仍未测（UNVERIFIED，
  与 289 §边界一致）。
- 只在 Mali-G77 / Android 13 / 4 KiB 页这台机器上跑过；Adreno、16 KiB 页没测（UNKNOWN）。
- `240×135` 只是"接近截图尺寸"的代表值，不是 2400×1080 的实机截图路径；后者在
  evidence 290 里按 24.8 ms/兆像素计过代价。
