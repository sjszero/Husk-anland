# 变更记录

## 2026-11-23 — 长期记忆归档：目标 v13、120 条参考清单、代理规则

- 新增 `docs/PROJECT-MEMORY.md`：把产品边界、硬约束、GPU/显示/视频/音频路线、
  2.0 替换方案、操作纪律和下一次接手顺序固化成文件，不依赖聊天上下文。
- 新增 `docs/design/reference-request-20261122.md`：用户提供的 12 类、120 个参考条目逐字归档；
  `references.md` 记录真实路径、可用性、已拉取 commit 和借鉴结论。
- 目标升到 **v13**：evidence 45 证伪了“Ubuntu panfrost + DRM 语义层是唯一实现”的假设；
  当前主动 GPU 路线改为 **panvk kbase fork（glibc）+ Zink**，DRM 语义层保留为后备。
- 记录代理规则：GitHub 不通时临时使用 `127.0.0.1:7897`，用完立即 `unset`。
- README / STATUS / 设计索引同步最新状态，避免旧结论继续误导后续工作。

## 2026-11-23 00:40 — kbase 完成通道：证伪一条、钉死三条

- **evidence 40**（新增）：`gpu-probe` 加 12 个模式（`ddkver`/`badptr`/`stride_probe`/`fence0-6`/`cr`/`sevt`/`sevt2`/`sevtjit`）。
- **FACT**：内核 kbase 自报 `K:r32p1-00bet5(GPL)`，与厂商 `libGLES_mali.so` 同代。
- **FACT**：atom 72 字节、`stride` 必须 72（56 → EINVAL）；坏指针 → EFAULT，证明数组被 `copy_from_user`。
- **FACT（推翻旧结论）**：`STREAM_CREATE` 的 timeline fd **不是完成信号** ——
  任何被接受的 submit（含 `core_req=0`、含本该阻塞的 `SOFT_EVENT_WAIT`）都让它 POLLIN，
  `read()` 返回 EINVAL。evidence 38 §6 的"可 poll = 完成通道"作废。
  本轮开头我自己报的 `fence1 PASS` 是假阳性，已被 `fence0` 对照打掉。
- **FACT**：软作业在本机不产生效果（`SOFT_EVENT_SET` 写显存 0 次，6 种偏移 + compat 双写 + 先 JIT init 全试过）；
  内核也不把 sync_file fd 写回 `base_fence`。
- 下一步：拿 r32p1 的 `base_jd_atom_v2` 定义（MTK GPL 内核源），或改用"GPU 写显存"作为唯一完成口径。

## 2026-11-22 23:20 — 目标 v10：视频补全

- **evidence 39**：把「guest 能不能直连硬解」钉死。`/dev/video0-4` 是 `0660 media:system` 的
  相机采集节点（不是编解码器），`untrusted_app` 打不开；厂商解码 `.so` 带 bionic DT_NEEDED，
  glibc guest 内加载不了 → **硬解只能在 bionic 侧调 NDK `AMediaCodec`**。
- 本机硬解器（取自 `/vendor/etc/media_codecs_c2.xml`）：`c2.mtk.{avc,hevc,vp9,av1}.decoder`
  及 mpeg4/h263/mpeg2/vc1/heif；厂商自报 1080p 帧率 avc 113–248 / hevc 125–276 / vp9 114–251 fps（未实测）。
- 目标升到 **v10**：总目标从「GPU / 显示 / 音频」改为「GPU / 显示 / **视频** / 音频」四件同级；
  第 4 条视频从一句口号扩成可验收规格（独立 socketpair + 共享内存环 → app 内 Rust 解码线程 →
  MediaCodec → AHardwareBuffer → MEM_IMPORT 回 guest；验收含 1080p30 H.264/HEVC 十分钟不掉帧、
  硬解 vs 软解 CPU 对比、AV 同步 <40ms、Chromium `<video>`）。
- 新增 `docs/goal-objective.txt`（粘贴块纯文本版），`docs/GOALS.md` 附三锚点表补 4 行视频事实。

## 2026-11-22 深夜 — 生命周期收口 + SoC 直连打通第一段

### 运行时壳（app-rs）
- NativeActivity 回调补齐：`onStop` / `onSaveInstanceState` / `onLowMemory` / `onContentRectChanged`。
- 修 4 个真 bug：BACK 被吞、`AKEYCODE_BACK` 变成 match 通配、onDestroy 不 teardown、重建后不再起会话。
- 修 2 个实测泄漏：`Term` 缺 `Drop`（每次重生漏 1 个 ptmx + 1 个 pty.log）、`Gfx` 缺 `eglTerminate`
  （每次旋转漏 sync_file/ashmem）。`session:` 日志加 `fds=` 便于长跑观测。
