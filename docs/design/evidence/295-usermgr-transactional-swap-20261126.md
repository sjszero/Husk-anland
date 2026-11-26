# 295 · husk-usermgr：四个库文件的换入变成可回滚、可恢复

日期：2026-11-26（CST）。真机 RMX3357 / MT6893 / Android 13 / kernel 4.19.191。
版本 `VERSION=0.0.36`。

## 0 · 结论

| 编号 | 缺陷 | 状态 | 判据 |
|---|---|---|---|
| S7 | `save()` 先把四个库都写进临时文件（这一步原来就对），但**四次 rename 不是一次事务**：第三个失败时 `passwd`/`shadow` 已经是新的、`group` 还是旧的，报错也回不去；两次 rename 之间崩溃同样留下混合库 | **FIXED** | 单测 3 项（回滚 / 恢复 / 丢弃过期备份）；真机端到端 8/8 |

- **FACT**：`cargo test -p usermgr` → **6 passed**（含新增 3 项）。
- **FACT**：真机（guest 内 `/etc`，Android 文件系统 + app 的 SELinux 域）实测 8 条断言全 PASS：

```
--- A. 崩溃恢复：只剩 .husk-bak，目标缺失 ---
rc=0
[PASS] passwd 已从备份恢复
[PASS] 备份已消费
--- B. 正常加组 + 加用户 ---
rc=0
rc=0
[PASS] passwd 有 huskprobe
[PASS] group 有 huskprobe
[PASS] shadow 有 huskprobe
[PASS] shadow 权限 -rw-r-----
[PASS] 无备份残留
--- C. 清理并复原 ---
[PASS] huskprobe 已删除
```

跑完把 `/etc/{passwd,shadow,group,gshadow}` 用备份复原，`passwd` 仍是 18 行、`group` 38 行，
guest 树回到测试前的状态；临时装的 `husk-usermgr` 与测试脚本已删。

## 1 · 修法

`save()` 现在只做两段：先 `write_tmp` 全部暂存（失败就清干净），再调 `install_staged`。

`install_staged` 对每个文件：

1. 清掉可能存在的陈旧 `<name>.husk-bak`；
2. 目标存在就先 `rename(target -> target.husk-bak)`；
3. `rename(tmp -> target)`；
4. 任何一步失败 → 已换入的按**逆序**回滚（删新件、把备份换回来），未换入的临时文件删掉，返回错误；
5. 全部成功 → 删掉四个备份。

`Db::load()` 一开始调 `recover_backups_in(&[PASSWD, SHADOW, GROUP, GSHADOW])`：
备份在、目标不在 = 崩在两次 rename 之间 → 把备份换回去；两者都在 = 换入已完成、只是清理没跑 → 删备份。
调用方在 `lock_pwdb()` 之后才 `load()`，所以恢复动作不会和另一个 `husk-usermgr` 撞。

## 2 · 单测（新增 3 项）

| 测试 | 断言什么 |
|---|---|
| `install_staged_rolls_back_when_a_later_file_cannot_be_replaced` | 在第三个文件的备份位上放一个非空目录，使 `rename(target -> bak)` 失败；断言 `passwd`/`shadow` 恢复到**旧内容**、回滚后的备份不残留、失败点之后的临时文件被清掉 |
| `recover_restores_a_backup_left_by_a_crashed_swap` | 只有 `.husk-bak`、目标缺失 → 恢复出旧内容，备份被消费 |
| `recover_drops_a_stale_backup_when_the_target_exists` | 两者都在 → 保留已换入的新内容，删掉备份（不能把已完成的换入回滚掉） |

第一条是本轮的判据：旧实现下 `passwd` 会停在 `new-passwd`，只有回滚才能让它回到 `old-passwd`。

## 3 · 没做的

- **UNVERIFIED**：真机上没有注入"rename 失败"（要造一个不可替换的目标位），回滚路径只有单测证据。
  真机只验了崩溃恢复 + 正常路径。
- 这不是数据库事务：没有日志，`fsync` 只对 `/etc` 目录做了一次；崩在"删备份"和"目录 fsync"
  之间最坏结果是残留一个备份文件，下次 `load()` 会清掉，不影响内容一致性。
- 与另一个 `husk-usermgr` 的并发只靠 `flock(/etc/.pwd.lock)`；不走这个锁的编辑器（直接改 `/etc/passwd`）
  不在保护范围内。
