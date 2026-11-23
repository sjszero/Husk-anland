# evidence 99 — ScreenCast 虚拟显示器 + PipeWire：链路建起来了，帧还没到

设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 已经成立的（FACT）

1. `org.gnome.Mutter.ScreenCast` 三步走全部成功（`husk-rd sc`）：
   ```
   husk-rd|sc|session|/org/gnome/Mutter/ScreenCast/Session/u3
   husk-rd|sc|stream|/org/gnome/Mutter/ScreenCast/Stream/u3
   husk-rd|sc|started
   husk-rd|signal|PipeWireStreamAdded|...|node=Some(32)
   ```
   顺序必须是 **CreateSession → RecordVirtual → Session.Start**。`Session.Start()` 会把
   它的 stream 一起启动，之后再调 `Stream.Start()` 只会得到 "Stream already started"；
   `PipeWireStreamAdded` 在 `Session.Start()` 期间发出，所以订阅必须在它之前。
2. 该节点在 PipeWire 里是 `Stream/Output/Video`，`state: "running"`，
   `EnumFormat` = video/raw **BGRx**，size 1x1..16384，framerate 0/1（可变）。
3. `pipewire` 守护进程 + `pw-cli` 在这个 guest 里工作正常（1.0.5）。
4. **手动链接可以建立并处于活动状态**：`pw-link <out-port-id> <in-port-id>` 之后
   `pw-link -l` 显示 `|->`（active）。注意 `pw-link 32:output_1 35:input_1` 这种
   "节点id:端口名" 写法会报 *Input & output port do not exist*，要传**端口 id**。
5. GTK3 客户端在 mutter 上能起到窗口阶段（补齐 xkb 数据后不再 segv）。

## 卡点（FACT + 归因）

**帧还没到消费者手里**，原因不在 PipeWire，而在"没有东西可画 + 客户端起不完整"：

- **没有 session manager 就没有自动连线**：`wireplumber 0.4.17` 在本机启动即
  `on_disconnected: disconnected from pipewire`（exit 70）。它走 `default_access`
  → 需要 logind；本机没有 systemd/logind（`/run/dbus/system_bus_socket` 不存在）。
  理论上可以用 `pw-link` 手动连（已验证），但那只是测试手段，产品里要么修
  wireplumber 配置、要么自己实现"最小策略层"。
- **GTK 客户端半路 bail**：`Gtk:ERROR ... Failed to load .../image-missing.png:
  Unrecognized image file format`、`Could not load a pixbuf`。即 gdk-pixbuf 的
  loader 模块（运行时 dlopen）没被找到。
- **同一个根因**：`XKB_CONFIG_ROOT` 补上后 xkb 才不报错；pixbuf 又要另一套 env。
  这说明 **loader 的 path-shim 只覆盖启动时已加载对象**，运行期 `dlopen` 进来的库
  （libxkbcommon、gdk-pixbuf loaders、libspa-*、libgstpipewire）里的 `open()` 仍然
  走到宿主路径。证据链：`docs/design/libc-interpose.md` 里"libc body patch"一直是
  未实现项，而 `NR_BODY_PATCH=1` 在本机因 `mprotect` 回 r-x 失败（EACCES）不可用。

## 结论（JUDGMENT）

显示路线 B（mutter headless + ScreenCast + PipeWire）的**协议面已经通**，剩下的不是
"能不能"，而是两件工程活：

1. **让 path-shim 覆盖 dlopen**：要么做一个 LD_PRELOAD 版路径层（和 v4l2-preload 同一
   手法，但覆盖 open/stat/opendir/access/fopen 全家），要么按 `libc-interpose.md` 把
   redirect 烘进 libc。没有这一步，GTK/GNOME Shell/任何插件式组件都起不完整。
2. **一个最小 session manager**（或修好 wireplumber 的 logind 依赖），让采集流自动连线。

在 1 做完之前，用 env 打补丁（`XKB_CONFIG_ROOT`、`GDK_PIXBUF_MODULE_FILE`…）只能让
单个程序勉强起来，不能作为产品路径。
