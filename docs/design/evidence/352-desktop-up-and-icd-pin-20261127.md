# 352 · 桌面真的起来了：旧 ICD 钉死路径的实测边界

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
宿主：`com.linuxhub.myth.husk`（黑盒运行容器，未改其源码）
运行时：工作区 `c924d62` + 本轮未提交改动，`loader_sha256=67f543b9…`

## 0 · 结论

| 项 | 结果 | 判定 |
|---|---|---|
| 桌面连续 3 次起来（19:24 / 19:27 / 19:29） | 是，全部无 SHM shim | FACT |
| 清掉设备上的 SHM shim 残留后重启 | 桌面照常起来（第 2、3 次） | FACT |
| 删掉宿主钉死的旧 ICD 清单 | `GNOME 已退出，退出码 139` | FACT |
| 旧清单改指 canonical 驱动 | 会话 `maps` 出现 `libvulkan_panfrost_husk.so`，renderer 仍 zink/Mali | FACT |
| GPU 读回 19 项 | `PASS=19 FAIL=0` | FACT |
| GPU 全局继承（桌面 + nested GTK） | `GPU_GLOBAL_AUDIT PASS=3 FAIL=0` | FACT |
| H38（MIT-SHM → 退出码 133） | 本轮未复现，触发条件未知 | UNVERIFIED |

## 1 · H38 未复现

evidence 350 §4 记录的失败是 `request_code 130 (MIT-SHM) minor_code 1` → 退出码 133。
本轮同一台设备、同一宿主、同一运行时，连续三次点"打开桌面"都正常进到桌面：

```text
19:24 桌面可见：顶栏时钟、左侧 Ubuntu 坞、桌面图标（主目录/回收站）、壁纸
19:27 同上
19:29 同上
```

三次都**没有**任何 SHM 相关兜底在场。因此 350 §4 那次不能写成"必现"，
只能写成"旧现场出现过一次，条件未知"。

## 2 · 设备上的外挂残留已清

升级实验期间在设备树里留下过两个文件，本轮删除并做了单变量验证：

```text
rootfs/etc/profile.d/zz-husk-noshm.sh          # export LD_PRELOAD=libhusk-noshm.so
rootfs/usr/lib/aarch64-linux-gnu/libhusk-noshm.so
rootfs/tmp/unit/, rootfs/tmp/gpu-verify.sh, rootfs/tmp/run-unit.sh
```

删除后重启两次，桌面照常起来 → 这两个文件不是"桌面能起来"的原因。
删除后会话环境实测 `LD_PRELOAD=[none]`。

## 3 · 宿主钉死的旧 ICD：删不得，但可以改指向

宿主 `doro-gpu-run` 把 `VK_ICD_FILENAMES` 写死成
`/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json`。会话内实测：

```text
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
GALLIUM_DRIVER=zink
MESA_LOADER_DRIVER_OVERRIDE=zink
PANVK_DXVK_COMPAT=0
```

**实验 A（删文件）**：把这个 json 移走 → 点"打开桌面"后
`GNOME 已退出，退出码 139`（SIGSEGV）。即：宿主那条钉死是启动硬依赖，
"宿主删掉钉死那行"在没有改宿主的前提下等于把桌面打死。证据显示加载器的
坏值纠正在宿主直启这条链路上没有生效，不能拿 evidence 351 的
`VK_ICD_FILENAMES=/nonexistent.json` 结论覆盖这条路径。

**实验 B（改内容）**：保留文件名，把 `library_path` 从
`libvulkan_panfrost_x11.so` 改成 `libvulkan_panfrost_husk.so` → 桌面正常起来，
会话内实证：

```text
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json   # 名字没变
libvulkan_panfrost_husk.so                                                  # maps 里加载的是它
mali 映射条目 83 条，/dev/mali0 fd 2 个
renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

两份驱动字节相同（`599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c`），
所以实验 A/B 的差异不是驱动差异，而是"清单在不在"。

## 4 · 安装器改动（防回归）

`tools/guest/install-husk.sh` 写完 canonical `husk_panvk_guest.json` 之后，
把树里已知的旧清单（`husk_panvk_frist370_execva.json` / `husk_panvk_frist370.json` /
`husk_panvk_x11.json` / `husk_panvk_g77.json` / `husk_panvk.json`）逐个改写为
指向 canonical 驱动，并删除 `icd.d/husk_panvk*.json.bak-*` 备份。
这样：文件名保留（宿主钉死仍然解析得到），内容唯一（不会再有机会加载别的 .so），
`.bak-xcb` 这类实验备份也不会被 Vulkan loader 扫到。

设备上另有一个 `husk_panvk_frist370_execva.json.bak-xcb` 残留，已删。

## 5 · 本轮门禁

```text
19-readback-suite : PASS=19 FAIL=0   (renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)))
15-gpu-global     : GPU_GLOBAL_AUDIT PASS=3 FAIL=0
                     gnome-shell pid=8977 gpu=zink gsk=cairo
                     nested-gtk  pid=14695 gpu=zink gsk=cairo
```

## 6 · 未验证

- H38 的真实触发条件（冷启动？首次 SHM 请求之前的时序？）——没有复现，写 UNVERIFIED。
- 桌面冷启动时延、强退重进、30 s 重进三组数字：本轮只做了"能不能起来"。
- Overview 黑色三角、交互卡顿、音频、相机：本轮未跑。
