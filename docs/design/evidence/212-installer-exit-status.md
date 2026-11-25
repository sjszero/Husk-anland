# 212 · 安装器不再吞掉失败；`NR_GPU_DEFAULT_OFF` 按布尔值判断

日期：2026-11-25 20:35–21:30（CST）。设备 RMX3357。

## 1 · 管道把子进程的退出状态换掉了（FACT）

`sh` 的管道返回最后一条命令的状态。安装器里有三处把真正干活的命令接给了 `sed`：

| 位置 | 改前 | 后果 |
|---|---|---|
| 第二遍解包（pass 2） | `'tar -xf …' 2>&1 \| sed 's/^/   /'` | `tar` 失败时返回值是 `sed` 的 0 |
| 符号链接改写 | `husk-symlinks --root / --apply 2>&1 \| sed …` | 同上 |
| 符号链接校验 | `husk-symlinks --root / --verify --quiet 2>&1 \| sed … \|\| { … exit 71; }` | `\|\|` 挂在 `sed` 上，永远不触发 |

同一台机器上量的对照（toybox `sh`）：

```
if (echo one; exit 3) 2>&1 | sed 's/^/   /'; then echo "pipe=0"; fi   → pipe=0
if (echo one; exit 3) > /data/local/tmp/hk-l 2>&1; then … else echo "log=$?"; fi  → log=3
```

改法：三处都写日志文件，直接读被调命令的返回码，再把日志 `sed` 一遍打到屏幕上。
pass 2 非 0 → `exit 69`；符号链接 `--apply` 或 `--verify` 非 0 → `exit 71`。

## 2 · pass 1 的状态捕获在 `set -e` 下根本没走到（FACT，读代码 + 实测）

脚本第 28 行 `set -eu`，pass 1 是这么写的：

```sh
tar xzf "$ROOTFS_TAR" -C "$DEST" > "$OUT/.husk-pass1.log" 2>&1
rc=$?
```

`tar` 非 0 时 shell 在 `rc=$?` 之前就退出，所以后面那段"硬链接错误可以容忍、其它错误
`exit 68`"的判断只会在 `tar` 成功时执行，也就是永远进不去。改成
`if tar …; then rc=0; else rc=$?; fi`，三个分支（`.gz` 直通、其它压缩格式先解压、纯 tar）都改。

## 3 · 截断的树包（FACT）

把 1.3 GB 的 rootfs 包截到 100 MB 再装：

```
tar: tar: EOF: Illegal seek
install-husk: pass 1 failed (tar rc=1); this archive is not usable
rc=68
```

不再打印 `== done ==`，目标目录没有被当成可用树。

## 4 · `NR_GPU_DEFAULT_OFF` 只看存在（FACT）

`runtime/crates/path-shim/src/gpuenv.rs` 原来是
`if crate::env_lookup(b"NR_GPU_DEFAULT_OFF").is_some() { return; }`，于是
`NR_GPU_DEFAULT_OFF=0` 也关掉 GPU 注入。文档写的是 `=1` 关闭。

改成按布尔值判断：`1`/`true`/`yes`/`on` 关；`0`/`false`/`no`/`off`/空串都不关。

## 5 · 删掉没有调用者的入口桩（FACT）

`tools/loader/entry-stub.S` 在 `_start` 之前装 SIGSYS handler。两个 `svc` 调用点
（`set_robust_list` / `rseq`）改成构建期 `nop` 之后它没有调用者，
`build-embedded-loader.sh` 编译的是 `init-hook.S`，全仓库没有任何脚本引用它。
删掉源文件和构建产物，`docs/ARCHITECTURE.md` 与 `docs/assets/architecture.svg` 同步改。
SIGSYS 现在由 shim 在 `nr_boot()` 里装 handler 回答（evidence 159）。
