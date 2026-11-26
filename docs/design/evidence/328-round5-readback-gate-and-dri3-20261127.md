# 328 · 第 5 轮收口：19 项读回门禁、DRI3 边界与呈现代价（2026-11-27）

设备：`UKHMMB8PYTGECMNV` / RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / arm64 / 4 KiB 页。应用：`com.linuxhub.husk`。

## 0 · 结论

- **FACT**：当前正式 rootfs 的完整 19 项 Rust `gl-probe readback-suite` 已在真机通过：
  `PASS=19 FAIL=0`，renderer 为
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`。
- **FACT**：该套件现在由 `tools/device-tests/19-readback-suite.sh` 执行，并同时接入
  `make readback-audit` 与 `make gpu-audit`。先前手工运行、未进门禁的缺口已关闭。
- **FACT**：loader 默认 GPU 注入复验为 `GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0`；
  plain 进程为 zink/PanVK，两个显式关闭开关都落到 llvmpipe。
- **FACT**：不经过 shim 的应用视角仍是 `/dev/dri` 只有 `card0`、没有 `renderD128`，
  `card0` 为 `226:0 / 0660 system:system` 且读取返回 `Permission denied`。
- **INFERENCE**：在“X server 跑在应用内、Mali 走 kbase、没有可用 render node”的当前布局下，
  DRI3 结构性不可达；这不是 path-shim 增加一个 hook 能补齐的问题。
- **UNVERIFIED**：本轮没有重新测量 2400×1080 的 30.27 ms 回读与 18.92 ms `XPutImage`；
  这两个数字沿用同机 evidence 290，不能写成今天重新测得的数值。
- **UNKNOWN / UNVERIFIED**：Adreno/KGSL、16/64 KiB 页真机、独立整桌面长稳不在本轮证据范围。

## 1 · 修前缺口（FACT）

基线的正式门禁没有跑完整读回套：

```text
$ git show HEAD:Makefile | grep -n 'readback-suite'
<no output>

$ git show HEAD:tools/device-tests/18-gpu-loader-defaults.sh | grep -n 'gl-probe.*readback'
36: ... ./root/gl-probe readback 16x16 ...

$ git cat-file -e HEAD:tools/device-tests/probes/readback-suite.py
absent
```

因此当时能证明“16×16 单点读回可用”和“renderer 是 zink”，但不能由门禁本身证明宽度边界、
金丝雀、整块读和截图行式路径仍然正确。旧 evidence 319 虽有完整套结果，但它是手工运行，
不能防止后续回归。

## 2 · 已修正内容（FACT）

新增 `tools/device-tests/19-readback-suite.sh`：

- 预检 guest bash、loader 与 host `gl-probe`；
- 将 `husk-exec.sh` 和 Rust `gl-probe` 推入当前 rootfs；
- 经真实 Husk loader 执行 `gl-probe readback-suite`；
- 同时要求进程退出码为 0、renderer 含 `zink`、最终必须为 `PASS=19 FAIL=0`；
- 可选 `LOG=` 保留原始日志。

新增 `make readback-audit`，并让 `make gpu-audit` 先跑同一门禁，再跑桌面 GPU 继承审计。

## 3 · 负向测试（FACT）

把期望断言故意改成 18 项，完整套仍实际输出 19 项，门禁必须以非零退出：

```sh
DEV=UKHMMB8PYTGECMNV EXPECT_PASS=18 make readback-audit
```

实际结果：

```text
STATE summary: PASS=19 FAIL=0 expected PASS=18 FAIL=0
FAIL  readback suite counts do not match the release contract
READBACK_AUDIT PASS=5 FAIL=1
make: *** [readback-audit] Error 1
NEGATIVE_RC=2
```

这证明脚本不是只打印 probe 输出，而是会按发布契约拒绝不合规结果。

## 4 · 正向测试（FACT）

命令：

```sh
DEV=UKHMMB8PYTGECMNV LOG=/tmp/husk-readback-audit-20261127.log make readback-audit
```

完整套原文：

```text
MESA: warning: kbase: KBASE_IOCTL_MEM_EXEC_INIT failed: Operation not permitted (executable BO allocation will not work)
WARNING: panvk is not a conformant Vulkan implementation, testing use only.
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
== T1 row-wise readback, width boundaries ==
  PASS  W=1 H=3: 12 bytes exact, glGetError=0
  PASS  W=2 H=3: 24 bytes exact, glGetError=0
  PASS  W=3 H=3: 36 bytes exact, glGetError=0
  PASS  W=4 H=3: 48 bytes exact, glGetError=0
  PASS  W=5 H=3: 60 bytes exact, glGetError=0
  PASS  W=7 H=3: 84 bytes exact, glGetError=0
  PASS  W=8 H=4: 128 bytes exact, glGetError=0
  PASS  W=15 H=3: 180 bytes exact, glGetError=0
  PASS  W=16 H=4: 256 bytes exact, glGetError=0
  PASS  W=17 H=3: 204 bytes exact, glGetError=0
  PASS  W=63 H=2: 504 bytes exact, glGetError=0
  PASS  W=64 H=2: 512 bytes exact, glGetError=0
  PASS  W=65 H=2: 520 bytes exact, glGetError=0
