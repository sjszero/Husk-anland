# 246 · stdio `fclose` 后的硬链接复制后备刷新

日期：2026-11-26 10:40–10:44（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页 /
arm64。测试对象是 `com.linuxhub.myth.husk` 的 Ubuntu 24.04 guest，只使用临时 loader 文件，
没有替换 2.0 的正式 loader，也没有修改 LinuxHub 2.0 源码。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| 应用层 `fopen` → `fputs` → `fclose` 写入能否刷新 emulated hard-link 目标 | 能，探针输出 `content=AAABBB` | FACT，真机 |
| 完整门禁 A | `PASS=38 FAIL=0` | FACT，真机 |
| coverage-probe | 全部 PASS，包含新增 `HARDLINK_STDIO` | FACT，真机 |
| coverage-sweep | `SKIP`：rootfs 没有 `strace`，不冒充通过 | FACT |
| link count、共享 inode 是否恢复 | 否；内核仍拒绝 app 数据目录里的 `link(2)` | FACT |
| 绕过导出 `fclose` 的 glibc 内部关闭路径 | 不保证刷新；仍是 UNKNOWN 边界 | UNKNOWN |

## 2 · 问题与根因

Android app 数据目录里的 `link(2)` 返回 `EACCES`，Husk 用“复制目标文件 + 登记源 inode”模拟。
登记后源文件继续写入时，已有的 `close(2)` 钩子会调用 `alias_refresh()`，把源内容重新复制到
别名路径。

旧实现漏掉了 stdio 的常见路径：程序调用 `fopen` / `fputs` / `fclose` 时，glibc 的 `fclose`
内部走 `__close_nocancel`，不会经过我们导出的 `close(2)`。因此：

```sh
printf AAA > a
ln a b
printf BBB >> a
cat b          # 旧结果：AAA，缺少 BBB
```

这不是“硬链接完全没有模拟”，而是“复制已经发生，但最后一次 stdio 写入没有触发刷新”。

## 3 · 修复

在 `runtime/crates/path-shim/src/lib.rs` 导出并 interpose `fclose()`：

1. 先调用 libc 的真实 `fflush(stream)`，确保 stdio 缓冲区已经写进源 fd；
2. 用 `fileno(stream)` 取得源 fd；
3. 调用 `alias_refresh(fd)`，把最新的源内容同步到已登记的别名路径；
4. 最后通过 link map 解析出的真实 `fclose` 关闭流，保持 libc 原有返回值和 errno 语义。

真实 `fclose` 用现有的 `boot::nr_resolve_name_skip()` 解析，跳过 shim 自己导出的同名函数，避免递归。
这只改变 stdio 关闭后的别名同步，不改路径解析、挂载、GPU 或 X11。

## 4 · 回归探针

`tools/probe/coverage-probe.c` 新增用例：

```text
mkdir /root/.p-hardlink
fopen("/root/.p-hardlink/a", "w") -> fputs("AAA") -> fclose
link("/root/.p-hardlink/a", "/root/.p-hardlink/b")
fopen("/root/.p-hardlink/a", "a") -> fputs("BBB") -> fclose
读取 b
```

新 loader 真机输出：

```text
HARDLINK_STDIO link_rc=0 link_errno=0 content=AAABBB
ok   硬链接复制后备跟随 stdio fclose
--- coverage-probe: PASS
```

完整门禁 A：

```text
PASS=38 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: SKIP  (rootfs 里没有 strace；装法见 docs/design/evidence/216)
```

## 5 · 构建与资产

本轮 loader 与发行包：

```text
loader size=804352
loader sha256=d38ac630e94198c4efc7eb8d2656df0cac21c9357084a9ddcbfea531efb9c6f8

dist/husk-arm64.tar.gz size=7127621
sha256=a59ae8dff80e02eccf681a4b6f81869163da41a7c6fc8ade964f901c73e28377

dist/SHA256SUMS size=84
sha256=ed32a0eb0bd51ec0545a13933bf342884180ba6b220e87474ca774822cfc5bed
```

`make check` 通过：fmt、目标架构 `cargo check`、主机侧单测、Clippy、69 个 shell 脚本、
271 个 Markdown 链接和 `VERSION=0.0.21` 一致性检查。`make test` 仍是 host-only 目标，
macOS 的 `cc`/`ld` 不支持 aarch64 Linux 链接参数；该结果照实记录，不写成通过。

## 6 · 边界

- 这不是内核级硬链接；`st_nlink`、共享 inode 和跨进程即时共享仍然不成立。
- 只覆盖程序正常调用导出 `fclose()` 的 stdio 路径。若某个 glibc 内部调用连 `fclose` 导出入口都绕过，
  该路径仍未验证。
- 本次没有改 2.0 源码，没有把临时 loader 留在集成树里，也没有改变 X11 路线。
