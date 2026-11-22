# Husk（壳）

**在无 root、不解 BL、不开虚拟化、不用 Termux 的普通 Android 应用权限下，
让完整的 Linux（glibc）用户态作为这个 app 进程本身直接跑在内核上。**

不加 ptrace，不加 seccomp supervisor，不加 FUSE，不加 namespace，不加虚拟机，
不在运行时翻译任何系统调用。

> 状态：**原型已跑通，未产品化**。2026-11-22 一天内在真机完成从"能不能跑"到
> "gcc + make 全链能编译并运行"的验证。私有仓库，暂不公开。

| 读什么 | 内容 |
|---|---|
| [架构](docs/ARCHITECTURE.md) | 分层、三个改动点、路径解析、为什么进 loader、已知缺口 |
| [实测结果](docs/RESULTS.md) | 31 项功能矩阵、越界、成本表、PRoot 对照、没测的项 |
| [快速开始](QUICKSTART.md) | 从零跑起来（20–30 分钟） |
| [方案（权威）](docs/native-runtime/06-sole-plan-t2t1.md) | 唯一路线、排除表、验收标准、剩余工作 |
| [证据](docs/native-runtime/evidence/) | evidence 01–23：设备原始输出，编号与方案引用一一对应 |
| [变更记录](CHANGELOG.md) | 今天做了什么、还有什么没做 |

---

## 1. 为什么叫"壳"，为什么不是容器

我们加在 guest 外面的唯一东西就是一层壳：

- **ELF 的壳**：改过的 guest `ld.so`，路径 shim 编译后直接嵌进它的第三个 `PT_LOAD`；
- **进程的壳**：app 进程就是 Linux 进程，内核直接执行 guest 代码，中间没有第二层。

没有 namespace、没有 mount、没有虚拟磁盘镜像，所以它不是容器，也不是虚拟机。
一个 guest 系统实例叫一个 husk：`husks/<distro>/`。

技术名保留 `LinuxHub Native Runtime (T2+T1)`，只出现在方案与证据里。

## 2. 两层，各干一件事

| 层 | 做什么 | 在哪做 | 运行时成本 |
|---|---|---|---|
| **T2** 路径 shim | 把 guest 视角的绝对路径映射到 app 私有目录下的 rootfs | 纯 Rust（`no_std`、零 `DT_NEEDED`、零外部 crate），嵌入 `ld.so`，由 `_dl_init` 钩子做符号介入 | 只有路径类调用过一次字符串处理；syscall 一条都不多 |
| **T1** 静态改写 | 入口桩、必须 `nop` 掉的 `svc` 调用点（`set_robust_list` / `rseq` 在 app 域被 `SECCOMP_RET_TRAP`）、将来的 `PT_INTERP` / `RPATH` | 构建期，`crates/nr-tools`（纯 Rust，零依赖） | 0 |

关键点：**T2 只处理路径，不处理 syscall 流。** 这是它和 PRoot 的本质差别。

## 3. 实测（PLE110 / OPPO，Android 16，kernel 6.6.118，PAGE_SIZE 4096）

域：`untrusted_app`，uid 10512；guest：Ubuntu 24.04 / glibc 2.39。

| 项目 | Husk | 对照 |
|---|---|---|
| `exec`（同条件） | **2.49–2.53 ms** | 原生 bionic 6.8–8.0 ms |
| `getpid` | **196 ns** | 内核原值 |
| `stat` 单路径组件 | **4.3–5.4 us** | — |
| `stat` 四组件 | **11–12 us** | — |
| PRoot（同 harness，另一台机） | 每条 syscall 一次 ptrace 往返：5000 次 stat **0.41–0.76 s** | Husk 同项 **0.06 s** |
| PRoot 在本机 | **连 guest ELF 都 exec 不了**（ENOENT→EACCES，四种配置都试过） | — |

## 4. 现在能做到什么

真机实测，全部有原始输出（`docs/native-runtime/evidence/`）：

- bash 交互、管道、子 shell、重定向、`stdin`
- `ls / find / readdir / pwd / cd / cp / mv / chmod / readlink / sort / sha256sum / glob`
- 符号链接（相对、绝对、`..`）正常解析，**且不能逃出 rootfs**
- 子进程继承环境与 cwd（起始 cwd = `/`）
- **gcc 13.3 + as + collect2 + ld.bfd**：编译 C 程序并运行
- **make**：并行构建、增量、clean
- 31 项功能矩阵 **31 PASS / 0 FAIL**；越界测试全过

## 5. 现在做不到什么（明写，不藏）

