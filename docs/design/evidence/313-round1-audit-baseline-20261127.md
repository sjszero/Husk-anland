# 313 · 第 1 轮审计基线（2026-11-27）

状态：**可复现清单已落库**。本文件把第 1 轮的四类审计结果固定下来；它不替代真机证据，
也不把源码存在写成运行已经通过。所有运行结论以文内 evidence 编号和复现命令为准。

## 0 · 审计快照

以下命令在仓库根目录执行：

```sh
git status --porcelain
git rev-parse --short HEAD
git describe --tags --exact-match HEAD
cat VERSION
git tag --points-at HEAD
```

本文件对应的快照是：

```text
HEAD       a41f022
VERSION    0.0.36
tag        v0.0.36
worktree   clean（本文件提交前除外）
```

审计范围：README、STATUS、LIMITATIONS、COMPATIBILITY、RESULTS、架构文档、CI、Makefile、
缺陷账本、设备测试入口和已有 evidence。没有在本轮重新跑真机，因此标记为 `UNVERIFIED` 的
条目仍不能用本次文档审计升级成 `FACT`。

## A · 已确认缺陷

### A-01 · 空壳 APK

- 证据：`app/app/build.gradle.kts:26-46`；`tools/ci/audit-apk-contents.sh:1-80`；
  `docs/design/evidence/293-round2-engineering-gates-20261126.md`。
- 修前：干净 clone 只执行 Gradle 会成功，但 `lib/arm64-v8a/libld.so`、`libhuskapp.so`、
  `libhuskguard.so` 三块都可能不在 APK 中。
- 复现：删除构建产物后运行 `./gradlew assembleDebug`，再执行
  `unzip -l app/app/build/outputs/apk/debug/*.apk | grep 'lib/arm64-v8a/'`；修前计数为 0。
- 修法：`preBuild` 依赖 `huskVerifyNativePieces`，CI 构建 APK 后再执行内容审计。
- 负向测试：删掉任一 `.so` 或对空壳 APK 运行 `sh tools/ci/audit-apk-contents.sh apk`，必须退出非 0。
- 门禁：Gradle `preBuild` + CI `apk`；状态 **FIXED**。

### A-02 · `make check` 的检查范围曾经远小于声明

- 证据：`Makefile:26-57`；`.github/workflows/ci.yml:44-83`；evidence 293 §2–§3。
- 修前：手写的 crate 列表只检查 12/39 个 Rust crate；CI 的测试范围也不足，注入到未列出的
  crate 的类型错误不会让检查变红。
- 复现（修前）：在 `runtime/crates/audio-relay` 注入一个 E0308，然后运行旧版
  `make check`；旧列表返回 0，`cargo check --workspace` 返回非 0。
- 修法：`make check` 使用 workspace 检查；CI 增加 workspace check、clippy 和可运行的 host 单测。
- 负向测试：在任意白名单外 crate 注入类型错误，`make check` 与 CI `check` 必须红。
- 门禁：`make check` / CI `check`、`clippy`、`test`；状态 **FIXED**。

### A-03 · 第 1 轮四类清单未落库

- 证据：`docs/design/defect-ledger.md:17-26`（H5）。
- 修前：四类审计结果只在会话里，无法让后续开发者按同一基线复查。
- 复现：`rg -n "第 1 轮|四张清单|A 已确认" docs`；修前没有独立基线文件。
- 修法：本文件提供 A/B/C/D 四张清单，并让每条带源码位置、复现、修法、负向测试和门禁归属。
- 负向测试：删除本文件或删除任一类清单后，文档链接审计和人工基线检查必须能发现缺失。
- 门禁：仓库文档审计；状态 **FIXED（本次）**。

### A-04 · libc 门禁版本钉子曾经指向不存在的产物

- 证据：`.github/workflows/ci.yml:268-287`；evidence 293 §8。
- 修前：下载 `2.39-0ubuntu8.12`，期望一个没有任何产物对应的哈希 `69ceef37`，门禁从第一天
  就是红的。
- 复现：按旧 workflow 的 URL 下载 deb，分别对 `8.5` 和 `8.12` 做 `dpkg-deb -x` 后计算
  patch 产物 sha256；结果为 `8.5 → 19554f27...`、`8.12 → 2ed8771f...`。
- 修法：钉子固定到真实发行字节 `8.5 / 19554f27...`，并把 8.12 是否升级另立为 H8 的第二半。
- 负向测试：把 workflow 中的 deb 版本或 `WANT_OUT` 改回旧值，libc 作业必须失败。
- 门禁：CI `libc`；状态 **FIXED（钉子） / FIXED（8.12 升版验收，evidence 315）**。

