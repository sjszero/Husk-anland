# 兼容性

同一份 rootfs 上，什么能用、什么不能用。「可用」指在目标机上真的执行过并得到预期结果，
不是「包已经装了」。

参考环境（实测条件，不是要求）：arm64 Android 13、kernel 4.19.191、4 KiB 页，以
`untrusted_app` 运行，无 root。具体机型与 SoC 见 `docs/STATUS.md`。

## shell、工具链、语言运行时

| 项 | Husk | proot | proot-distro | 证据 |
|---|---|---|---|---|
| `bash` / coreutils | 可用 | 可用 | 可用 | bench `have_*` |
| `git`（clone + status） | 可用，889 ms | 可用 | 可用 | bench `git_clone` |
| `python3` + `json` | 可用，88 ms | 可用 | 可用 | bench `python_json` |
| `python3 -c 'import numpy'` | 可用 1.26.4，410 ms | 可用 | 可用 | bench `python_numpy`、evidence 155 |
| `gcc` 编译 hello | 可用，165 ms | 可用 | 可用 | bench `gcc_hello` |
| `make -j4`（200 目标，链接产物 91 KB） | 可用，5 194 ms | 可用 | 可用 | bench `make_j4`、evidence 155 |
| `vim` / `nano` / `node` | 可用 | 可用 | 可用 | bench `have_*` |
| `apt-get install` / `dpkg -l` / `dpkg --configure -a` | 可用 | 可用 | 可用 | evidence 154 |
| `ldconfig`（`husk-ldconfig`） | 可用，950 条 / 6 目录 | 用树自带的静态 ldconfig | 同 proot | evidence 154 |

## 系统服务（guest 内）

| 项 | Husk | 证据 |
|---|---|---|
| dbus 会话总线（`dbus-send --session ListNames`） | 可用 | evidence 152 |
| polkit 授权（`pkcheck --action-id … power-off`） | 可用，rc=0 | evidence 151 |
| 无障碍总线 at-spi | 可用 | evidence 151 |
| gvfs（`gio mount -l`） | 可用，不回退到会话总线 | evidence 152 |
| RandR（`xrandr --version`） | 可用，无缩放告警 | evidence 152 |

## GPU

| 项 | Husk（当前版本的树） | 证据 |
|---|---|---|
| `/dev/mali0` 直开 | 可用 | evidence 156 |
| PanVK（kbase 后端）枚举 + compute + 离屏 graphics | 可用，`Mali-G77 MC9` | evidence 156 |
| GL/EGL 经 Zink 落到 GPU | 可用，renderer `zink Vulkan 1.3(Mali-G77 MC9)`，上限 GLES 2.0 | evidence 156 |
| 桌面硬件加速（WSI / 窗口呈现） | 未做 | `docs/LIMITATIONS.md` §7 |

不开 Zink 时 GL 落在 `llvmpipe`；开了 zink 但用发行版自带的 `panfrost_icd.json` 会
`VK_ERROR_INCOMPATIBLE_DRIVER`，因为它要的是 DRM render node，本机只有 kbase。

## 浏览器

不可用，原因明确：随附 rootfs 来自离线包索引，其中没有浏览器包
（`epiphany-browser` / `firefox` 均 `Unable to locate package`；`xterm`、
`gnome-terminal`、`nautilus` 等则在索引内）。要装浏览器需要换一份完整镜像源，不在本版范围。

## 不在支持范围

Termux 不是交付形态（`LIMITATIONS.md` §3）。传感器 / GPS / USB / 蓝牙 / NFC / 指纹未开始；
硬件视频解码只有应用侧的 MediaCodec 链路（evidence 80），guest 内的全链未见可用结论。
桌面硬件加速（WSI）未做，见 `LIMITATIONS.md` §7。