== T2 canary: row-wise read must not touch the tail ==
  PASS  W=5 H=3: head exact, 4096-byte tail untouched
  PASS  W=17 H=3: head exact, 4096-byte tail untouched
  PASS  W=65 H=2: head exact, 4096-byte tail untouched
== T3 whole-block read: exact bytes and canary ==
  PASS  W=16 H=16: whole-block exact, 8192-byte tail untouched, glGetError=0
  PASS  W=65 H=4: whole-block exact, 8192-byte tail untouched, glGetError=0
== T4 screenshot path: row loop into W*H*4 ==
  PASS  240x135: 129600 bytes byte-identical to ground truth

PASS=19 FAIL=0
  PASS  probe exited 0
  STATE renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
  PASS  renderer contains zink
  STATE summary: PASS=19 FAIL=0 expected PASS=19 FAIL=0
  PASS  complete readback suite passed
READBACK_AUDIT PASS=6 FAIL=0
```

原始日志 SHA-256：

```text
a789260ac1845df5492b1a4481ed4d14aba16994ae1cde312e8ccac9dd8a3f5d  /tmp/husk-readback-audit-20261127.log
```

host `gl-probe` SHA-256：

```text
2b85e557829178174ac028e0eb0bb3dd51e4fd61b3fa9ed7a0fa3f2d3df3f31f  runtime/target/aarch64-unknown-linux-gnu/release/gl-probe
```

## 5 · GPU 默认值复验（FACT）

```text
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1: llvmpipe (LLVM 20.1.2, 128 bits)
HUSK_GPU=llvmpipe: llvmpipe (LLVM 20.1.2, 128 bits)
```

## 6 · DRI3 当前视角复核（FACT）

不经过 shim，以应用 uid 观察：

```text
total 0
crw-rw---- 1 1000 1000 226, 0 card0
/dev/dri/renderD128: No such file or directory
/dev/dri/card0: 226:0 660 system:system
dd: /dev/dri/card0: Permission denied
```

这与 evidence 290 的判定链一致：server 在应用进程里，拿不到真 DRM fd；Mali 的 kbase 节点
也不是 DRM/PRIME 设备。guest 里能看到的假 `renderD128` 不能当作宿主 render node。

## 7 · 呈现代价（引用同机既有实测，本轮未重测）

evidence 290 在同一机型上测得：

```text
2400x1080 zink GPU -> CPU 回读       30.27 ms/frame
2400x1080 XPutImage 协议拷贝         18.92 ms/frame
合计                                 约 49.2 ms/frame，约 19.9 FPS
2400x1080 llvmpipe 整窗路径          约 20.1 ms/frame，约 49.7 FPS
归一化 zink 回读                     24.8 ms/MP
```

这些数字是既有 FACT；本轮只复核了设备节点可见性，没有重跑计时，因此不把它们冒充为今天的
新测量。结论不变：GPU 渲染本身不是当前全屏 X11 路径的唯一瓶颈，窗口内容回读和 `XPutImage`
拷贝仍需按 evidence 290 的方案处理。

## 8 · 旧“16 字节行距”结论的处理（FACT）

evidence 314 已证明旧结论是探针把 `GL_PACK_ALIGNMENT` 与 `GL_PACK_ROW_LENGTH` 常量写反，
不是 Mesa、PanVK 或 Husk 的运行时缺陷。本轮没有修改 Mesa 公共读回层；本轮修的是验证层缺口，
并把正确的 Rust 全套提升为正式门禁。不得重新宣传“Mesa 行距恒 16 字节”。

## 9 · 边界

- 只验证 Mali-G77 / Android 13 / 4 KiB 页；Adreno、16/64 KiB 真机仍为 **UNKNOWN**。
- `KBASE_IOCTL_MEM_EXEC_INIT` 仍打印 `Operation not permitted`；本次 GLES 读回通过不能推导
  所有 executable BO 工作负载均通过。
- 本轮不是整桌面多应用长稳；后者属于第 6 轮。
- 不修改 `VERSION`，不创建 tag/Release，不把第 5 轮证据当成发行完成。
