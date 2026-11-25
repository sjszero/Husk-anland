# 193 · Dock 里到底有几个、动画到底跑不跑、音频到底通不通

日期：2026-11-25 13:43–13:52（CST）。设备 RMX3357（MT6893 / Mali-G77 MC9 / Android 13 /
4 KiB 页），应用 `com.linuxhub.myth.husk`，会话由 **APP 自己**拉起（evidence 192 那条链）。
X 屏 2400x1080，会话 shell 跑在 llvmpipe（见 192 §1）。

## 1 · Dock：配了 7 个，屏幕上只有 4 个，都不是我们放的（FACT）

`org.gnome.shell favorite-apps` 的live值：

```
['firefox-esr.desktop', 'org.gnome.Evolution.desktop', 'rhythmbox.desktop',
 'libreoffice-writer.desktop', 'org.gnome.Nautilus.desktop',
 'org.gnome.Software.desktop', 'yelp.desktop']
```

来源不是我们写的东西，是 rootfs 里 Ubuntu 自带的那份 override：

```
/usr/share/glib-2.0/schemas/10_gnome-shell.gschema.override:2
favorite-apps=[ 'firefox-esr.desktop', … 'yelp.desktop' ]
```

（我们自己的 `99-husk-look.gschema.override` 只写主题/字体/壁纸/窗口摆放，不碰 dock。
2.0 侧 `app/src/main/assets/desktop/gnome/apply-ubuntu-defaults.sh:22` 另有一套
Nautilus/Terminal/TextEditor/Settings 的写法，但那条没有落到这个会话的 dconf 里。）

屏幕上实际渲染的 4 个（截屏左侧 0–90 px 放大，见 `193-dock.png`）：

| # | 图标 | 对应 desktop 文件 | 状态 |
|---|---|---|---|
| 1 | 蓝色文档 | `libreoffice-writer.desktop` | 已装，能起 |
| 2 | 文件夹 | `org.gnome.Nautilus.desktop` | 已装 |
| 3 | 问号 | `yelp.desktop`（帮助） | 已装 |
| 4 | 垃圾桶 | —— | **不是 favorite**，是 dash 底部的 trash |

缺的 3 个是**没装的包**，不是 bug：`firefox-esr`、`org.gnome.Evolution`、`rhythmbox`、
`org.gnome.Software` 在这个最小 rootfs 里都没有，desktop 文件不存在 → 不渲染。
第一个图标左上角那个小圆点是"正在运行"指示。

要改就一句话（会话总线里）：

```sh
gsettings set org.gnome.shell favorite-apps "['org.gnome.Nautilus.desktop','org.gnome.Terminal.desktop']"
```

## 2 · 动画：是有的，而且是新链路的动画（FACT）

`org.gnome.desktop.interface enable-animations = true`。

量法：APP 的 X 服务器自己每 5 s 打一行
`LorieNative: N frames in 5.0 seconds = X FPS`（`present copies offloaded to GPU` 那行）。
按 Super 触发概览动画，同时抓这个计数器：

| 时段 | 帧 | FPS |
|---|---|---|
| 静止桌面 | 0–5 | **0–1** |
| 概览动画（前 5 s） | 96 | **19.2** |
| 概览动画（后 5 s） | 217 | **43.4** |

**INFERENCE**：静止时 0 FPS 是正常的（没有内容变化就不 present）；19–43 FPS 说明动画真的在
跑，而且是这套直连呈现链自己在推帧，不是"卡成 PPT"。这台机器上 shell 走 llvmpipe（CPU），
所以 43 FPS 已经是上限附近；把 shell 挪到 zink/PanVK 之后应当更高，但那是另一件事，
本轮没做。

对比口径：以前 proot 那一版"进桌面/切窗口很卡"，卡的是整条呈现（回读+SHM 拷贝），
现在这条链在同样 2400x1080 上能稳定推 40 帧左右 —— 但两者没有同机同时刻的对照数据，
所以只能说到这（JUDGMENT，不是结论）。

## 3 · 音频：通，但要先关掉 PulseAudio 的 SHM（FACT + 已修）

### 3.1 现场：默认状态下所有 pulse 客户端都连不上

```
$ pactl info
Received memfd ID registration request over a pipe that does not support memfds
Received SHM frame on a socket where SHM is disabled.
Connection failure: Connection terminated
```

客户端日志（`PULSE_LOG=4`）显示协议版本一致（remote 35 / local 35），死在**共享内存协商**：
memfd 注册请求被当成"管道不支持 memfd"，随后服务器直接断连。

**INFERENCE（未证）**：这条链上 `memfd_create` 的 fd 经 `SCM_RIGHTS` 传到对端时丢了、
或者 path-shim 的 `memfd_create` 拦截与 pulse 的期望不一致。真正修它要动 fd 传递，
本轮先止血。

### 3.2 止血：客户端关 SHM

写 `/etc/pulse/client.conf.d/99-husk-no-shm.conf`（Debian/Ubuntu 的 pulse 会读 drop-in）：

```
enable-shm = no
```

之后**默认配置**（不再需要 PULSE_CLIENTCONFIG）：

```
$ pactl info
Server Name: pulseaudio
Server Version: 16.1
Default Sink: doro_android
Default Source: doro_android.monitor

$ pactl list sinks short
0  doro_android  module-pipe-sink.c  s16le 2ch 44100Hz  IDLE

$ paplay /usr/share/sounds/freedesktop/stereo/complete.oga ; echo $?
0
```

### 3.3 端到端确认：真的到 Android 了（FACT）

喂 8 s 连续 PCM 的同时抓 Android 的播放配置：

```
AudioPlaybackConfiguration piid:607 deviceId:3 type:android.media.AudioTrack
  u/pid:10265/29526 state:started
  attr:AudioAttributes: usage=USAGE_MEDIA content=CONTENT_TYPE_MUSIC
```

`uid=10265` = 本应用，`pid=29526` = 会话的 sh 父进程，`state:started`。链路即
**guest PulseAudio → module-pipe-sink(`/run/doro-audio/pulse-output.pcm`) → Rust `audio-relay` →
AAudio AudioTrack**，和 82–87 记的 VA-API 那条并列，是"中转"类接口（不是直连 `/dev/snd`，
guest 里本来也没有声卡节点）。

已落地：`tools/guest-config/pulse-client-no-shm.conf` + `install-husk.sh` 的 2b-5 段自动写入。

## 4 · 顺手修的两处小坑

- `husk-ding-fallback.sh` 现在**拒绝相对路径的 loader**：脚本会 `cd` 进 rootfs，相对路径会
  让每个探针静默失败，然后白等 90 s 再什么都不起（本轮就这么踩了一次）。传绝对路径时实测
  0.06 s 返回（X 在、DING 在 → 直接 no-op，不会重复起 DING）。
- `rootfs/root/Desktop` 是 777 → DING 拒绝"从桌面启动文件"；`husk-session.sh` 里补了 `chmod 755`（192 §6）。

## 5 · 结论

| 问题 | 答案 |
|---|---|
| Dock 里有多少个 | 配了 7，渲染 4（+垃圾桶）；缺的 3 是没装的包；列表来自 Ubuntu 的 `10_gnome-shell.gschema.override`，不是我们设的 |
| 有动画吗 | 有，`enable-animations=true`；概览动画实测 19.2 → 43.4 FPS（llvmpipe，2400x1080） |
| 音频呢 | 通：PulseAudio 16.1 → pipe-sink → Rust relay → AAudio，`AudioTrack state:started`；前置条件是 `enable-shm=no`（已随安装器落地） |
