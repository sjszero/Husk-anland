# 319 · G2：三进程并发按 C4 修正后的 19 项读回套复验（2026-11-27）

设备：`UKHMMB8PYTGECMNV` / RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 /
kernel 4.19.191 / 4 KiB 页。

## 0 · 结论

- **FACT**：同一棵正式 rootfs 上同时启动 3 个 `readback-suite.py`，三个进程都创建了
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` 上下文，OpenGL ES 版本为 3.2。
- **FACT**：每个进程都按 C4 修正后的完整套跑完 19 项断言，结果均为
  `PASS=19 FAIL=0`。
- **FACT**：三份原始日志的测试段落逐项一致，宽度边界、金丝雀、整块读和截图路径均通过。
- **UNVERIFIED**：本轮验证的是三个同树进程并发，不是“整桌面 + 多个应用同时持续渲染”
  的长时间稳定性；后者仍需在独立 X11 桌面会话中验收。
- **UNKNOWN**：本机只有 Mali-G77，Adreno/KGSL 的并发行为无法用本证据推断。

## 1 · 测试对象

```text
default entry:
files/linux/distributions/ubuntu-24.04/rootfs
-> ../../../release-tree-libc812-20261127/root

renderer:
zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))

OpenGL ES:
3.2 Mesa 25.2.8-0ubuntu0.24.04.5
```

测试期间三个进程使用同一发行树、同一 PanVK ICD 和同一 loader 注入的 GPU 默认值；没有切换
Wayland，也没有改动 X11 合成器。

## 2 · 并发结果（原文）

三个进程同时运行，分别记录到以下结果：

```text
# process 1
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
PASS=19 FAIL=0

# process 2
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
PASS=19 FAIL=0

# process 3
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) version=OpenGL ES 3.2 Mesa 25.2.8-0ubuntu0.24.04.5
PASS=19 FAIL=0
```

套件分组：

```text
T1 row-wise readback, width boundaries     13 PASS
T2 canary: row-wise read must not touch tail 3 PASS
T3 whole-block read: exact bytes + canary   2 PASS
T4 screenshot path: 240x135                 1 PASS
                                               ------
                                               19 PASS
```

原始日志保存在主机：

```text
/tmp/husk-rb-1.log
/tmp/husk-rb-2.log
/tmp/husk-rb-3.log
```

## 3 · 与旧证据的区别

evidence 300 的三进程并发使用的是旧读回套：

```text
PASS=17 FAIL=0
```

evidence 314 修正了 PixelStore 常量，并把整块读从错误负向断言改成精确正向断言，因此同一套
变为 19 项。本证据补上了当时明确留下的缺口：在正式 807 包树上按修正后的 19 项套重新做三进程
并发，而不是拿旧的 17 项结果替代。

## 4 · 边界

- 本轮没有重新验证 Adreno/KGSL：**UNKNOWN**。
- 本轮没有重新验证 16 KiB / 64 KiB 真机：**UNVERIFIED**。
- 本轮没有启动独立 Husk X11 桌面，也没有做整桌面多应用长稳：**UNVERIFIED**。
- `KBASE_IOCTL_MEM_EXEC_INIT` 仍打印 `Operation not permitted`；这没有阻止本套 GLES 读回，
  但不能据此推导所有需要 executable BO 的 Vulkan/图形工作负载都已通过。
- 本证据不修改 `VERSION`、已发布 `v0.0.36`、tag 或 Release。
