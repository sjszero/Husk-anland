# 297 · 第 4 轮：干净树上把门禁 A 跑成 34/0（+1 SKIP）

设备：`UKHMMB8PYTGECMNV`（RMX3357 / MT6893 / Android 13 / 4.19.191 / 4 KiB 页），
应用 `com.linuxhub.husk` vc36，`VERSION=0.0.36`（未发版）。
全部数字都是本轮在这台机器上重新跑出来的；旧文档里的数字不当作证据。

## 0. 一句话结论

上一版门禁 A 在干净树上 27/9，**9 条失败里 8 条不是运行时的错**：4 条是断言依赖了树里
没有的包（awk / getent / acl），1 条是版本号写死，2 条是 apt 在离线树上的正常行为，
1 条是宿主的 `EXPECT_PKGS_MIN` 根本没传进 guest。剩下 **1 条是真缺陷**：树里没有
`/usr/lib/locale/locale-archive`，所以"中文优先"在任何按包拼出来的树上都不生效。
修完之后同一类树是 34 PASS / 0 FAIL / 1 SKIP，并且顺手暴露出第二个真缺陷：
`libcrypto.so.3` 缺失让 coreutils 的 `sort` 直接死。

## 1. 门禁 A 的三个数字（同一台设备，同一天）

| 轮次 | 树 | 结果 | 说明 |
|---|---|---|---|
| 修前（evidence 296） | 61 包（旧 `BASE`） | `PASS=27 FAIL=9` | 9 条里 8 条是门禁自身的问题 |
| 门禁修完 | 66 包（`BASE` + libc-bin/locales/acl/mawk） | `PASS=35 FAIL=1` | 剩下 1 条：`apt-get -s rc=100` |
| 基础树补完 | 67 包（再加 libssl3t64） | `PASS=34 FAIL=0 SKIP=1` | `apt-cache policy` + `apt-get -s` 合成 1 条 SKIP（离线树没有包索引/没有可下的 deb） |

条数从 36 变成 35 是因为 apt 那两条在离线树上合并成一条明确的 SKIP，不是把断言删了。

## 2. 真缺陷 1：树里没有 locale archive，"中文优先"是空话

**现象（FACT）**：干净树上 `LC_ALL=zh_CN.UTF-8 locale charmap` 返回空，
`LC_ALL=zh_CN.UTF-8 date -d 2026-11-23 +%A` 返回 `Monday`。
安装日志同时写着 `wrote etc/profile.d/husk-locale.sh (no zh_CN.utf8 locale in this rootfs; LANG left alone)`。

**根因（FACT）**：`/usr/lib/locale/locale-archive` 是 `locales` 包的 **postinst** 生成的。
`nr-tools rootfs` 是把 .deb 直接解包进目录，postinst 从来不跑；任何"手工拼树"的路径
（包括 2.0 的 base tarball + debs）都一样。没有 archive，glibc 找不到 zh_CN，
`LANG=zh_CN.UTF-8` 静默回落到 C —— 这就是"一进系统全是英文"的来源。

**修法**：`install-husk.sh` 新增 `== locale ==` 一步，用**树自己的** `localedef` 在设备上
生成 `zh_CN.UTF-8` + `en_US.UTF-8`，然后写 `/etc/default/locale`。
不能在构建机上生成：`localedef` 是 aarch64 程序。

**顺序约束（FACT，很容易踩第二次）**：这一步必须排在 libc 还原（2d）**之后**。
放在 2d 之前时，树自带的（`nr-tools` 打过 slot 补丁的）libc 下 shim 的路径改写没有生效，
localedef 里每个 `/usr/...` 都落到宿主根上：

```
localedef failed for zh_CN.UTF-8 (rc=1)
cannot create temporary file: /usr/lib/locale/locale-archive.8oHzRw: No such file or directory
```

挪到 2d 之后，同一棵树同一命令 `rc=0`。所以代码里这一段写在 2d 和 2e 之间，并留了注释。

**修后（FACT）**：

```
== locale ==
   generated zh_CN.UTF-8 + en_US.UTF-8 (6222864 bytes)
   wrote etc/default/locale (zh_CN.UTF-8)
```

重跑门禁：

```
PASS  zh_CN charmap (UTF-8)
PASS  zh_CN weekday (星期一)
```

`localedef` 生成两个 locale 用时约 2.3 s（单次调用实测）。

## 3. 真缺陷 2：基础树缺 libcrypto.so.3，`sort` 直接死

**现象（FACT）**：66 包树上跑门禁 A 时，`env | grep ... | sort` 这一行打出
`sort: error while loading shared libraries: libcrypto.so.3: cannot open shared object file`，
于是"宿主路径泄漏"那条断言拿到的是空输入 —— 它 PASS 了，但 PASS 得没有意义。

