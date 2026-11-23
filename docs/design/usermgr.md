# 账号管理：为什么不能直接用 shadow，以及我们自己写的边界

> 目标段落：A（用户与权限）。设备：RMX3357 / Android 13 / 4.19.191 / 4 KiB / `untrusted_app`。
> 状态：实现完成，上机验收见 evidence 111。

## 1. 问题（FACT，先测后用）

| 测的东西 | 结果 |
|---|---|
| `useradd -m -s /bin/bash husktest` | `useradd: /etc/passwd.21848: lock file already used (nlink: 1)` → 拒绝 |
| `groupadd -r huskgrp` | `groupadd: /etc/group.21946: lock file already used (nlink: 1)` → 拒绝 |
| `adduser --system --group ...`（perl） | 自身能写 passwd，但中途 exec `/sbin/groupadd -g -1 huskgrp` → 同一个锁错误 |
| 裸 `ln .t1 .t2`（toybox，没有 shim 参与） | `Permission denied` |

结论：不是我们的路径改写错了，是**内核对 app 私有目录里的 `link(2)` 直接拒绝**。
shadow 的 `lckpwdf()` 用"把 `/etc/passwd` 硬链接到锁名，然后要求 `st_nlink == 2`"来判定锁是否空闲，
于是它永远认为"有人在用"，`useradd`/`groupadd`/`usermod`/`userdel`/`groupdel` 全部不可用。
任何在 postinst 里建账号的包都装不上。

之前用 `tools/guest-config/guest-accounts.txt` 预置账号只是绕过：只覆盖我们**事先知道**的账号。

## 2. 读过什么（按目标第 0 条记录）

| 来源 | 版本 | 拿走了什么 | 为什么不采用/怎么用 |
|---|---|---|---|
| shadow（上游，Ubuntu 24.04 的 1:4.13） | 本体 | 文件格式（`passwd`/`shadow`/`group`/`gshadow` 字段语义）、`login.defs` 键、退出码、`!` 锁定约定、`chpasswd`/`passwd` 的选项面 | **行为规格**来源；不抄代码 |
| `lxc/lxc` | 无（只读设计） | "假 root"下 uid 映射的语义边界：`getuid()==0` 不等于 `CAP_SETUID` | 我们已有的 fake-root 就是这种语义；`su` 仍不可用，如实记平台限制 |
| `containers/youki` `f330e83` | 2026-11-23 | 其 rootfs/进程代码验证了一点：容器里 `/etc/passwd` 是**镜像内容**，运行时没有账号工具 | 说明"装包建账号"这条必须由运行时自己解决，youki 不提供 |
| Debian `adduser`（3.137，`/usr/sbin/adduser`） | 设备上的实际文件 | 它先选 UID/GID，再 exec `groupadd`/`useradd`；因此只要修好这两个，perl 版也能用 | 不重写 adduser（它是策略脚本，不是底层） |

没有可复用的现成 Rust 实现：`useradd` 类工具都是 libc + 文件编辑，没有 crate 覆盖 shadow 语义。

## 3. 设计

- 一个二进制 `husk-usermgr`，按 `argv[0]` 分派：`useradd groupadd usermod userdel groupdel chpasswd passwd`。
- 四份数据库文件统一在**一把 `flock(2)`**（`/etc/.pwd.lock`）下读改写；写用"临时文件 + rename"，模式保持
  `0644`/`0640`。
- 无法解析的行原样保留（这些文件与系统其余部分共享，丢行比留着更危险）。
- 默认值全部来自 `/etc/login.defs`：`UID_MIN/MAX`、`SYS_UID_MIN/MAX`、`GID_*`、`USERGROUPS_ENAB`、
  `CREATE_HOME`、`HOME_MODE`、`PASS_MAX_DAYS`、`ENCRYPT_METHOD`；缺省值与 shadow 一致（100-999 系统段）。
- 密码哈希交给系统 `crypt(3)`（dlopen `libcrypt.so.1`，`$6$`/`$5$`/`$1$`），不自己实现 SHA512-crypt：
  自实现只会增加"看起来对、验证不过"的风险，而 `login` 校验用的就是同一个函数。
- 失败顺序按 shadow：先建组、再建用户、最后建家目录；中途失败最多留下一个空组。

### 边界（明确不做）

- 不改 `/etc/subuid`、`/etc/subgid`（本环境没有 uid 映射）。
- 不实现 `useradd -K`（sysconfig 覆盖）的完整语义，只接受并忽略，避免 postinst 因未知参数失败。
- `su`/`passwd` 切换用户仍然不可用：没有 `CAP_SETUID`，与容器里"非特权用户"的边界一致，如实记录（不是本工具的问题）。
- 家目录所有者不做 `chown`：fake root 下 `chown` 是空操作，文件本来就在 app uid 下。

## 4. 验收（evidence 111）

1. `groupadd -r huskgrp` → rc=0，`getent group huskgrp` 有行
2. `useradd -m -s /bin/bash husktest` → rc=0，家目录存在，`getent passwd husktest` 正确
3. `adduser --system --group --home /nonexistent ... husksvc` → 端到端 rc=0
4. `echo 'husktest:secret' | chpasswd` → shadow 里是 `$6$...`，且 `crypt("secret", hash) == hash`
5. `userdel -r husktest` → passwd/shadow 行消失、家目录消失
6. `apt-get install -y <一个 postinst 建账号的包>` → rc=0 且 `getent passwd <账号>` 有行
