# 142 · 跨进程的虚构所有权：`st_uid` 从"本进程的谎"变成"容器的答案"

日期：2026-11-24　设备：RMX3357（`UKHMMB8PYTGECMNV`），包 `com.linuxhub.myth.husk`，
App 自建树（`distributions/ubuntu-24.04/rootfs`，814 包）。

## 0 · 结论（FACT）

139 §3.2 / 141 §5 记的那个缺口被补上了：**rootfs 里的文件现在对 guest 一律报 root:root，
只有 guest 自己 chown/创建的才报成别人**，而且这个答案跨进程、跨 App 重启都成立。

```text
修之前（同一棵树，旧 loader 33dcd681…）
$ ls -l /etc/passwd /etc/sudo.conf /etc/sudoers /usr/bin/sudo
-rw-r--r--. 1 10265 husk10265 1259 /etc/passwd
-rw-r--r--. 1 10265 husk10265 4343 /etc/sudo.conf
-r--r-----. 1 10265 husk10265 1800 /etc/sudoers
10265:10265 /usr/bin/sudo          (stat -c %u:%g)

修之后（新 loader d210644d…）
-rw-r--r--. 1 root root 1259 /etc/passwd
-rw-r--r--. 1 root root 4343 /etc/sudo.conf
-r--r-----. 1 root root 1800 /etc/sudoers
-rwsr-xr-x. 1 root root 335120 /usr/bin/sudo
```

`sudo` 因此越过了所有权检查，卡在下一条（§3）：

```text
修之前  sudo: /etc/sudo.conf is owned by uid 10265, should be 0
修之后  sudo: PAM account management error: Socket operation on non-socket
        sudo: unable to send audit message: Socket operation on non-socket
        sudo: a password is required
```

## 1 · 为什么是"表"，而不是拦 stat 就报 0

拦 `stat` 直接回 0 只在"全部归 root"时成立，而容器的语义里**有一批文件不是 root 的**：
`apt` 用 `_apt` 下载、`su`/`login` 起的会话文件属于那个用户、包的 postinst 会把某些目录
chown 给 `messagebus`/`systemd-*`。对这些文件，后续的 root 进程必须看到"别人家的"，
否则 `ssh`、`git`、`sudo` 这类按 owner 做安全判断的程序会自相矛盾（把别人的文件当自己的，
或者把自己的当别人的）。

所以表是**例外表**：不登记就是 0:0（root 占绝大多数），登记的才是别人。
键用 inode（`dev`+`ino`）而不是路径：`rename` 与硬链接因此不需要任何记账，
`x.dpkg-new` 改名就位后 owner 自然跟着走。

## 2 · 实现（`runtime/crates/path-shim/src/lib.rs`）

- 文件 `/var/lib/husk/owners`，一行一条 `dev ino uid gid`；追加写（`O_APPEND` + 单次
  `write`），追加尾巴超过 1 MiB 时整表重写一次；每个 guest 进程首次用到时整读一遍并建
  内存哈希（开放寻址，装填率 3/4 自动翻倍）。**加载为空 = 全部 root**，所以新 rootfs
  不需要任何初始化。
- 写入点：`chown`/`lchown`/`fchownat`（含 `fchown` 的 `AT_EMPTY_PATH` 形式）、
  `openat` 的 `O_CREAT`/`O_TMPFILE`（且仅当"创建者不是 root、文件原本不存在"）、
  `mkdir`/`mkdirat`/`mknodat`/`symlink`/`symlinkat`；`unlink`/`unlinkat`/`rmdir` 在
  `st_nlink == 1` 时清掉记录（inode 复用不会继承旧 owner）。
- 读出点：`stat`/`lstat`/`fstatat`/`stat64`/`lstat64`/`fstatat64`/`statx`/`fstat`/`fstat64`。
  只改 `st_uid`/`st_gid`（`struct stat` 偏移 24/28，`struct statx` 偏移 20/24）。
- **只对 rootfs 的 `st_dev` 生效**：`/dev`、`/proc`、vendor 节点这些 passthrough 路径
  保持真实值，不会因为 inode 撞号被误认。