**根因（FACT）**：noble 的 coreutils 里 `sort`（以及 `shuf`、`md5sum` 的随机源）链到 libcrypto，
而 `BASE` 里没有任何提供 `libcrypto.so.3` 的包。树的可用性因此是假的：`ls` 能跑，
`sort` 不能，任何 `... | sort` 的脚本静默返回空。

**修法**：`BASE` 加 `libssl3t64`。同时补上三个"看着可选、其实基础系统必备"的包并写明理由：
`libc-bin`（getent / localedef / iconv）、`locales`（/usr/share/i18n/*）、
`acl`（getfacl/setfacl）、`mawk`（Ubuntu 的 awk）。修后同一条管道输出正常：

```
  vars whose value points at the app dir: NR_LD NR_LIBRARY_PATH NR_ROOTFS
```

## 4. 门禁自身的 6 处假失败（不是运行时问题，但会把真问题淹掉）

| # | 假失败 | 根因 | 修法 |
|---|---|---|---|
| 1 | `shadow hash (got '')` | 断言用 `awk`，最小树没有 awk | 改成 `grep -m1 '^nrtest:' /etc/shadow` + `case *'$6$'*` |
| 2 | `group entry (got '')` | 断言用 `getent`（在 libc-bin 里） | 改成 `grep -m1 '^[^:]*:[^:]*:1500:' /etc/group \| cut -d: -f1` |
| 3 | `apt version (want '2.8.3' got '')` | 版本号写死，且取值也走了 awk | `re()` 正则 `^2\.8\.[0-9]+$`，纯 shell 取版本 |
| 4 | `installed packages (want >= 90 got 61)` | 宿主的 `EXPECT_PKGS_MIN` 没进 guest（guest 只读 `${EXPECT_PKGS_MIN:-90}`） | host → guest 显式传 `EXPECT_PKGS_MIN`；最小树传 60/66 |
| 5 | `getfacl still runs` / `policy knows acl` | 树里没装 acl | 装了才断言；没装 → `SKIP` 并写原因（不是 FAIL） |
| 6 | `apt-get -s rc (want '0' got '100')` | 离线树：`/var/lib/apt/lists` 只有 `partial/`，`ls -A` 判空被它骗过 | 按真的索引文件判（`*_Packages*` / `*InRelease*`）；rc≠0 且输出含 `cannot be downloaded` → SKIP |

`SKIP` 是新的第三态，汇总行现在是 `PASS=.. FAIL=.. SKIP=..`。SKIP 不是通过：
它意味着"这棵树上测不了"，换一棵有该包的树就必须变成 PASS 或 FAIL。

## 5. 顺带修掉的假警告：hard-link 校验

安装日志长期打印
`install-husk: warning - hard links missing; pass 2 did not go through the shim`，
而同一份日志上面明明列着 `-rwxr-xr-x. 1 501 root 2346 /usr/bin/uncompress`。

**根因（FACT）**：校验写成 `test -s /usr/bin/perl5.38.2 && test -s /usr/bin/uncompress`，
两条 `&&` 串在一起；没装 perl 的树上第一条就失败 → 整句判 no → 结论却是"pass 2 没走 shim"。

**修法**：逐个判，且 `perl5.38.2` 只在树里真有 `perl` 时才判；失败时把缺的文件名打出来。
修后：

```
== ok: hard links materialised (uncompress are real files) ==
```

## 6. 复现命令

```sh
# 在设备上拼一棵干净树（67 包，含 locales/libc-bin/acl/mawk/libssl3t64）
runtime/target/debug/nr-tools rootfs --out /tmp/husk-base3 --fresh \
  --cache /tmp/nr-rootfs/deb --tar /tmp/husk-base3.tar.gz

adb -s $DEV push /tmp/husk-base3.tar.gz /data/local/tmp/
adb -s $DEV shell 'run-as com.linuxhub.husk sh -c "cat /data/local/tmp/husk-base3.tar.gz > files/husk-base3.tar.gz"'
adb -s $DEV shell 'run-as com.linuxhub.husk sh -c "cd files/linux-clean && \
  TMPDIR=/data/data/com.linuxhub.husk/files/tmp sh install-husk.sh \
  --rootfs ../husk-base3.tar.gz --dest ../linux-t6/root --loader ./libld.so"'

DEV=$DEV PKG=com.linuxhub.husk ROOTFS=files/linux-t6/root LD=files/linux-clean/libld.so \
  EXPECT_PKGS_MIN=66 sh tools/device-tests/14-gate-a-acceptance.sh
```

## 7. 本轮明确没做的事（UNVERIFIED）

- 整棵 862 包桌面树**没有**重跑门禁 A → 不能声称桌面树 36/0。
- `dirfd-probe` 仍未构建 → `04-path-reliability.sh` 的 `relative-dirfd-contained` 仍是 UNVERIFIED。
- `localedef` 生成 archive 后**没有**在真实桌面会话里验证过中文界面（只验证了
  `locale charmap` / `%A` 两条，桌面会话属于第 6/8 轮）。
- 2.0 旧包 `com.linuxhub.myth.husk` 未改动、未复测。
