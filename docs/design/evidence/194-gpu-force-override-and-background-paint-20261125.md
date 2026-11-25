# 194 · 桌面 shell 真吃到 GPU 了；同时抓到一个背景只画一部分的缺陷

日期：2026-11-25 14:00–15:00（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4.19.191 / 4 KiB），应用 `com.linuxhub.myth.husk`，X server `:0`（应用自己的 `:x11` 进程）。
这一份记两件事：**加载器把图形默认值从"填空"扩到"覆盖/屏蔽"之后的实测**，以及**桌面背景
只画了一块区域**这个新缺陷的量化。缺陷本身**没有解决**，这份只把现象、范围、排除项钉死。

## 1 · 为什么要把默认值从"填空"扩成"覆盖"

`gpuenv` 之前只有一档：程序没设才补。LinuxHub 2.0 的会话脚本自己会 export
`MESA_LOADER_DRIVER_OVERRIDE=swrast GALLIUM_DRIVER=llvmpipe LIBGL_ALWAYS_SOFTWARE=1`
（`GpuRuntimeEnvironment.kt` 里写死，注释说"PRoot 上 mutter 必须 swrast"），于是加载器的
默认值全部让位，桌面 shell 永远停在 llvmpipe —— 这正是 evidence 192 §40 看到的东西。

0.0.6 改法：`/etc/husk/gpu-env` 支持三种行前缀，加载器在 `getenv()` 里按顺序查：

| 写法 | 语义 | 什么用它 |
|---|---|---|
| `NAME=VALUE` | 程序没设才补（原来的行为） | `MESA_GLES_VERSION_OVERRIDE`、`GSK_RENDERER` |
| `!NAME=VALUE` | 无条件覆盖，程序设了也照这个 | `GALLIUM_DRIVER=zink`、`MESA_LOADER_DRIVER_OVERRIDE=zink` |
| `-NAME` | 让这个变量对程序**不可见**（getenv 返回 NULL） | `LIBGL_ALWAYS_SOFTWARE`（它比 GALLIUM_DRIVER 更优先） |

逃逸开关照旧最先判：`HUSK_GPU=llvmpipe` 或 `NR_GPU_DEFAULT_OFF=1` 一条都不注入。

## 2 · 验证（FACT）

### 2.1 填表：故意给最坏的环境

`gl-probe es3` 同一份二进制，用的是**应用那套**环境：

| 预设 | RENDERER |
|---|---|
| 无预设 | `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` |
| `GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=swrast LIBGL_ALWAYS_SOFTWARE=1` | 仍然是 `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))` |

### 2.2 逃逸开关仍然优先

| 开关 | RENDERER |
|---|---|
| `HUSK_GPU=llvmpipe` | `llvmpipe (LLVM 20.1.2, 128 bits)` |
| `NR_GPU_DEFAULT_OFF=1` | `llvmpipe (LLVM 20.1.2, 128 bits)` |

### 2.3 真实 APP 链：shell 真的在 zink 上

应用自己拉起的会话，`gnome-shell`（`USER=husk`、`HOME=/home/husk`）：

```
/proc/6997/maps: .../libvulkan.so.1.3.275
                 .../libvulkan_panfrost_x11.so
/proc/6997/environ: MESA_LOADER_DRIVER_OVERRIDE=swrast GALLIUM_DRIVER=llvmpipe LIBGL_ALWAYS_SOFTWARE=1
```

**坑（必须写下来）**：`/proc/<pid>/environ` 仍然显示 llvmpipe，因为改的是 `getenv()` 的返回值，
不是进程环境块。**用 environ 判断渲染器会看走眼**，以后一律看 `maps` 里有没有 libvulkan / PanVK，
或者直接跑 gl-probe。192 §40 那条"APP 链 shell 是 llvmpipe"就是被 environ 骗的结论，现在推翻。

## 3 · 新缺陷：桌面背景只画了一块区域

### 3.1 zink 会话：右侧一块三角，亮度只有 1/3

把屏幕像素和 `usr/share/backgrounds/warty-final-ubuntu.png`（按 zoom-fill 换算到 2400×1080）
逐点比：

| 采样点 | 屏幕 | 壁纸 | 比值 |
|---|---|---|---|
| (1600,1000) | 106,37,65 | 106,36,64 | 1.00 |
| (1900,200) | 41,8,26 | 41,8,25 | 1.00 |
| (2100,300) | 11,2,6 | 35,6,19 | 0.31 |
| (2200,900) | 38,13,16 | 118,41,49 | 0.32 |
| (2350,1000) | 43,15,15 | 135,48,47 | 0.32 |

