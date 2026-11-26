# 283 · 第 3 轮：usermgr 四库暂存换入 + 只读挂载元数据现状复核

日期：2026-11-26（CST）。设备：无（本轮全部是源码与宿主单测，未连设备）。
对应缺陷编号：A-04（usermgr 四库非事务、fsync/chmod 失败被吞）。

## 1 · 修前

FACT，`runtime/crates/usermgr/src/main.rs`（提交 `19c6699` 时）：

```rust
fn save(&self) -> R<()> {
    write_atomic(&PASSWD, ...)?;
    write_atomic(&SHADOW, ...)?;
    write_atomic(&GROUP, ...)?;
    write_atomic(&GSHADOW, ...)?;
    Ok(())
}

fn write_atomic(path: &str, content: &str, mode: u32) -> R<()> {
    ...
    f.sync_all().ok();
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).ok();
    fs::rename(&tmp, path)...
}
```

三个问题，逐条：

1. `sync_all().ok()`：fsync 失败（EIO、ENOSPC 之后的内核回退）不报错，内容可能没落盘就 rename 了。
2. `set_permissions(...).ok()`：chmod 失败不报错。`/etc/shadow` 是 `0o640`，这一步失败意味着影子密码对同组可读，而调用者看到的是成功。
3. 四个库是四次独立 `rename`：中途崩溃或第三次失败，会留下 `passwd` 新的、`group` 旧的半套库。`getent group` 和 `useradd` 之后的行为都不可预测。

同样的 `.ok()` 还在 `create_home` 的 chmod 与 `/var/mail` spool 的 chmod 上。

## 2 · 修后

FACT，`runtime/crates/usermgr/src/main.rs`：

- `write_tmp()`：写 `.husk.tmp` → `sync_all()`（失败即 `Err`）→ `set_permissions()`（失败即 `Err`）。
- `save()`：先把四个库**全部**暂存；任何一个暂存失败就清掉已暂存的，返回错误，一个文件都不换入；
  全部成功后逐个 `rename`；`rename` 失败时清掉还没换入的暂存。
- 最后 `fsync_dir(PASSWD)`：对 `/etc` 目录 fsync，让目录项变更也落盘。
- `create_home` / `append_mail_spool` 的 chmod 从 `.ok()` 改成返回错误。

**这不是真事务**：没有 journal，四次 `rename` 之间仍然存在窗口。诚实的说法是：
窗口从「四次写入期间」缩到「四次 rename 之间」，并且**任何失败都会上报**，不再静默。
要彻底消除窗口需要 journal 或目录级原子替换（例如把四个文件放进一个目录再 symlink 切换），
那是另一个改动，且要改 `/etc` 的布局，不在本轮。

## 3 · 回归测试

FACT，宿主单测，`cargo test --manifest-path runtime/Cargo.toml -p usermgr`：

```
running 3 tests
test tests::write_tmp_reports_open_failure ... ok
test tests::staged_swap_lands_content_and_mode ... ok
test tests::staging_all_four_leaves_no_tmp_behind_on_success ... ok
test result: ok. 3 passed; 0 failed
```

三个测试分别钉住：暂存失败不落盘、内容与 `0o640` 模式都到位、成功后不留 `.husk.tmp`。
`make check` 与 CI 的 `test` job 现在都跑 `-p usermgr`。

## 4 · 未验证

- UNVERIFIED：没有在设备上跑 `useradd / usermod / userdel / groupadd` 全链路，也没有做
  「rename 中途失败后重启能否回到一致状态」的故障注入。宿主单测只覆盖了暂存层，
  没有覆盖 `save()` 里四个库的整组行为（那需要一个可注入的 `/etc`，属于设备侧测试）。
- UNVERIFIED：`fsync_dir` 在目标 rootfs 的 `/etc` 上是否被允许（设备上 `/etc` 是真实目录，
  不是符号链接），未实测。

## 5 · 顺带复核：只读挂载的元数据写

FACT：`docs/design/mount.md:61-64` 已经写明 `chmod/fchmodat`、`chown/fchownat/lchown`、
`utimensat/utimes/lutimes` 与 xattr 写都由 `ro_refuse_at` 在入口判定，代码里也确实有三处调用
（`path-shim/src/lib.rs` 的 `chmod_target`、chown、utimens 分支）。

所以「只读挂载只检查 openat 写入，chmod/chown/utimensat 没实施」这个结论是**过期的**，
不该再当缺陷报。真正剩下的缺口只有 mount.md 自己写的两条：

1. fd 形式的 `fchmod` / `fchown`（没有路径，无法判定挂载点）；
2. `renameat2(RENAME_EXCHANGE)`。

这两条留给后续轮次，且第 1 条如果要修，需要一张 fd → guest 路径的表，属于路径 shim 的
新能力，不在本轮范围。