- 旋转实测：`67x58 -> 142x26 -> 67x58`，同一 pid，guest `stty size` 一致。
- 规格文档：`docs/design/lifecycle.md`（15 条不变量，逐条给代码位置与验证方法）。

### SoC 直连（GPU）
- **evidence 35**：从厂商 `libGLES_mali.so`（r32p1）反汇编取出 26 条真实 kbase 命令字。
  `JOB_SUBMIT = 0x40108002`（W,16），guest 内实测 **EINVAL 而非 ENOTTY** —— 旧结论"提交接口待查"是测量错误。
- **evidence 36**：上游 Mesa（25.2 与 main）**没有** kbase 后端（只有 panfrost_kmod / panthor_kmod），
  rootfs 也无 panvk ICD → 路线改为 Rust 侧 DRM 语义层。
- `mem_alloc(flags=0xf)` + `mmap(prot=3, off=VA)` 实测成功，CPU 写读一致；
  规则：`prot` 必须等于 flags 低两位，否则 EPERM。

### 目标与文档
- 目标重写为 v7/v8：已完成项（SIGSYS 长稳、生命周期、页大小）降为回归门禁；
  新增【D】段——用 Husk 替换 2.0 的 proot、一次性消除 proot 补丁、回滚包
  `LinuxHub-Myth-Prometheus.apk` sha256 `c4951876…9adc2b7`。
- README / RESULTS / FUTURE-WORK / ENGINEERING 按实测刷新：分清 PLE110 与 RMX3357 两台设备，
  撤销"厂商内核节点一律不可直连"的错误结论。

## 2026-11-22 夜 — 壳应用改为纯 Rust

- **去掉 Java 与 C**：APK 改为声明 `android.app.NativeActivity`，入口是 `app-rs/` 里的
  `ANativeActivity_onCreate`。Java Activity、JNI 外壳、`pty.c` 全部删除。
- **壳应用现在包含**：PTY 与 `fork`/`execve`（直接跑 guest loader）、EGL + GLES2 渲染、
  运行时字体光栅化（`fontdue`，字模取自系统等宽字体）、ANSI 子集解析、光标块、
  生命周期（surface 销毁/重建、窗口尺寸→`TIOCSWINSZ`、失焦不画光标、会话随进程存活、
  子进程退出后下一次点击自动重启）。
- **单元测试**：`app-rs` 的终端解析与屏幕缓冲有 9 个主机侧测试（`cargo test`）。
- **构建工具**：Python 参考实现删除（逐字节对拍已在 evidence 22/23 记录，可从 v0.1.0 取回）；
  loader 重构后重新构建，sha256 仍为 `1600a850…1cbee`。
- **基准**：新增 `runtime/crates/guest-bench`（Rust 版客舱基准）；`bench.c` 保留为无预编译
  二进制时的兜底。
- 语言构成：Rust 5,681 行 / 汇编 271 行；Java 0、Python 0。


## 2026-11-22 — v0.1 原型（一天内完成）

**结论：在无 root、无虚拟化、无 ptrace 的 `untrusted_app` 域里跑通了 bash + coreutils +
完整 gcc 13.3 / make 工具链。**

### 运行时
- 路径 shim 从 `LD_PRELOAD` 下沉进 loader：成为 `ld.so` 的一个额外 `R|E` `PT_LOAD`，
  由 `_dl_init` 钩子完成符号介入（76 个名字的 GOT 槽）。运行时**零 syscall 拦截**。
- `set_robust_list` / `rseq` 两处 `svc` 构建期 `nop`：app seccomp 政策会把它们判成
  `SECCOMP_RET_TRAP`，且 glibc 在 loader 内部就调用它们。
- 起始 cwd 改为 `/`；`..` 在 rootfs 边界截断；符号链接逃逸测试 0 escapes。
- `execve` 认出动态解释器后改走同一个 loader，子进程天然带 shim。

### 构建工具（全部 Rust，零依赖）
- `nr-tools scan-syscalls`：按 `mov x8,#N; svc #0` 模式定位调用点，读 `.gnu.version_d`
  自检 glibc 版本，四种异常一律拒绝写文件。
- `nr-tools embed`：把 shim 与服务表嵌进 `ld.so`，所有重定位在构建期烘好。
- `nr-tools provision`：替代 sh+python 的工具链供应脚本。
- 三个工具都与原 Python / sh 实现**逐字节对拍**通过（evidence 22、23）。

### 验证
- 31/31 功能矩阵；11 项越界测试 0 escapes；gcc+make 全链 exit 0。
- 成本：`getpid` 196 ns、`stat` 单组件 5.4 us（+2.5 us）、`fork+execve+wait` 2.49 ms
  （同机原生 bionic 6.8–8.0 ms）。

### 已知缺口
- `execl/execle/execlp` 拦不到；`freopen/glob/nftw/dlmopen` 未覆盖；
  `posix_spawn` 用 `fork` 而非 `vfork`；16K 页内核未测。
