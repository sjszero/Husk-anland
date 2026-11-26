> **历史快照（已被 evidence 315 取代）**：本文的 105 槽/`69ceef37` 是旧转换结果；当前为 106 槽、101 个已打钩、8.12 输出 `2ed8771f`。

# 170 · libc 补丁链 Rust 化：从「跑完就算成功」到「每条断言都过」

日期：2026-11-24 22:50–23:05（CST）。主机 macOS（`nr-tools` 是主机侧工具，不交叉编译），
被测输入：`libc6_2.39-0ubuntu8.12_arm64.deb` 里的 stock `libc.so.6`
（sha256 `2544e7902ec1bd3a68b526b6af427c34d2a98e56b9dbf8395b4b498dd2112cd6`，1722920 字节）。

## 1 · 为什么换掉那个 shell 脚本

`tools/guest-config/install-libc-patch.sh` 跑得通，但它**没法断言**，四个缺口都是结构性的：

| # | 缺口 | 实测后果 |
|---|---|---|
| 1 | 从不检查 interposer 是否**拒绝**了某个名字（`libc-interpose` 只把拒绝项打进 stdout） | 105 个 hook 里**有 5 个从来没打上**，没人发现（§3） |
| 2 | 推回设备后只 `sha256sum` 打印，不比对 | 传输出错 / 半截文件也显示"成功" |
| 3 | 不和 glibc 版本绑定 | 换个 8.7 或 2.40 的 libc 也照打，断言全失效 |
| 4 | 不检查页对齐 | 只有在 4 KiB 设备上才验证过，16 KiB/64 KiB 内核拿不到任何保证 |

外加一个体验问题：对已经打过的 libc 再跑一次，`libc-interpose` 会死在
`expected exactly one PT_NOTE, found 0`（FACT），错误指向 PT_NOTE 而不是"你已经打过补丁了"。

## 2 · 新入口

```
nr-tools libc-patch <libc.so.6> <out.so.6> --table tools/loader/libc-interpose.txt
nr-tools libc-patch <已打补丁的 libc> x --table ... --assert-only
```

`make libc-patch`（设备往返：拉 → 打 → 推 → 备份 `.stock` → 刷新 `tools/loader/libc-nop.so.6`）、
`make libc-verify`（无设备：对仓库里的构件跑 `--assert-only`，可选 `STOCK=` 做字节一致性）。

断言按顺序硬失败：

1. 输入是 aarch64 ELF64、`ET_DYN`，`.gnu.version_d` 里的 glibc 版本 == `--version`（默认 2.39）；
2. 输入还留着 `clone3` 站点（== `--clone3-sites`），且**没有** slot 页 —— 已经打过的一律拒绝，
   提示改用 `--assert-only` 或换 `.stock`；
3. interpose 之后，slot 页里"填上的名字集合"必须等于 **表 − `libc-interpose.expected`**，
   名字与相对顺序都对；多一个拒绝、少一个拒绝都失败；
4. 打完之后 `clone3` 站点为 0、`set_robust_list` 为 0（被改写）、`rseq` 数量不变
   （它靠 `GLIBC_TUNABLES` 关，调用方会看返回值）；
5. 每个 `PT_LOAD` 在 4 KiB / 16 KiB / 64 KiB 下都满足 `p_offset ≡ p_vaddr (mod page)`。

## 3 · 顺带查出来的真问题：5 个 hook 一直是空的

`tools/loader/libc-nop.so.6`（今天 15:46 的版本）里，slot 页有 105 个槽，**5 个是空的**：

| 槽 | 名字 | interposer 给的原因 |
|---:|---|---|
| 42 | `__readlink_chk` | something branches into the window |
| 43 | `__readlinkat_chk` | something branches into the window |
| 53 | `__realpath_chk` | something branches into the window |
| 68 | `__openat64_nocancel` | not exported by this libc（`.symtab` 已剥） |
| 77 | `__mq_open_2` | something branches into the window |

影响面（逐条判过，不夸大）：

- **应用层不受影响**：这 5 个名字在 shim 的 GOT 表里（`path-shim/src/boot.rs:1068/1069/1079/1091`）
  与实现里（`lib.rs:2554/8831/9431/9446`）都有，走 PLT 的调用仍然被改道。
- **libc 内部对这 5 个的直调仍然落到 host**：例如 fortified 构建下 libc 自己调
  `__realpath_chk`。这是**已知缺口**，不是"已经解决"。要修得让 interposer 能处理"有分支跳到
  窗口里"的入口（重定位那个分支，或改在另一个入口下钩），本版没做。
- `__openat64_nocancel` 在当前发行版的 libc 里根本不在 `.dynsym`，文件级打不了钩；
  它的非 `nocancel` 版本 `openat64` 是打上的。

所以这次不是"修好了这 5 个"，而是**让它们从"悄悄缺失"变成"写在 `libc-interpose.expected`
里、被集合比对钉住"**：以后 glibc 一换、多出一个拒绝，构建立刻失败。

## 4 · 字节一致性：新工具产出 == 仓库里的构件

```
$ shasum -a 256 /tmp/out3.so.6 tools/loader/libc-nop.so.6
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  /tmp/out3.so.6
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  tools/loader/libc-nop.so.6

$ STOCK=.../libc.so.6 make libc-verify
tools/loader/libc-nop.so.6: glibc 2.39, 100/105 hooks (5 declared refusals), clone3 0,
  page-congruent 4K/16K/64K, sha256 69ceef3759…f77c
runtime/target/libc-patch/verify.so.6: ok - glibc 2.39, 100/105 hooks, clone3 1->0,
  set_robust_list 2->0, 1722920 -> 1777664 bytes, sha256 69ceef3759…f77c
byte-identical to tools/loader/libc-nop.so.6
```

设备上跑的那份 libc 也就是这个哈希（`…69ceef37`），所以设备、仓库构件、新工具三者一致。

## 5 · 负样本（断言必须真的会失败）

| 输入 | 结果 |
|---|---|
| 已打补丁的 libc | `no clone3 site left - this libc is already patched. Pass the stock libc …, or use --assert-only` |
| `--version 2.38` 对 2.39 的 libc | `glibc 2.39, expected 2.38 (assertions below are version-tied)` |
| 非 ELF 文件 | `not an aarch64 ELF64` |
| stock libc + `--assert-only` | `no slot page (HUSK-SLOT-1 magic not found)` |
| 把 `expected` 裁到只剩 3 行 | `5 hook(s) missing that are not declared in libc-interpose.expected: __readlink_chk (slot 42), …` |

`make check` 全绿（fmt、aarch64 `cargo check`、clippy、190 个 md 链接、版本三元组），
`cargo test -p nr-tools` 里新增的 sha256 已知向量（含 1 MiB 多块路径）通过。

## 6 · 没做的

- 那 4 个"分支进入窗口"的 hook **没有**修（§3），只是被声明+钉住。
- `--assert-only` 里的 `rseq` 期望值来自常量（1，可用 `--rseq-sites` 改），没有从
  libc 版本自动推导。
- CI 里**还没跑** `make libc-verify`：workflow 文件尚未入库（需要 `workflow` scope）。