- `execl / execle / execlp` 拦不到：glibc 内部走私有 `execve`，没有 PLT 槽；稳定版 Rust 定义不了
  C 变参，汇编蹦床会被 `rustc` 的 cdylib 导出表本地化。两次尝试均失败并如实记录。
- `posix_spawn` 用 `fork`，不是 `vfork`（少一次优化，不是错误）。
- `freopen` / `glob` / `nftw` / `dlmopen` 未覆盖。
- 16K 页内核未测。
- rootfs 里没有 python3（247 个二进制），所以 guest 内也只能用 shell/C 工具链。
- **产品化未开始**：PTY/会话管理、与 LinuxHub 2.0 的集成、`fd→路径`缓存都还在待办。

## 6. 仓库结构

```
docs/native-runtime/          方案与证据
  06-sole-plan-t2t1.md        唯一权威方案（含验收清单与剩余工作）
  07-proot-comparison.md      与 PRoot 的同条件对照
  evidence/01–23              设备原始输出，编号与方案引用一一对应
linuxhub-runtime/             Rust workspace（可直接 cargo build）
  crates/path-shim            嵌入 loader 的路径 shim（no_std、零依赖）
  crates/runtime-core         shim 的宿主无关核心
  crates/capability-probe     设备能力探针
  crates/nr-tools             构建工具：scan-syscalls / embed / provision
tools/nr-loader-hook/         loader 手术现场：入口桩与 _dl_init 钩子（271 行汇编）、
                              三个 Python 参考实现（用于逐字节对拍）、build 脚本
tools/native-runtime-tests/   设备侧测试：31 项矩阵、越界、bench、gcc/make 验收、工具链供应
tools/native-runtime-probe-apk/ 探针 APK（Java，96 行）：从 untrusted_app 拉起 guest loader
binaries/                     改写后的 loader / libc 与原始文件——它们的 sha256 就是身份
```

## 7. 怎么复现

完整步骤见 [QUICKSTART](QUICKSTART.md)。最短路径：

```sh
export NDK=<ndk>/toolchains/llvm/prebuilt/<host>/bin
./tools/nr-loader-hook/build-embedded-loader.sh        # 改出带 shim 的 loader
./tools/native-runtime-tests/provision-toolchain.sh    # 装 gcc/make（纯 Rust）
cd tools/native-runtime-tests
DEV=<adb serial> ./run-guest.sh 01-functional-matrix.sh  # 31 PASS / 0 FAIL
DEV=<adb serial> ./run-guest.sh accept-gcc-make.sh       # gcc + make 全链
```

指纹（可用于校验你手上的是不是同一份产物）：

```
ld-embedded.so                1600a850985624b05c8b1c1f9cee5fe8185462da91d0de369dcb9ff30f21cbee
ld-linux-aarch64.so.1.orig    bd29514c1b45cc29b152011dd7c504e00dce240c19e4dd120196053873af9180
libc-nop.so.6                 18acad63e69559c974c0ae093333d9cfa661c18ba4ad04a48268d8d1405e1bcc
libpath_shim.so               73106ac673ea28c56f28cab56b6dd07b9c2415bb1cf801a8ab2f219906e16c1a
```

在本仓库里直接跑 `tools/nr-loader-hook/build-embedded-loader.sh`，产出的 loader
就是上面第一个 sha256——不是"理论上可以"，是刚在本机跑出来的。

## 8. 语言构成

| 语言 | 行数 | 说明 |
|---|---|---|
| Rust | ~4500 | 运行时本体 + 构建工具，全部 `no_std` / 零依赖 |
| 汇编 | 271 | `entry-stub.S` / `init-hook.S`：这段跑在任何 Rust 运行时存在之前，稳定版 Rust 定义不了 naked fn |
| C | 103 | `bench.c`：基准必须跑在 guest 里 |
| Shell | ~600 | 测试脚本——它们测的就是 bash，换掉等于换掉被测对象 |
| Java | 96 | 探针 APK |
| Python | 630 | 仅作**对拍参照**，构建默认走 Rust |
| glibc 二进制 | — | `ld.so`（+1 个 `PT_LOAD`、2 处 `nop`）、`libc.so.6`（2 处 `nop`），不重编译 |

## 9. 边界与来源

- 改写的对象是 Ubuntu 24.04 的 `ld.so` / `libc.so.6`（GPL/LGPL 衍生），`binaries/` 只用于校验产物身份，
  不随本仓库提供 rootfs；分发二进制时需自行满足相应的许可证义务。
- 本仓库是私有库：`docs/` 与测试脚本里含有本机绝对路径、设备序列号等环境信息，公开前需清理。
- 方案里所有结论都带标签：**FACT**（命令输出）/ **INFERENCE** / **UNKNOWN**；未实测不写 PASS。
