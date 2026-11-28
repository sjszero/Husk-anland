# 351 · 装进测试宿主后的 GPU 实证（0.0.37 第一轮补充）

日期：2026-11-27（CST）
设备：RMX3357 / Mali-G77 MC9 / Android 13 / 4.19.191
宿主：`com.linuxhub.myth.husk`；Husk `0.0.36`、提交 `c924d620f6de`、`mode=upgrade`

## 0 · 结果

| # | 断言 | 结果 | 等级 |
|---|---|---|---|
| 1 | 默认进程拿到 zink/Mali | `renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` | FACT |
| 2 | 继承一个**坏** `VK_ICD_FILENAMES` 仍拿到 GPU，不静默回退 llvmpipe | 仍 `zink Mali` | FACT |
| 3 | `HUSK_GPU=llvmpipe` 是显式回退 | `renderer=llvmpipe (LLVM 20.1.2, 128 bits)` | FACT |
| 4 | `NR_GPU_DEFAULT_OFF=0` 不被误判成关闭 | 仍 `zink Mali` | FACT |
| 5 | `/proc/<pid>/maps` 加载的是 husk 驱动 | `libvulkan_panfrost_husk.so` | FACT |
| 6 | `/proc/<pid>/fd` 有 `/dev/mali0` | `mali0_fds=1` | FACT |
| 7 | 进程环境是 canonical 三件套 | `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json`、`GALLIUM_DRIVER=zink`、`PANVK_DXVK_COMPAT=0` | FACT |

复现（guest 内）：

```sh
/tmp/gl-probe readback 16x16                        # 1
VK_ICD_FILENAMES=/nonexistent.json /tmp/gl-probe readback 16x16   # 2
HUSK_GPU=llvmpipe /tmp/gl-probe readback 16x16      # 3
NR_GPU_DEFAULT_OFF=0 /tmp/gl-probe readback 16x16   # 4
# 5/6/7：后台起 gl-probe 后读 /proc/<pid>/{maps,fd,environ}
```

第 2 条是关键：**坏值被我们的默认值纠正**，而不是悄悄落到 llvmpipe。它证明"禁止旧 ICD 被选中"
在 loader 这一层是硬的，不是靠脚本运气。

## 1 · 宿主侧仍有一处钉死旧 ICD

`rootfs/usr/local/bin/doro-gpu-run:92`（宿主自带脚本）：

```text
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
```

该旧 ICD 指向 `libvulkan_panfrost_x11.so`。当前该文件与 `libvulkan_panfrost_husk.so`
**字节相同**（都是 `599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c`），
所以现在不致命；但它意味着"会话里 ICD 是 canonical"这句话在宿主启动路径上不成立，
一旦驱动更新，旧副本会继续赢。**记为外部阻塞 H40**：宿主必须删掉这行，或改指 canonical。

## 2 · 仍未验证

- 桌面会话内（宿主 exec 的那条链）ICD 到底用哪个：桌面被 H38 卡死，无法取 /proc/maps。
- 三进程并发读回、19 项完整套件之外的 4 路长稳已在本轮跑过（见 evidence 350），
  但那是通过 `make readback-audit`/`make gpu-soak` 走的，不是桌面会话内。