- 开关：`NR_OWNERS=0` 单独关掉这张表（`NR_FAKE_ROOT=0` 仍然一把关掉整个虚构），
  这样任何回归都能直接归因到它而不是"又改了一次运行时"。

## 3 · 顺带量出来的下一条墙（FACT，接 §0）

`sudo` 现在能加载、能过 owner 检查，再往下是 **PAM**：`Socket operation on non-socket`
（`ENOTSOCK`）来自 `pam_systemd` 之类要连 logind/systemd 的模块；`unable to send audit
message` 走的是 audit netlink。这两件都不在门禁 A 判据里，属"服务/桌面表面"，
记在这里，不假装 sudo 可用。

## 4 · 回归（同一棵 App 树，新 loader d210644d…）

```text
14-gate-a-acceptance.sh   PASS=34 FAIL=0（EXPECT_PKGS=814）
12-supervisor.sh          PASS=8  FAIL=0
```

第一次跑出 7 个 FAIL 是测试数据撞号（我手工建的 `t1` 占了 uid/gid 1500，验收脚本的
`nrtest` 也要 1500），删掉 `t1` 后全绿 —— 记为一次操作失误，不是回归。

跨进程与用户会话：

```text
$ (root) mkdir /var/tmp/ownt; (NR_FAKE_UID=1000) bash -c 'echo a > /var/tmp/ownt/a; mkdir /var/tmp/ownt/b; ln -s a /var/tmp/ownt/l'
$ (另一个进程) stat -c '%u:%g %n' …        → 1000:1000 a / b / l
$ chown 1500:1600 a → 1500:1600 a; chown root:root a → 0:0 a
$ rmdir b; touch b  → 0:0 b（删掉的 inode 记录被清掉）
$ useradd -u 1500 -g 1500 -m t1; husk-su t1 -c 'echo hi > ~/mine; echo hi > /tmp/t1file'
  → /home/t1、/home/t1/mine、/tmp/t1file 全部 1500:1500
```

`/var/lib/husk/owners` 在一棵 814 包的树上只有 **115 字节**；root 创建文件**一条都不写**
（root 就是默认值，先查后写，见 §2）。

## 5 · 代价（FACT）

76,331 个文件全量 stat（`find /usr/share -xdev`，三次各取中位）：

```text
表开  574 / 583 / 654 ms
表关  523 / 569 / 611 ms      （NR_OWNERS=0）
```

两次测量互相咬在噪声里（同一台机器、同一棵树，关表也会跑出 523–611），所以结论只到
"看不出可测量的退化"：每次 stat 多一次自旋锁 + 一次哈希探测，没有额外系统调用。

## 6 · 兜里多带的一件：`useradd -m` 的 home 归属

`husk-usermgr` 建 home 之后从来没 chown 过它，所以 `/home/t1` 报 0:0 —— 在真机上是
"用户自己的家目录属于 root"，`ssh`/`git` 会直接拒绝。现在 `create_home()` 收尾做一次
递归 `lchown`（走 guest 的 `lchown`，也就是上面这张表）。

## 7 · 指纹

```text
libld.so            d210644d3c99782a61c3d88d60d0697182167a1b969629596eff12c66411fe7c  (673,280 B)
libpath_shim.so     293a95bc3bd21d91d4eb715ab283a6aaa6f6c8adadac1bed675a1932747fbfbe  (277,808 B)
husk-usermgr        801b276b94638929799cdbc61efee1e04a8cf654929cb5e06c8fa61cbe9c83cf
libc-nop.so.6       32d4d3f9501a2c717cf2246a2df1a898f1361b49e0576614461d70fc94774b56  (未重建)
```

## 8 · 仍未做（UNKNOWN）

- PAM/audit 那条（§3）：sudo 真正可用还差服务表面，桌面段的 `SCM_CREDENTIALS`（139 §3.3）
  是同一个方向。
- `chmod` 不走表（本来就正确：权限位是真的写下去的），但 `st_gid` 的辅助组判定没有做：
  guest 进程只有一个主组（139 的 getgroups 即如此），多组场景未验证。
- 16K/64K 真机仍 UNKNOWN（本机只有 4 KiB），与既有记录一致。
