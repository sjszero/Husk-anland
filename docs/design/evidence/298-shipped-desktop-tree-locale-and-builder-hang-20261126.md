# 298 · 出厂桌面树：界面英文的真因，以及构建器静默挂死

设备：`UKHMMB8PYTGECMNV`（RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB 页），
`com.linuxhub.husk` vc36，`VERSION=0.0.36`（未发版）。
树：`files/linux/distributions/ubuntu-24.04/rootfs`（862 个包 installed，825 个 `/usr/bin` 条目）。

## 0. 结论

1. **"一进桌面就是英文"不是设置问题，是这棵树里根本没有 zh_CN**：有 413 个 `.mo`
   翻译文件，却没有 `/usr/lib/locale/locale-archive`，而 `/etc/default/locale` 写的是
   `LANG=C.UTF-8`。两处都补上之后，`locale charmap=UTF-8`、`%A=星期一`。
2. 这棵树还是**P1 修复之前**的 `nr-tools` 拼的：862 个包 installed 但只有 92 个
   `.list`，`dpkg -C` 报 1722 行。它不能当发行物，必须用当前 `nr-tools` 重拼（P11）。
3. 重拼过程中发现构建器会**静默挂死**：`http_get` 用 `TcpStream::connect((HOST, 80))`，
   DNS 解析没有任何超时，787 包跑到 123 个 deb 时既不报错也不退出（P13，已修）。

## 1. 门禁 A 在这棵桌面树上的三个数字（同一台设备，同一天）

| 状态 | 结果 | 失败项 |
|---|---|---|
| 修前 | `PASS=30 FAIL=4 SKIP=1` | `zh_CN charmap` / `zh_CN weekday` / `dpkg -C` ×2 |
| 树内补 locale 后 | `PASS=32 FAIL=2 SKIP=1` | 只剩 `dpkg -C` ×2（1722 行） |

SKIP 那条仍是"离线树没有包索引"（`/var/lib/apt/lists` 只有 `partial/`），与运行时无关。

## 2. locale：两个同时成立的原因

**FACT（修前）**：

```
$ ls rootfs/usr/lib/locale/        →  C.utf8          # 没有 locale-archive
$ cat rootfs/etc/default/locale    →  LANG=C.UTF-8    # 默认不是中文
$ ls rootfs/usr/share/locale-langpack/zh_CN/LC_MESSAGES | wc -l  →  413   # 翻译是齐的
```

翻译齐、locale 缺，于是 `LANG=zh_CN.UTF-8` 在 glibc 里查不到 zh_CN，gettext 不生效，
界面回落英文。`husk-session.sh` / `husk-desktop.sh` 的选择逻辑本身是对的
（"有 `usr/lib/locale/locale-archive` 就用 zh_CN"），是树里没有那个文件，所以它每次都
选了 `C.UTF-8`。

**修法（FACT）**：在设备上用树自己的 `localedef` 生成，再修默认值：

```
$ husk-exec ... usr/bin/localedef -i zh_CN -f UTF-8 zh_CN.UTF-8   # rc=0，约 2.3 s
$ husk-exec ... usr/bin/localedef -i en_US -f UTF-8 en_US.UTF-8   # rc=0
$ chmod 644 rootfs/usr/lib/locale/locale-archive                  # localedef 建出来是 0600
$ printf 'LANG=zh_CN.UTF-8\nLANGUAGE=zh_CN:zh\n' > rootfs/etc/default/locale
$ husk-exec ... usr/bin/env LC_ALL=zh_CN.UTF-8 usr/bin/locale charmap  → UTF-8
$ husk-exec ... usr/bin/env LC_ALL=zh_CN.UTF-8 usr/bin/date -d 2026-11-23 +%A → 星期一
```

这三步已经进了 `install-husk.sh` 的 `== locale ==`（见 evidence 297），所以**用当包重装的
树不会再缺**；缺的只是这棵**已经装好的**旧树，以及由旧路径生成的发行包。

