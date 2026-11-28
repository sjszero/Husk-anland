# 350 · 在测试宿主 App 内升级 Husk 运行时（0.0.37 第一轮）

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
宿主：`com.linuxhub.myth.husk`（黑盒运行容器，未改其源码）
Husk：`VERSION=0.0.36`、提交 `c924d620f6de`（未升版、未提交）

## 0 · 结论

| 项 | 结果 | 判定 |
|---|---|---|
| 运行时已整包替换为本次构建（loader/工具/ICD/驱动/配置） | 是 | FACT |
| 运行身份落盘 `rootfs/etc/husk/runtime-identity` | 是，六项齐全 | FACT |
| 门禁 A 核心 | `PASS=38 FAIL=0 SKIP=0` | FACT |
| GPU 读回 19 项 | `PASS=19 FAIL=0`，zink Vulkan 1.3 (Mali-G77, MESA_PANVK) | FACT |
| GPU 并发长稳 4×300 s | `PASS=19 FAIL=0`，fd=7 稳定，RSS Δ≤524 kB | FACT |
| 桌面会话 | **起不来**：宿主 X server 拒绝 MIT-SHM | 外部阻塞 |
| 音频 | `AUDIO_AUDIT PASS=1 FAIL=3`，`/run/husk/audio-out.sock` 不存在（宿主侧桥未起） | 外部阻塞 |
| A-hw | `PASS=4 FAIL=11`，宿主 App 无相机/麦克风/定位授权 | 需人工授权 |

## 1 · 本次部署身份

```text
version=0.0.36
git_commit=c924d620f6de
mode=upgrade
loader_sha256=67f543b9df61d832f6f7b8fb752f37f775300a96d9a690095932e1cb9fa1db5d
libc_sha256=16d5ba2120c1a92c0c9cba62dd8ec244d29d5d30a733d4ac530be4fb1b92933f
gpu_env_sha256=ad230fcc0f8248ccfbf657e0b28e00bcf28cd41f3e644bdc1788f522f4445603
panvk_driver_sha256=599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c
icd_sha256=25f6ccd9662ada7072b43d87aaa45dcf9413519918a30d55679b60042afd0eae
```

设备侧自证：`cat <rootfs>/etc/husk/runtime-identity`；`sha256sum` 重新计算必须逐项相等。

## 2 · 安装器这一轮的改动（都是升级路径暴露出来的）

1. `--upgrade` 模式：不重解 4.3 GB 树，只重跑 guest 侧步骤（工具、ICD、驱动、配置、locale、符号链接、身份）。
2. `LIBS` 从"解包流程内部"提到全局：升级模式跳过解包后，所有 guest 调用都因 `set -u` 直接死。
3. DING 补丁判定改为"load_bytes 调用是否在 try 块内"：宿主自带的 DING 已经是多行 try/catch，旧判定只认单行写法，把已修好的树判成未修。
4. 升级前按 cmdline 停掉指向本树的 guest 进程：否则 `cp usr/bin/husk-logd` 报 `Text file busy`。
   匹配必须同时认 `/data/user/0/...` 与 `/data/data/...` 两种写法——运行中的进程用的是后者。
5. 工具改名后同时保留 `usr/bin/<cmd>` 链接：宿主 sudo-access 步骤写死 `chown /usr/bin/passwd`，
   名字消失会让它中止并停掉整个桌面。

## 3 · 缺陷：两次构建产出不同 loader，其中一份丢 /proc/self/exe

```text
67f543b9...  869888 B  /proc/self/exe -> /usr/bin/readlink     门禁 A 38/0/0
d5dd7e63...  738816 B  /proc/self/exe -> <app>/files/linux/husk/libld.so   门禁 A 35/3
```

同一份源码，`make loader` 现在可复现为 67f543b9；738816 B 那份来自另一次 `make dist` 内的构建。
坏件不是"功能全废"：基本 exec 正常，只有 `publish_exe_guest()` 没生效，于是
`sys.executable`、按自身目录找资源的程序全部错位，而门禁 A 只有 3 条能抓到。
结论：**loader 必须带构建自检与可复现断言**，不能只看"门禁大multi通过"。见 H37。

## 4 · 外部阻塞：宿主 X server 拒绝 MIT-SHM（H38）

```text
(gnome-shell:3763): libmutter-WARNING: NOTE: Not using GLX TFP!
(gnome-shell:3763): Mtk-ERROR: Received an X Window System error.
  error_code 10 request_code 130 (MIT-SHM) minor_code 1   <- XShmAttach
GNOME 已退出，退出码 133                                  <- SIGTRAP
```

`DISPLAY=:0 xdpyinfo` 显示 `MIT-SHM Present`，即宿主 X proxy **宣告支持却拒绝 attach**。
Mutter 46 没有"禁用 SHM"的开关，因此客户端只会 trap。修法在宿主侧：真正支持 ShmAttach（POSIX shm
FD 传递），或干脆不宣告 MIT-SHM 让客户端退回非 SHM 路径。

## 5 · 未验证 / 需人工

- 桌面、输入、Overview 三角：被 H38 卡住，**没有 Husk 侧结果**。
- 音频：宿主未起 `audio-out.sock`，无法判定 Husk 侧链路。
- 相机/麦克风/定位：宿主 App 无授权，按清单不得用 ADB 手动授权冒充。
- 8 个 `com.linuxhub.husk`（已卸载包）残留进程仍在跑，shell 无权 kill，需重启设备清理。
