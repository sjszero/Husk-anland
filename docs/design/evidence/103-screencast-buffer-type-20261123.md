# evidence 103 — ScreenCast 流的 buffer 类型：结论 + 一个新阻塞点

设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 1. 想测什么

mutter 46 的 ScreenCast 流到底是 **dmabuf** 还是 **system memory**（memfd）。
这决定桌面帧能不能零拷贝上屏。

## 2. 源码层面的答案（FACT）

mutter 46.2 的 ScreenCast 流实现两条路都支持，**由消费者协商决定**：

- 枚举格式时，如果渲染器支持 dma-buf（`cogl_renderer_is_dma_buf_supported()` 且
  `eglQueryDmaBufModifiersEXT` 能拿到 modifier 列表），格式对象里会带
  `SPA_FORMAT_VIDEO_modifier`；
- 消费者回传的格式里**带 modifier** → 分配 `SPA_DATA_DmaBuf`（fd 直接给消费者）；
- 消费者回传的格式里**没有 modifier** → 分配 `SPA_DATA_MemFd`（内存映射，必有一次拷贝）。

结论：**零拷贝不是 mutter 的限制，是我们这一侧协商什么就给什么。**
我们的 guest 已经能 import dma-buf 到 PanVK（evidence 49/50/73），所以这条路在原理上通。

## 3. 现场实测（FACT）

环境：guest 内 `dbus-daemon --nofork` + `pipewire` 1.0.5 + `mutter --headless --no-x11`，
然后 `husk-rd sc 60`：

```
husk-rd|sc|session|/org/gnome/Mutter/ScreenCast/Session/u1
husk-rd|sc|stream|/org/gnome/Mutter/ScreenCast/Stream/u1
husk-rd|sc|started
```

**没有 `PipeWireStreamAdded` 信号**，`pw-cli ls Node` 也拿不到任何节点。

## 4. 新阻塞点：guest 里有 session bus 时 PipeWire 客户端会卡住（FACT，已二分）

| 条件 | `pw-cli ls Node` |
|---|---|
| 没有 session bus（同一台机、同一个 pipewire、同一份配置） | **正常，列出 2 个节点** |
| 有 session bus（`$XDG_RUNTIME_DIR/bus` 存在） | **卡住，30–40 秒也不返回任何节点** |

补充事实：
- D-Bus 本身健康：`dbus-send` 的对端 Ping 与"服务不存在"都立即返回，bus 不阻塞；
- 卡住的不是服务端启动：`pipewire-0` socket 正常、server 日志无错误、
  去掉 `libpipewire-module-access` / `module-portal` 后症状不变；
- 客户端已经连上并对 core 做了 bind（`PIPEWIRE_DEBUG=4` 里能看到 `proxy id 1 bound 32`），
  之后再也收不到 registry 事件；
- 与 `pw-cli` 自己的环境变量无关：客户端不带 `DBUS_SESSION_BUS_ADDRESS`，
  只要 bus socket 在 `$XDG_RUNTIME_DIR` 下，照样卡（libdbus 会自己找到它）。

## 5. 下一步（按顺序）

1. 服务端 `PIPEWIRE_DEBUG=4` 看客户端注册到哪一步断掉——是 access 检查、还是 registry 广播没发。
2. 若确认是 libdbus 在客户端的某次阻塞调用：把 pipewire 客户端改成不碰 session bus 的配置
   （`module-rt` 不加载）再验一次。
3. 通之后，用我们自己的 Rust 消费者把格式协商成带 modifier 的那一版，再 `enum-params` 确认
   buffer 类型；这个消费者本来就要写（PipeWire → dmabuf → AHB → SurfaceControl）。

## 6. 顺带的环境教训

- 每次实验前必须精确清掉上一轮的 `mutter` / `pipewire` / `dbus-daemon`：
  这次留了 15 个残留进程，会把后面所有观察污染成"看起来卡住"。
- `--library-path` 是替换不是追加；本机 loader 只吃相对程序路径。