### A-05 · 假 DRM/V4L2 fd 身份识别不精确

- 证据：`runtime/crates/fd-identity/src/lib.rs:62-165`；
  `runtime/crates/drm-preload/src/lib.rs:186-203`；evidence 294 §1。
- 修前：DRM preload 用 FD 路径子串 `mali`，普通文件 `/tmp/malicious-data`、`mali-cache`
  可能被误送进 DRM 模拟；V4L2 和 path-shim 的识别规则也不完全一致。
- 复现：`cargo test -p husk-fd-identity` 的负向样例；旧实现应把 `malicious-data` 判成 DRM。
- 修法：只接受精确的 `/dev/mali0`、`/memfd:husk-drm`、规范 V4L2 memfd，并处理
  `readlink` 的 `" (deleted)"` 后缀。
- 负向测试：普通路径、错误后缀、越界 V4L2 编号、另一种 memfd 必须返回 `None`。
- 门禁：`cargo test -p husk-fd-identity` + fd-identity 设备探针；状态 **FIXED**。

### A-06 · 整块 `glReadPixels` 行距错误（已更正：探针常量错配）

- 证据：`docs/design/evidence/162-glreadpixels-bulk-readback-broken.md:5-34`；
  evidence 289 §结果；evidence 299 §0；`tools/device-tests/probes/readback-suite.py`。
- 更正：`GL_PACK_ALIGNMENT` 是 `0x0D05`，`GL_PACK_ROW_LENGTH` 是 `0x0D02`；旧探针把两者
  写反，实际把 `PACK_ROW_LENGTH` 设为 4，才产生“16 字节行距”。这不是 Mesa 缺陷。
- 复现：运行 `python3 tools/device-tests/probes/readback-suite.py`；修正后 T3 必须对整块读
  做精确断言，并检查金丝雀和 `glGetError()=0`。
- 修法：修正 `readback.py`、`readback-suite.py`、`gl-probe` 的常量，并把 T3 从负向改成正向。
- 负向测试：旧常量组合必须被状态断言抓到；整块读、PBO、宽度 1–65、金丝雀。
- 门禁：`readback-suite.py` + 截图/像素比对；状态 **FIXED（探针错误）**，证据 314。

## B · 源码级风险

### B-01 · Path Shim 规模过大且大量使用 unsafe/raw 系统调用

- 位置：`runtime/crates/path-shim/src/lib.rs`（当前 17,288 行）；`docs/ARCHITECTURE.md:15-26`。
- 风险：路径、FD、挂载模拟、进程视图和虚拟文件集中在一个 crate；局部修复很容易改变其他
  程序的 errno、生命周期或并发行为。
- 复现：`wc -l runtime/crates/path-shim/src/lib.rs`；
  `rg -n "unsafe fn|static mut|syscall\(" runtime/crates/path-shim/src/lib.rs`。
- 修法：按子系统拆模块并保留统一错误/锁策略；每次拆分先用差分测试证明行为不变。
- 负向测试：跨线程虚拟文件、并发 mount、符号链接、重入 open、错误 errno 回归。
- 门禁：`make check` + `tools/device-tests/01-functional-matrix.sh` + 压力探针。

### B-02 · GOT 改写不是安全隔离边界

- 位置：`docs/LIMITATIONS.md:19-27`；`docs/ARCHITECTURE.md:25-40`、`77-94`；
  `README.md:101`。
- 风险：静态 ELF、直接 raw syscall、未经过 loader 的进程不会被 GOT interpose 覆盖；
  因此不能把 Husk 当恶意代码沙箱。
- 复现：静态链接探针直接调用 `openat`，经 loader 执行并观察它是否访问宿主路径；
  另用 `readelf -l` 区分静态与动态 ELF。
- 修法：保持“兼容运行环境”边界；若需要强隔离，另立 namespace/VM 威胁模型。
- 负向测试：静态 ELF / raw syscall 必须被文档和测试标为不受控，不能错误宣称已完成隔离。
- 门禁：安全边界文档审计 + `SECURITY.md`；状态 **永久约束**。

### B-03 · X11 应用域没有可用 render node，窗口内容仍回读

- 位置：`docs/LIMITATIONS.md:63-78`；`docs/COMPATIBILITY.md:45-48`；
  `docs/design/hardware-interface-map.md:84-97`；
  `docs/design/evidence/290-dri3-presentation-cost-20261127.md`。