## 3. dpkg 记录：这棵树不能当发行物

**FACT**：

```
$ dpkg -C | head -3
The following packages are missing the list control file in the
database, they need to be reinstalled:
 accountsservice      accountsservice (staged by nr-tools rootfs)
$ dpkg -C | wc -l        → 1722
$ ls /var/lib/dpkg/info/*.list | wc -l  → 92   （installed 包数 862）
```

`(staged by nr-tools rootfs)` 这行字就是证据：它是 P1（补 `.list`/`.md5sums`/`.conffiles`）
修好之前的构建器产物。P1 已经修了并在 67 包的干净树上验过（`dpkg -C` 0 行），但这棵 862
包的树是用旧二进制拼的，重拼一次才行——见 P11。

## 4. 构建器静默挂死（P13）

**FACT**：重拼 729 个包（解析出 787 个）时：

```
22:42  123 个 deb 就位，然后没有任何新文件、没有错误、没有退出
22:46  仍然 123 个；进程状态 Ss，CPU 0.0%，没有打开的 TCP 连接
```

根因：`http_get` 是 `TcpStream::connect((HOST, 80))`，而 `ToSocketAddrs` 的 DNS 解析
**没有超时**。解析器一旦不回，整个构建就永久停在那里，连"哪个包失败"都打印不出来。

修法（`runtime/crates/nr-tools/src/provision.rs`）：
- 先 `to_socket_addrs()` 解析，再 `connect_timeout(20s)`；
- 读超时 60 s、写超时 20 s；
- 整条请求重试 3 次（archive 中途断流是常态）；
- 下载后校验字节数等于索引里的 `Size`，短写直接 die，不写进缓存；
- 每 25 个新包打一行 `fetched N new (i/N total)`，让"慢"和"卡住"能区分开。

## 5. 这台设备本身的两个限制（不是 Husk 的缺陷，但会挡住验证）

**FACT**：

- `adb shell pm revoke com.linuxhub.husk android.permission.CAMERA` →
  `SecurityException: Neither user 2000 nor current process has
  android.permission.REVOKE_RUNTIME_PERMISSIONS`；`pm reset-permissions` 同样被拒。
  也就是说**没法用 adb 把权限打回"未询问"状态**。
- `pm install` 会拉起 OPPO 的"安装向导"（`com.oplus.appdetail/.InstallGuideActivity`），
  它把安装挂住直到有人在界面上点"继续安装"；该按钮不在可点击树里
  （`uiautomator` 只能拿到 `取消安装` / `搜索相关应用`），自动点击落空。

因此"不带 `-g` 全新安装 → 首启弹框 → 拒绝两次 → 设置页"这一条**本轮没有能重放**。
已提交的证据仍然只有 288（2026-11-26 那次真机记录），本轮记为 **UNVERIFIED**，
并把它列为"换一台不受该 ROM 限制的设备，或人工点一次安装向导"才能补的项。

## 6. 复现命令

```sh
# 树内补 locale（任何按包拼出来、没跑过 postinst 的树都适用）
adb shell "run-as com.linuxhub.husk sh files/linux/husk-exec2.sh \
  files/linux/distributions/ubuntu-24.04/rootfs files/linux/husk/libld.so \
  usr/bin/localedef -i zh_CN -f UTF-8 zh_CN.UTF-8"

# 门禁 A（桌面树）
DEV=UKHMMB8PYTGECMNV PKG=com.linuxhub.husk \
  ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs LD=files/linux/husk/libld.so \
  EXPECT_PKGS_MIN=700 sh tools/device-tests/14-gate-a-acceptance.sh
```

## 7. 未验证

- 桌面会话里**实际看到的**中文界面（只验到 `locale charmap` / `%A` 两条；会话级观察属第 8 轮）。
- 重拼后的完整桌面树（P11）尚未跑完/装回设备。
- 全新 `-g` 外安装的权限流程：见 §5，UNVERIFIED。
