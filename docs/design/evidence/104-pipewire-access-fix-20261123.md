# evidence 104 — PipeWire 客户端在 guest 里卡死的根因与修法

设备 RMX3357 / Android 13 / 4.19.191 / 4 KiB；guest `untrusted_app`。2026-11-23。

## 1. 症状（FACT）

有 session bus 时，`pw-cli` / `pw-dump` 连上 PipeWire 后永远收不到 registry 事件；
没有 session bus 时同一份 pipewire、同一个二进制正常工作。服务端日志（`PIPEWIRE_DEBUG=4`）：

```
mod.access | client 0x… has trusted pid 27056
mod.access | client 0x… flatpak check failed: No such file or directory
mod.access | client 0x… is from flatpak
mod.access | client 0x… wait for 'flatpak' permissions
pw.client  | pw_impl_client_set_busy() busy 1
```

## 2. 根因（FACT，源码 + 实测对上）

pipewire 的 `module-access` 在默认（legacy）模式下用 `pw_check_flatpak()` 判断客户端是不是
flatpak 应用，它第一步就是 `openat("/proc/<pid>/root")`。

我们的 shim 有意**不把 `/proc/*/root` 与 `/proc/*/cwd` 透传**（防止 guest 拿它们跳出 rootfs），
于是内核返回 `ENOENT`（而不是 pipewire 预期用来"当作没沙箱"的 `EACCES`）。
pipewire 的代码把 `res != 0`（含负数错误）一律当成"客户端来自 flatpak"，
于是客户端被挂上 `ACCESS_FLATPAK`，**永久 busy**，一条 registry 事件也不发。

所以这不是 mutter、也不是 D-Bus 的错，是"我们把 `/proc/*/root` 挡住"与"pipewire 把错误码当沙箱判据"
两者相撞。

## 3. 修法（已验证）

不用改 shim 的逃逸防护，改 pipewire 的 access 模式：给它 `access.socket` 规则，
legacy 模式就不启用，flatpak 检查根本不会发生：

```
{ name = libpipewire-module-access
    args = { access.socket = { pipewire-0 = "unrestricted"
                               pipewire-0-manager = "unrestricted" } } }
```

配置文件放 `tools/guest-config/pipewire.conf`（其余与发行版默认一致）。

## 4. 修好之后（同一轮实测）

```
husk-rd|sc|started
husk-rd|signal|PipeWireStreamAdded|/org/gnome/Mutter/ScreenCast/Stream/u1|node=Some(32)

pw-cli ls Node
  id 32, node.name = "libld39.so", media.class = "Stream/Output/Video"
pw-cli info 32
  media.name = "meta-screen-cast-src", state suspended, 1 output port
  params: PropInfo / Props / EnumFormat(r-) / Format(-w)
pw-cli enum-params 32 Spa:Enum:ParamId:EnumFormat
  video/raw BGRx (和 BGRA) 1280x720, 1x1..16384, framerate 变长
```

结论：**ScreenCast 的 Producer 节点现在真的能看到了**，格式是 BGRx/BGRA。
`Buffers` 不是节点参数（节点只列 PropInfo/Props/EnumFormat/Format），
要看 buffer 类型必须作为消费者连上去协商——也就是下一步要写的 Rust 消费者。

## 5. 下一步

1. 写 Rust 消费者：连到 node 32，`EnumFormat` 里带 `SPA_FORMAT_VIDEO_modifier`（LINEAR），
   观察 mutter 是否按 dmabuf 分配（源码层面它在这种情况下用 `SPA_DATA_DmaBuf`）。
2. 拿到 dmabuf fd → `AHardwareBuffer` → 复用已验证的 3-buffer ring 上屏。
3. 顺带确认 `pw-cli` 的正常输出（本次已恢复），把它写进设备测试脚本。