- 风险：X server 跑在 App 内时，`/dev/dri/renderD128` 的权限和 `untrusted_app` 边界使 DRI3
  不可达；窗口内容进入 X server 仍需 GPU 回读 + SHM 拷贝，约 30.27 ms/帧 @2400×1080。
- 复现：检查 `stat /dev/dri/renderD128` 与 X server 的 FD 权限，再跑 evidence 290 的呈现计时。
- 修法：不能靠 path-shim 单独修；需要改 X server 侧能力和真实 dma-buf 所有权模型，
  或明确保留回读代价。
- 负向测试：窗口尺寸边界、回读目标行距、金丝雀、X server 无 render node 时的失败路径。
- 门禁：X11 呈现审计 + readback/成本探针；状态 **UNVERIFIED（零拷贝）**。

### B-04 · 符号链接与路径解析仍有语义压力面

- 位置：`runtime/crates/runtime-core/src/lib.rs:297-323`；
  `tools/device-tests/path-probe.rs:119-165`；`docs/design/evidence/286-*`。
- 风险：Linux 允许约 40 次符号链接跟随，且小栈线程不能因路径解析而耗尽栈；并发替换、
  dirfd 越界、绝对符号链接和 `readlink` 竞态都可能改变 errno 或访问目标。
- 复现：`cargo` 构建 `path-probe`，在设备上跑 16/17/40/41 层以及 64 KiB 小栈线程。
- 修法：保持迭代 resolver 和固定大小缓冲；对每个入口统一路径/只读检查；加入 dirfd 和竞态负向测试。
- 负向测试：16、17、40、41 层；绝对链接；`..`/dirfd 越界；并发 rename/unlink。
- 门禁：`path-probe` + `05-mount-audit` + 门禁 A；状态 **部分已验证，竞态仍 UNVERIFIED**。

### B-05 · `hasCode=false` 阻塞需要 Java 回调的硬件能力

- 位置：`app/app/src/main/AndroidManifest.xml:8-12`；
  `docs/design/hardware-interface-map.md:99-104`。
- 风险：定位、USB、蓝牙需要 `LocationManager`、`Intent` 或广播接收器；当前没有 Java/dex
  垫片，不能仅靠 Rust NDK 把所有能力都直连。
- 复现：用不带 `adb install -g` 的普通安装启动定位/USB/蓝牙探针，观察权限或回调缺失。
- 修法：要么增加极小 Java 垫片并定义生命周期，要么明确只支持 last-known 等功能子集；
  NFC/指纹继续归为不需要。
- 负向测试：拒绝权限、无 Activity、后台回调、USB 首次授权和蓝牙关闭状态。
- 门禁：A-hw + 普通安装权限验收；状态 **来源风险已确认，完整链路 UNVERIFIED**。

## C · 文档与证据冲突

### C-01 · `make -j4` 的 5,194 ms 与 3,509 ms 样本没有在同一处说明

- 位置：`docs/COMPATIBILITY.md:22`；`docs/BENCHMARKS.md:74`、`110`；
  evidence 155 §1。
- 冲突：COMPATIBILITY 写 `5 194 ms`，evidence 155 单独运行写 `3 509 ms`；BENCHMARKS 已解释
  这是 5 轮轮转中位数与另一次单次样本的差异，但兼容性表只写了前一个。
- 复现：`rg -n "5194|5 194|3509|3 509" docs README.md`。
- 修法：在兼容性表脚注中明确“5194 为轮转中位数，3509 是单次样本”，不要只留一个无上下文数字。
- 负向测试：文档链接/数值审计应能发现同一性能项缺少样本口径。
- 门禁：文档审计；状态 **OPEN（文案口径）**。

### C-02 · P11 账本状态与最终 807 包树证据不一致

- 位置：`docs/design/defect-ledger.md:53`；evidence 303、308、310。
- 冲突：账本仍把旧 `862 installed / 92 .list / dpkg -C 1722` 写成 P11 OPEN；但当前最终树
  记录为 `807 installed / 807 .list`，且门禁 A/Tree Integrity 通过。两边可能不是同一棵树，
  不能直接覆盖历史，也不能继续把旧状态当成当前现状。
- 复现：`rg -n "807|862|P11" docs/design/defect-ledger.md docs/design/evidence/303* docs/design/evidence/308*`。
- 修法：增量记录“旧树失败、最终树重建成功”的证据链；P11 只能在明确是同一棵树的前提下关闭，
  否则拆成 P11a（旧树）和 P11b（最终树）。
