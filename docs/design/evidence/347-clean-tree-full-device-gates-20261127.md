# 347 · 干净安装树完整真机门禁

日期：2026-11-27（CST）
设备：`UKHMMB8PYTGECMNV` / RMX3357 / Android 13 / MT6893 / Mali-G77 MC9 / 4 KiB 页
包名：`com.linuxhub.husk`
干净树：`files/clean-audio-344/root`
干净 loader：`files/clean-audio-344/bundle/libld.so`
版本：`0.0.36`（未 bump、未 tag、未 Release）

## 0 · 结论

- **FACT**：同一棵干净安装树已通过 A-core、coverage-probe、V4L2、DRM、fd identity、
  用户/组竞态、A-hw、音频、相机/GStreamer、GPU loader 默认值、readback 和
  300 秒多进程 GPU soak。
- **FACT**：A-hw 为 `PASS=15 FAIL=0`；音频为 `AUDIO_AUDIT PASS=4 FAIL=0`；
  相机 fixture 的 raw + GStreamer 640×480 / 1280×720 全部通过。
- **FACT**：GPU 默认值 `GPU_LOADER_DEFAULTS PASS=9 FAIL=0`，普通 exec 使用
  `zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))`，两个负向开关分别落到 llvmpipe。
- **FACT**：readback 内部 `PASS=19 FAIL=0`，外层 `READBACK_AUDIT PASS=6 FAIL=0`。
- **FACT**：4 个并发 job、300 秒、2400×1080 的 GPU soak 为
  `GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0`；四个 job 均 `errors=0`、fd=7、
  RSS delta=272 kB。
- **UNVERIFIED**：`coverage-sweep` 因干净树没有 `strace` 被明确跳过；
  宿主 X11/GNOME 全链路仍是独立门禁，不能用本轮结果代替。

## 1 · A-core 与路径/文件系统探针

```text
make gate-a DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
  ROOTFS=files/clean-audio-344/root \
  LD=files/clean-audio-344/bundle/libld.so
```

退出码 0：

```text
A-core: PASS=34 FAIL=0 SKIP=1
coverage-probe: PASS
coverage-sweep: SKIP（没有 strace）
dpkg-conffile-mode-probe: PASS
v4l2-ref-probe: PASS
drm-fd-probe: PASS
fd-identity-probe: PASS
supplementary-groups: PASS=1 FAIL=0
session-locale: PASS
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
session-guard: PASS
rootfd-close-range: stat_ok=8 range_ok=500
path-race: PASS，toggles=60000 operations=212050
```

## 2 · A-hw

测试时临时把 `files/rootfs-full` 指向干净树，单独传入干净 loader；测试结束已恢复原链接。

```text
  PASS  CAMERA granted=true
  PASS  RECORD_AUDIO granted=true
  PASS  ACCESS_COARSE_LOCATION granted=true
  PASS  app reports all hardware permissions granted
  PASS  captured 3 frame(s) through Camera2 (1382400 bytes each)
  PASS  app opened a Camera2 session
  PASS  five default sensors readable through three IIO views
  PASS  microphone returned 200704 PCM bytes (peak=444)
  PASS  gpsd delivered a last-known fix
  PASS  no crash records for the app process
PASS=15 FAIL=0
```

## 3 · 音频

```text
== guest audio provisioning ==
  NOTE  root=/|helper_modules=11|module_dirs=1
  PASS  audio-setup provisioned PulseAudio helper modules
  PASS  app created audio-out.sock
== direct AAudio bridge ==
  DONE status=PASS input_bytes=192000 frames=96000
  latency_ms_avg=125.163 latency_ms_max=128.000 xruns=1
== standard paplay -> PulseAudio -> Rust relay -> AAudio ==
  AUDIO_PLAYER_RC|paplay=0|relay=0
  relay|PASS|chunks=43|frames=87035|input_bytes=348140
  latency_ms_avg=104.680 latency_ms_max=106.667 xruns=1
AUDIO_AUDIT PASS=4 FAIL=0
```

**UNVERIFIED**：厂商仍返回 `perf=10 (NONE)`，不能宣称低延迟；本轮没有独立人耳听音。

## 4 · 相机与 GStreamer

```text
640x480  raw: frames=10 bytes=4608000  fps=77.31
1280x720 raw: frames=10 bytes=13824000 fps=38.39
GST_AUDIT|PASS|640x480|frames=3
GST_AUDIT|PASS|1280x720|frames=3
camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)
```

## 5 · GPU loader 默认值

```text
GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
NR_GPU_DEFAULT_OFF=1 -> llvmpipe
HUSK_GPU=llvmpipe      -> llvmpipe
```

## 6 · Readback

```text
TAG renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
PASS=19 FAIL=0
READBACK_AUDIT PASS=6 FAIL=0
```

包含行宽边界 1/2/3/4/5/7/8/15/16/17/63/64/65、4096/8192 字节 canary 和
240×135 截图逐字节一致。

## 7 · 300 秒 GPU soak

```text
4 jobs × 300 s × 2400×1080
p1..p4 renderer=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
p1..p4 errors=0
p1..p4 fd=7 stable
p1..p4 RSS delta=272 kB
GPU_SOAK_AUDIT PASS=19 FAIL=0 SKIP=0
```

## 8 · 边界

- **FACT**：本轮没有换 Wayland/合成器，GPU 测试走现有 loader 与 PanVK/Zink 路径。
- **UNVERIFIED**：X11 server 的呈现仍是回读 + SHM 拷贝，不能把 offscreen GPU 结果
  写成“X11 零拷贝已完成”。
- **UNVERIFIED**：Adreno/KGSL 仍没有本机真机验收。

