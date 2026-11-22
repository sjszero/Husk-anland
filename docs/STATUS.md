# 当前状态（2026-11-22）

## 能用

| 能力 | 状态 | 证据 |
|---|---|---|
| 普通应用权限下跑 glibc 用户态（`untrusted_app`） | 可用 | evidence 12, 14, 17 |
| 功能矩阵（bash + coreutils + 管道/重定向/子 shell） | 31 / 31 | evidence 17 |
| 越界与符号链接逃逸 | 0 escapes | evidence 18 |
| gcc 13.3 全链（`cc1`/`as`/`ld`）+ make | 通过 | evidence 16, 17 |
| 交互式会话（PTY 里跑 `bash -i`） | 可用 | evidence 25 |
| 设备可达性：`/dev/ptmx`、`/dev/kgsl-3d0` | 可打开 | evidence 24 |
| 壳应用（`app/`）单会话终端 | 可运行 | 本机实测 |

交互式会话的根因修复：glibc 2.35+ 启动时注册 `rseq`，被 Android seccomp 判为 SIGSYS，
发生时路径层还没来得及装处理器——启动器必须带 `GLIBC_TUNABLES=glibc.pthread.rseq=0`。
同时修掉一个真 bug：`fork()` 之后调用 JNI 是未定义行为，现在 argv/envp 在父进程准备好。

## 能用但粗糙（界面）

- 提示符（PS1）不显示：命令能执行，但 shell 不画提示符；
- 输入框需要点一下才出键盘；
- `TIOCSWINSZ` 尺寸变化、全屏程序（`vim`/`htop`）未验证；
- 会话内 `pwd` 是 `/root`，应改为 `/`；
- 切后台 / 息屏 / Surface 重建后的会话生命周期未处理；
- 应用代码里仍留着开发期的自检与 `ProcessBuilder` 对照，产品化时删除。

## 未开始

- rootfs 的导入、选择、删除（现在靠 `adb` 手工 tar）；
- 终端渲染（现在是最小 ANSI 清洗，不是完整终端）；
- 图形桥、音频桥（见 [FUTURE-WORK.md](FUTURE-WORK.md)）；
- 16K 页内核、第二台设备、长稳测试；
- 与 LinuxHub 应用的正式集成。