- 负向测试：对最终树执行 `dpkg -C`、`.list` 计数、Tree Integrity；不能拿旧树输出冒充。
- 门禁：`make dist` + 真机安装 + Tree Integrity；状态 **OPEN（账本证据对齐）**。

### C-03 · X11/桌面状态在 README、STATUS、RESULTS 之间容易被读成不同结论

- 位置：`README.md:128-140`；`docs/STATUS.md:114-125`；`docs/RESULTS.md:74-79`；
  `docs/LIMITATIONS.md:29-41`；`docs/design/evidence/312-*`。
- 冲突：历史 X11 窗口结果确实来自 LinuxHub 2.0 的宿主 X server；Husk 运行资产不含
  `Xorg`/`Xvfb`，独立 GNOME/X11 仍为 UNVERIFIED。若读者只看到 GPU 窗口历史数据，容易误读为
  Husk 已交付桌面。
- 复现：并列阅读上述段落；再执行 evidence 312 的 `ls`/`find` 静态检查。
- 修法：所有 GPU 窗口数字前加“宿主显示载体”限定；把独立桌面结论唯一指向 evidence 312。
- 负向测试：文档审计搜索 `Xorg`、`Xvfb`、`UNVERIFIED`、`零拷贝`，每个能力只允许一个状态口径。
- 门禁：文档链接 + 能力状态表审计；状态 **OPEN（统一措辞）**。

### C-04 · 第 5 轮读回账本把 C1 标成 OUT-OF-SCOPE，但原始目标要求修公共读回

- 位置：`docs/design/defect-ledger.md:56`；evidence 162、289、299；原始目标第 5 轮。
- 更正：旧账本把探针常量错配误判成 Mesa 缺陷。正确常量为
  `GL_PACK_ALIGNMENT=0x0D05`、`GL_PACK_ROW_LENGTH=0x0D02`；修正后整块读应通过。
- 复现：运行修正后的 `readback-suite.py`；T3 现在必须是正向断言，不能再用“希望它坏”掩盖错误。
- 修法：修正探针常量，补状态和整块读断言；不再把逐行读取当成 Mesa 缺陷的兜底。
- 负向测试：故意换回旧常量，T3 和状态检查必须失败。
- 门禁：readback-suite + 截图路径；状态 **FIXED（探针错误）**，证据 314。

## D · 未实现、未验证、环境跳过

### D-01 · Husk 独立 X11 桌面启动

- 位置：`docs/design/evidence/312-x11-host-boundary-20261127.md:9-20`、`44-54`；
  `docs/RESULTS.md:74-79`。
- 当前：最终树没有 `Xorg`/`Xvfb`；独立 GNOME/X11 启动为 **UNVERIFIED**。
- 复现：按 evidence 312 执行 `ls files/rootfs-full/usr/bin/Xvfb ...` 和 `find ... -type s`。
- 修法：由宿主 App 提供 X server/输入/显示，或把自建 X11 资产纳入发行并单独验收；不能借旧树。
- 负向测试：干净安装后从零启动桌面，检查 X socket、首帧、输入、主题、动画和退出。
- 门禁：未来“独立 X11 桌面”门禁；状态 **UNVERIFIED**。

### D-02 · Adreno / KGSL 真机

- 位置：`docs/design/hardware-interface-map.md:20`；`docs/RESULTS.md:59`；
  `docs/LIMITATIONS.md:52-58`。
- 当前：只有源码/主机侧测试，没有 Adreno 真机证据；状态 **UNKNOWN**。
- 复现：在 Adreno 设备上跑 GPU audit、GL/Vulkan、离屏和 X11 窗口探针。
- 修法：补 KGSL/Turnip 适配和真机驱动选择，不以 Mali 参数冒充通用。
- 负向测试：无 `/dev/mali0`、有 KGSL 节点、驱动不匹配、软件回退。
- 门禁：A-hw GPU audit；状态 **UNVERIFIED（无设备）**。

### D-03 · 16 KiB / 64 KiB 页真机

- 位置：`docs/ARCHITECTURE.md:94`；`docs/page-size.md`；`docs/LIMITATIONS.md:153`。
- 当前：主机静态检查和 QEMU 不能替代目标手机；真机 **UNKNOWN**。
- 复现：在 16K/64K 页设备安装 loader 和 APK，运行动态链接、GPU、相机、音频探针。
- 修法：保持 PT_LOAD 同余检查，同时补 App `.so` 和 guest ELF 的页大小验收。
- 负向测试：错误页大小 ELF、4K/16K/64K 混装、loader 启动失败路径。
- 门禁：跨页大小设备门禁；状态 **UNVERIFIED（无设备）**。

