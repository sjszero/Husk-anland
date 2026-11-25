# 236 · 虚构身份与身份登记表的原子发布

日期：2026-11-26 02:10（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页。
对象：`path-shim` 的 `FAKE_ROOT`、`FAKE_UID/GID`、跨进程身份登记 `IDENT_PUB` 和诊断计数
`IDENT_NOTES`。

## 1 · 结论

| 项 | 修前 | 修后 | 证据等级 |
|---|---|---|---|
| `FAKE_ROOT` | `static mut u8`，首次调用多线程读写 | `AtomicU8` | FACT，读码 |
| `FAKE_UID/GID` | 两个 `static mut u32`，读写没有原子发布 | 一个 `AtomicU64` 打包 uid/gid，CAS 更新 | FACT，读码 + 真机功能探针 |
| `IDENT_PUB_UID/GID` | 两个 `static mut u32`，发布与去重不是一体 | 一个 `AtomicU64` + 发布锁 | FACT，读码 |
| `IDENT_NOTES` | `static mut u32` 的 `+= 1` | CAS 预留到 4096 条上限 | FACT，读码 |
| 并发 `setuid`/`setgid` 的极端交错 | 可能让读者看到一对从未一起发布过的 uid/gid，且构成数据竞争（UB） | uid/gid 作为一个 64 位值原子更新 | INFERENCE；未构造旧代码的稳定复现 |

`setuid` 和 `setgid` 在真实 Linux 里本来就是两个系统调用，不能承诺一对调用整体原子；
这里修的是 Husk 自己“对外回答身份”的共享状态。它不应该比调用方提交的顺序更混乱。

## 2 · 真机功能回归

用当前 loader 运行 Rust 探针，依次验证 `getuid/getgid`、`setuid/seteuid`、
`setgid/setegid`、`setresuid/setresgid` 以及恢复 root：

```text
RESULT PASS uid=0 gid=0 after-restore
```

同一 loader 下普通 guest 身份仍为：

```text
uid=0(root) gid=0(root) groups=0(root)
```

这只能证明改造没有破坏单线程语义；并发 setuid 场景没有用可控竞态检测器复现，按
INFERENCE / UNKNOWN 记录。

## 3 · 实现边界

- `FAKE_ID` 用低 32 位存 uid、高 32 位存 gid；更新走 CAS，不把两个值拆成两次 store。
- `ident_sync()` 在同一把短锁里完成“判断是否变化 → 写登记文件 → 发布新值”，避免两个线程
  一个写了文件、另一个最后覆盖本地状态，导致后续跳过必要更新。
- `IDENT_NOTES` 的预算仍保留 4096 条，达到上限后不再写日志。
- 这些改动只在 path-shim 进程内；宿主真实 uid 仍由 Android 内核决定，不会获得权限。