边界是一条直斜线：y=100 时 x≈2092 起变暗，y=340 时 x≈1780，斜率约 −1.3 px/行。
**不是壁纸自带的折痕**（左侧逐点 1.00 吻合）。

### 3.2 llvmpipe 会话：背景只画到第 480 行

同一台机器、同一张壁纸、`GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=swrast`：

| 行 | 纯黑占比 |
|---|---|
| y=400 | 0.00 |
| y=470 | 0.00 |
| y=480 | 0.63 |
| y=600 | 0.63 |
| y=900 | 1.00 |

（0.63 = 非窗口区域全黑；窗口本身照常绘制。）连抓两次、间隔一分钟，数字一致。

### 3.3 排除项（都做过，FACT）

| 怀疑 | 实验 | 结果 |
|---|---|---|
| 加载器新改动 | 换回改动前的 `libld.so`（md5 `e5bfb5…`）重开会话 | 数字完全一样 |
| DING（桌面图标进程） | kill 掉全部 DING 再抓 | 一样 |
| 没重绘/脏区残留 | 连按两次 Super 进概览再退（整屏重绘） | 一样 |
| 多屏拼错 | `xrandr`：只有 `builtin 2400x1080+0+0` | 单屏 |
| 屏幕中途改大小导致背景尺寸没跟上 | 会话启动后 80 次采样（0.25 s 一次）`xwininfo -root` | 全程 `2400x1080`，没有改过 |
| 壁纸文件本身 | 与文件按 zoom-fill 换算逐点比 | 左侧 1.00 吻合 |

**UNKNOWN**：根因还没找到。现象共同点是"背景 actor 没覆盖整屏"，但两种渲染器表现不同
（zink 右侧 1/3 亮度、llvmpipe 只画 0–479 行），所以不像单纯的三角形光栅化错误。

## 4 · 顺手补的工具：`xcap`

这台机器的 `screenrecord` 被 ROM 屏蔽（`/system/bin/screenrecord` 存在但 shell 执行
`rc=127 inaccessible`），`screencap` 又只能看到应用窗口（会话不是我起的就会截到
"GNOME 已退出"覆盖层）。所以写了个 40 行的 C：

```
XGetImage(root, 0,0,W,H) -> "W H bpp stride" + BGRX 原始像素
```

在 guest 里 `gcc -O1 -o tmp/xcap tmp/xcap.c -L/usr/lib/aarch64-linux-gnu -l:libX11.so.6` 编，
宿主机用 PIL 转 PNG。后面所有视觉证据都用它，避免再出现"截到的是应用覆盖层"这种误判。

## 5 · 环境坑（写下来免得再踩）

1. **`pkill -x gnome-shell` 打不中**：经加载器起的进程 comm 是 `gnome-shell`（进程自己
   `prctl` 设的），但脚本里的 `pkill -x` 有时匹配不到；按 `/proc/*/cmdline` 匹配才可靠。
2. **应用会自己再拉一个 shell**：我把它的会话进程杀掉之后，应用会重启一个。两个 shell 用
   `--replace` 互相抢 WM，**谁最后起来谁是 WM**——于是带 `LD_PRELOAD` 探针的那个 shell 可能
   根本不是 WM，抓不到任何 `XIGrabDevice`。
3. **上一轮的客户端窗口还活着**：X 客户端不随 WM 重启死掉，桌面上会堆一排陈旧的
   `fresh-probe` / `Public` / `root@localhost`。指针停在哪个窗口上，点击就归谁——拖窗测试
   必须先按 cmdline 清干净客户端，否则量到的是旧窗口。
4. 清场脚本 `kill -9` 的范围要显式排除 `usr/bin/gnome-shell` / `mutter-x11-frames` / `ding.js`；
   这一轮有一次把应用自己的 `:x11` 一起带走，整个显示没了。

## 6 · 这一轮的结论

- 0.0.6 的 GPU 强制覆盖**成立**：应用那套 llvmpipe 环境不再能压住桌面 shell 的 zink，
  逃逸开关仍然有效，改法全在加载器（Rust）+ `/etc/husk/gpu-env`，没动 2.0 代码。
- 桌面背景"只画一块"是**发布阻塞项**：它和渲染器无关（两种都有，只是形态不同），
  也不是我这轮改动引入的。
- 窗口拖动（目标 ②）没能出数：探针没抓到任何 `XIGrabDevice`，因为拖动根本没进 mutter。
  下一次要先把"谁是 WM / 谁是窗口持有者"钉死，再拖。

## 7 · 图

- `194-bg-zink-right-dark.png` — zink 会话：右侧一块三角比壁纸暗到 1/3。
- `194-bg-llvmpipe-top480.png` — llvmpipe 会话：背景只画了 0–479 行，下面全黑。