### D-04 · 窗口内容到 X server 的零拷贝

- 位置：`docs/LIMITATIONS.md:63-78`；`docs/design/evidence/290-*`。
- 当前：应用侧 AHB→SurfaceControl 段有零拷贝证据；X server 入口仍回读 + SHM 拷贝。
- 复现：用 2400×1080 窗口负载测 GPU 回读和 X server 传递，不能只看应用侧 ring。
- 修法：处理 X server 侧 render node/权限和 dma-buf 所有权；若不可行，必须继续公布代价。
- 负向测试：无 render node、跨进程 buffer、尺寸边界、sync fence。
- 门禁：呈现链性能门禁；状态 **UNVERIFIED**。

### D-05 · 定位 / USB / 蓝牙完整链路

- 位置：`docs/design/hardware-interface-map.md:28-32`、`99-104`；
  `docs/STATUS.md:68`。
- 当前：归入“权限或厂商限制”，需要 Java 垫片或功能子集；完整链路 **UNVERIFIED**。
- 复现：普通安装（不带 `-g`）启动对应探针，覆盖首次授权、拒绝、后台回调。
- 修法：加最小 Java 类承接 Activity/Intent/广播，或明确只支持 last-known 等子集。
- 负向测试：权限拒绝、USB 拔出、蓝牙关闭、Activity 退到后台。
- 门禁：A-hw 扩展；状态 **UNVERIFIED**。

### D-06 · `iio-sensor-proxy` D-Bus 消费端

- 位置：`docs/design/hardware-interface-map.md:25`；evidence 302。
- 当前：NDK→IIO sysfs 五类默认传感器已验证；`iio-sensor-proxy` 未安装，D-Bus 输出未验。
- 复现：在 guest 安装并启动 `iio-sensor-proxy`，用 `gdbus`/`monitor-sensor` 读取。
- 修法：补包、systemd/用户服务启动和 IIO 设备命名契约。
- 负向测试：无传感器、事件停止、D-Bus 未启动、权限拒绝。
- 门禁：传感器 D-Bus 门禁；状态 **UNKNOWN**。

### D-07 · 相机 DMABUF、V4L2 controls、跨进程假 fd

- 位置：`docs/STATUS.md:62`；`docs/design/hardware-interface-map.md:21`；
  `docs/RESULTS.md:85-86`。
- 当前：前后摄 `read`/`readv`/mmap、拍照、录像已验证；DMABUF/controls 和跨进程假 fd
  仍为 **UNVERIFIED**。
- 复现：向 V4L2 facade 申请 DMABUF、设置 controls，并通过 SCM_RIGHTS 传 fd。
- 修法：补 buffer ownership、sync fence、权限传递和明确 `EOPNOTSUPP`，禁止伪成功。
- 负向测试：无 buffer、错误 format、跨进程关闭顺序、权限拒绝。
- 门禁：A-hw 相机扩展；状态 **UNVERIFIED**。

### D-08 · 整块 `glReadPixels` 真实修复

- 位置：evidence 289 §UNKNOWN/UNVERIFIED；evidence 299 §边界；C-04。
- 当前：旧探针的 16 字节行距是常量错配，不是运行时缺陷；修正后以 evidence 314 的真机结果为准。
- 复现：设备上执行修正后的 `readback-suite.py`；整块读、金丝雀和错误码必须同时通过。
- 修法：修正 `readback.py`、`readback-suite.py`、`gl-probe` 的常量，并把 T3 改成正向断言。
- 负向测试：故意换回旧常量时，T3 必须红；宽度边界、PBO、金丝雀仍必须绿。
- 门禁：读回门禁 + 截图/录屏；状态 **FIXED（探针错误，evidence 314 真机日志已归档）**。

## 结论

- 本轮已经把审计基线落成可复现文件，但 **不宣布第 1 轮之外的功能完成**。
- A/B/C/D 里的 `FIXED` 只对应该条证据；C-01、C-02、C-04 和 D 类仍未完成。
- 下一步优先顺序：先补 A-06/C-04 的真实 Mesa 读回修复；再处理 D-01 独立 X11；随后按门禁重跑
  并统一 P11、H8/8.12 的证据链。没有通过运行门禁前不得升级 VERSION 或发布新资产。
