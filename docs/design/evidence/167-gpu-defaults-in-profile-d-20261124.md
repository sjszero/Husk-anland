# 167 · GPU 运行时"默认就有"：/etc/profile.d 而不是每个应用各配一遍

日期：2026-11-24（CST）。设备 RMX3357（MT6893 / Mali-G77），包 `com.linuxhub.myth.husk`。

## 0 · 要解决的问题

到 evidence 165 为止，GPU 是靠**会话脚本**注入环境变量才有的（`husk-x11-gpu*.sh` 里写死
`GALLIUM_DRIVER=zink`、`VK_ICD_FILENAMES=…`）。后果：从会话里拉起的程序有 GPU，
**进了 guest 再开的程序没有**（比如 `sh -l` 之后跑一个 GTK 程序，或者用户在终端里手敲）。

这就是"能不能像真机一样全局提供加速"的最后一公里。做法不是给每个应用加参数，
而是把默认值放到 guest 自己会读的地方：`/etc/profile.d/husk-gpu.sh`。

## 1 · 为什么不能"文件放对就行"，必须要环境变量

两件硬事实（都在本机看过，不是推测）：

1. 这个 rootfs 装了上游 `mesa-vulkan-drivers` 的全套 ICD：
   `asahi/broadcom/intel/radeon/nouveau/freedreno/lvp/panfrost/gfxstream` 都在
   `/usr/share/vulkan/icd.d/` 里。Vulkan loader 会挨个试，其中 `panfrost_icd.json` 指向的库
   **没有 x11 平台**（evidence 165 §1），会和我们的 ICD 抢设备。
2. 我们的 PanVK 是 `-Dgallium-drivers=` 编的，只有 Vulkan，没有 gallium panfrost，
   所以 GL 必须走 zink → Vulkan。这条不是偏好，是构建产物决定的。

所以"默认"的准确含义是：**默认值写一次**，而不是"不用配置"。写一次的位置必须让
guest 里的任何程序都能拿到 —— `/etc/profile.d` 就是那个位置（登录 shell、`bash -l`、
以及从它派生的所有子进程）。

## 2 · 落盘与逃逸

`tools/guest-config/husk-gpu-defaults.sh` → 安装器写到 `<rootfs>/etc/profile.d/husk-gpu.sh`
（`tools/guest/install-husk.sh` 的 2b-2b 步）。规则：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `VK_ICD_FILENAMES` / `VK_DRIVER_FILES` | `/usr/share/vulkan/icd.d/husk_*.json`（取第一个，跳过 `*.bak`） | 从全套 ICD 里挑出我们那个 |
| `MESA_LOADER_DRIVER_OVERRIDE` / `GALLIUM_DRIVER` | `zink` | 构建里没有 gallium panfrost |
| `MESA_GLES_VERSION_OVERRIDE` | `3.2` | 这一版组合的上限，写在唯一一处 |

- **不覆盖已有值**：用户自己 export 过的，一律不动。
- 逃逸：`HUSK_GPU=llvmpipe`（脚本已在用）或 `NR_GPU_DEFAULT_OFF=1`。
- `husk-session.sh` 里原来那套注入保留：它覆盖的是"App 拉起的那条会话链"，
  profile.d 覆盖的是"进到 guest 以后"。两者值相同，互为兜底。

## 3 · 实测（只给 DISPLAY，不给任何 GPU 变量）

```
$ sh xe-drmfd.sh usr/bin/bash -lc "export DISPLAY=:0; /tmp/gl-probe window"
stage 6: context OK
stage 7: makeCurrent OK
renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
stage 8: first draw OK
stage 9: first swap OK
stage 10: 60 swaps OK
readback px=0,255,0,255 err=0x3000
RESULT PASS
```

对照（同一命令，但 `HUSK_GPU=llvmpipe`）在 evidence 156 里记的是 `llvmpipe (LLVM …)`。
也就是说：**同一个二进制，默认进 GPU，一个变量就能退回软件**，不需要改任何启动脚本。

## 4 · 仍未做（UNKNOWN / NOT DONE）

- 桌面以外的硬件（视频/音频/相机/输入）走的不是环境变量而是 shim 与 App 侧后端，
  它们本来就"默认在"，但**没有**单独的默认值文件；是否需要同样写一份，未评估。
- `MESA_GLES_VERSION_OVERRIDE=3.2` 只是把版本号报高：超出 3.2 的实际能力边界没测。
- Adreno / 高通：profile.d 里没有 kgsl 相关项（我们只编了 PanVK）。等实机。
