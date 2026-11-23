# 123 · `utimensat` 不再假成功：时间戳真的写进去了

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页
全部命令在 guest（app uid 10302）里跑，无 root、无 chroot、无 ptrace。

## 1. 问题（FACT）

`path-shim` 的 `utimens_target()` 里有一条 fake-root 捷径：

```rust
if fake_root_on() { return 0; }   // 假成功
```

它把**所有** `utimensat`/`utimes`/`futimens` 都当成"guest 没权限改，按用户命名空间的
做法报成功"。这条捷径是从 `chown` 那边照抄过来的，但两者的权限模型不同：

| 系统调用 | 改它需要特权吗 | 调用方会回读吗 | 假成功的后果 |
|---|---|---|---|
| `chown` | 需要 `CAP_CHOWN` | 一般不会 | 无害（正当中户命名空间仿真） |
| `utimensat` | **不需要**（进程自己拥有的文件） | **会**（`make`/`git`/`tar`/`rsync`/`touch -h`） | 文件时间停在旧值，增量构建/打包判定错误 |

`04-path-reliability.sh` 里的 `metadata-utime-nofollow` 检查一直 FAIL，就是这条捷径造成的：
跑完 `touch -h -d @1500000000 l` 再 `stat` 符号链接，拿到的还是"现在"。

## 2. 修法

删掉 `utimens_target()` 里的 fake-root 短路，让调用真正落到内核（路径已经过 shim
翻译、`AT_SYMLINK_NOFOLLOW` 的最终组件已按 `O_PATH|O_NOFOLLOW` 固定）。
`chown_target()` 的同款短路**保留**——`chown` 确实需要 `CAP_CHOWN`，那是正当仿真。

## 3. 上机验证（FACT）

在真机 guest 里：

| 检查 | 结果 |
|---|---|
| `tools/device-tests/04-path-reliability.sh` | `#checks=15 fail=0`（修前 `metadata-utime-nofollow` FAIL） |
| `touch -d @1600000000 f; stat -c %Y f` | `1600000000` |
| `touch -h -d @1500000000 l; stat -c %Y l`（`l` 是符号链接） | `1500000000` |
| `apt-get install --reinstall -y sl` | rc=0，dpkg 数据库 1194 包 |

loader 与 shim 同源重编（`tools/loader/ld-embedded.so` 460288 B，`SEG_ALIGN` 三段全 `0x10000`），
rootfs 里的 `libc.so.6` **不需要**重打——本次只改了 shim。
