# 设计文档索引

产品介绍见 [../../README.md](../../README.md)，当前进度见 [../STATUS.md](../STATUS.md)，
实测数字见 [../RESULTS.md](../RESULTS.md)。上机操作手册见 [../ENGINEERING.md](../ENGINEERING.md)。

| 文件 | 内容 |
|---|---|
| [runtime.md](runtime.md) | 运行时架构：分层、预算、验收项、剩余工作 |
| [capability-baseline.md](capability-baseline.md) | 目标机权限边界与执行能力基线 |
| [device-interface-choices.md](device-interface-choices.md) | 每类硬件的 guest 接口形态与当前状态 |
| [husk-backend-interface.md](husk-backend-interface.md) | 统一 GPU / 显示 / 视频 / 音频 / 相机 / 输入后端接口 |
| [gpu-drm-shim.md](gpu-drm-shim.md) | GPU：DRM 语义层（后备方案）与当前 PanVK 路线 |
| [acquire-fence.md](acquire-fence.md) | 显示：producer / acquire fence 链路与 fallback |
| [video-bridge.md](video-bridge.md) | 视频：MediaCodec ↔ guest 的 AHB 交接协议 |
| [audio-bridge.md](audio-bridge.md) | 音频：guest ↔ app AAudio 的桥 |
| [libc-interpose.md](libc-interpose.md) | libc 内部调用（非 PLT）的槽位方案 |
| [compat-layer.md](compat-layer.md) | 兼容层机制：三层拦截、覆盖账（96/111）、代价与未闭合项 |
| [lifecycle.md](lifecycle.md) | 壳应用生命周期与进程回收 |
| [page-size.md](page-size.md) | 页大小无关的设计与体检 |
| [dlopen-path-coverage.md](dlopen-path-coverage.md) | dlopen 进来的对象如何走 guest 路径 |
| [rootfs-packages.md](rootfs-packages.md) | 2.0 rootfs 需要补的包与复现步骤 |
| [evidence/](evidence/) | 设备原始输出，编号被其他文档引用 |
| [patches/](patches/) | 构建期补丁 |

## 写作规范

- 结论标 FACT / INFERENCE / UNKNOWN，未实测不写 PASS。
- 每个数字都要能指回 `evidence/` 里的设备输出。
- 与 [runtime.md](runtime.md) 冲突的描述一律作废，先改那份。
- **只写我们自己的设计与实测**：不收录第三方仓库的源码分析、清单或对照表。
