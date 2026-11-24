# 150 · polkit 能叫出调用者的名字了（身份链三层修完两层）

日期 2026-11-24 · RMX3357 · `com.linuxhub.myth.husk` · loader
`1d32c50ff0c6a355ad531b0511d90c8c3fd2a2a4cf99c66beecf2f06936b3aa6`（738,816 B）

## 1 · 问题

`polkitd` 要"给人起名字"，它不信我们，直接问内核：

| 它问哪里 | 在假身份下原本答的是 |
|---|---|
| `getsockopt(SO_PEERCRED)`（accept 到的那个连接） | 真实 uid **10265**（Android 应用 uid，passwd 里没有） |
| `recvmsg` 收到的 `SCM_CREDENTIALS` | 同上 |
| `/proc/<pid>/status` 的 `Uid:`（`polkit_unix_process` 会**再读一遍**） | 同上 |

结果任何特权请求都变成
`org.freedesktop.PolicyKit1.Error.NotAuthorized: Only trusted callers (e.g. uid 0
or an action owner) can use CheckAuthorization() for subjects belonging to other
identities`。

**发送端伪造不了**：内核只接受真实的 `ucred`（这正是 evidence 147 §5 让 `sendmsg`
改写它的原因）。所以要补的是**接收端**——每个 guest 进程自己在
`NR_FAKE_UID`/`NR_FAKE_GID` 里带着它的"身份"，而 `/proc/<pid>/environ` 在同一沙箱内
可读，于是接收方可以问对方"你自己说你 uid 是多少"。

## 2 · 改动（都在 `path-shim`）

1. **`getsockopt(SO_PEERCRED)`**（新增 body patch，`boot.rs` 的 `BODY_PATCH` 93 → 94）：
   按 `ucred.pid` 读该进程 environ 里的 `NR_FAKE_UID/GID`，改写 uid/gid，**pid 保持真实**
   （pidfd、`/proc/<pid>` 查找都还要用）。
2. **`recvmsg`**：对收到的 `SOL_SOCKET/SCM_CREDENTIALS` 做同样的还原。
3. **`sendmsg` 不再把"转发来的身份"改写成我们自己**：`pid != getpid()` 的
   `SCM_CREDENTIALS`（dbus-daemon 转发客户端身份时就是这种）**整条丢掉**，让接收方回落到
   连接本身的 peer 凭据。以前一律改写成真实 uid，正是"每个调用者都变成 10265"的原因。
4. **`/proc/<pid>/status` 服务化**：对 guest 进程（environ 里有 `NR_FAKE_UID`）读真文件后
   把 `Uid:`/`Gid:` 两行改成虚构值再交给调用者；非 guest 进程原样透传。

## 3 · 实测（FACT）

```
$ python3 credtest.py            # 默认假 root
getuid=0 getgid=0
scm_credentials pid=1964 uid=0 gid=0
so_peercred     pid=1964 uid=0 gid=0

$ NR_FAKE_UID=1000 NR_FAKE_GID=1000 python3 credtest.py
getuid=1000 getgid=1000
scm_credentials pid=1965 uid=1000 gid=1000
so_peercred     pid=1965 uid=1000 gid=1000

$ grep -E "^(Uid|Gid)" /proc/<gnome-shell>/status
Uid:	1000	1000	1000	1000
Gid:	1000	1000	1000	1000
```

改完桌面照常起来、`gnome-shell` 在、僵尸 0。

## 4 · 还差的一层（FACT，未修）

```
$ pkcheck --action-id org.freedesktop.login1.power-off --process <gnome-shell pid>
Error checking for authorization org.freedesktop.login1.power-off:
  GDBus.Error:org.freedesktop.PolicyKit1.Error.NotAuthorized:
  Only trusted callers (e.g. uid 0 or an action owner) can use CheckAuthorization()
  for subjects belonging to other identities
```

即"**调用者**是否可信"这一层还没过。当前线索：polkit 判可信与否看的是调用者身份，
而调用者是经 **dbus-daemon 转发**的；我们已把转发凭据丢掉、让接收方回落到连接 peer，
于是 polkitd 看到的应该是 `dbus-daemon` 自己的身份——它现在到底被认成谁，还没量。
下一步二选一：

- 让会话总线把调用者身份按 `GetConnectionCredentials` 报成虚构身份（bus 侧也同样问题）；
- 或者接受"跨 subject 查询被拒"，改由 **policy 规则**放行（已随仓库给出
  `tools/guest-config/49-husk.rules`：root 与 uid 1000 = 管理员，因为本沙箱没有能成功的
  PAM 口令路径）。

## 5 · 复现

```sh
NDK=<ndk>/bin OUT=/tmp/loader sh tools/loader/build-embedded-loader.sh
adb push /tmp/loader/libld.so /data/local/tmp/libld.so
adb shell run-as com.linuxhub.myth.husk sh -c \
  'cd files/linux/husk && cp /data/local/tmp/libld.so new && chmod 755 new && mv -f new libld.so'
# 重启 App 让整个会话换到新 loader，然后：
adb shell run-as com.linuxhub.myth.husk sh files/linux/xe.sh bin/sh -c \
  'python3 /root/dt/credtest.py'          # 脚本见 §3
```
