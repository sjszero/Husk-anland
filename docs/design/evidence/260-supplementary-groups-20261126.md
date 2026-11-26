# 260 · 补充组：从“只报主组”修到 `/etc/group` 和 `setgroups/initgroups` 可读写

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB。
被测物：`tools/loader/ld-embedded.so`（SHA-256 `f21663f1d9699faf1e18fbc2d90bc5bd68cf5fbfb5e496d765437f47aeab841e`），通过 `run-guest.sh` 进入 App 的真实 guest 根。

## 0 · 结论

FACT：修前，假用户进程的 `os.getgroups()` 只有 `[1500]`；`os.setgroups([4242])` 后仍是
`[1500]`；`os.initgroups("huskuser", 1500)` 后也没有 `4242`。这说明此前的实现只是
“返回成功”，没有真实的补充组状态。

FACT：修后，普通 session 会从 guest 自己的 `/etc/group` 自动得到 `[1500, 4242]`；
`setgroups([4242])` 得到 `[4242]`；`initgroups()` 恢复 `[1500, 4242]`；组权限目标
`/tmp/husk-group-audit/target`（`0:4242`，`0660`）的 `os.access(R_OK/W_OK)` 都是 true。

## 1 · 修前原始输出（FACT）

```text
== G-01 supplementary groups survive setgroups and grant group access ==
  GROUP_ENTRY huskgroup:x:4242:huskuser
  AFTER_START [1500]
  UID_GID 1500 1500
  ID_GROUPS 1500
  PY_GETGROUPS [1500]
  AFTER_SETGROUPS [1500]
Traceback ... AssertionError: setgroups did not retain the supplementary group
```

`GROUP_ENTRY` 已经证明 `/etc/group` 里有 `huskuser`，不是测试账号没建出来。
这次失败的原因是 shim 没有保留补充组，不是 `/etc/group` 数据错误。

## 2 · 修复内容

`runtime/crates/path-shim/src/lib.rs`：

- 增加进程内、带锁的 guest 组表，最多 32 个 gid；不再把 Android 的
  `input/log/adb/sdcard_rw/...` 原样泄漏给 guest。
- 首次使用时从 guest `/<rootfs>/etc/passwd` 找用户名，再读 `/<rootfs>/etc/group`
  中“主组或成员列表包含该用户”的条目；`NR_FAKE_UID/GID` session 不依赖额外启动参数。
- `getgroups()` 返回真实表；`setgroups()` 替换表；`initgroups()` 按用户名重建表。
- `access/faccessat` 的 group 位判断改为查补充组，而不是只比较主 gid。
- 组表读写用 `FAKE_GROUPS_LOCK` 串行化；`getgroups` 在缓冲区不够时按 POSIX 返回
  `EINVAL`，空列表不再对 null 指针调用 `from_raw_parts`。

## 3 · 修后原始输出（FACT）

```text
== G-01 supplementary groups survive setgroups and grant group access ==
  GROUP_ENTRY huskgroup:x:4242:huskuser
  AFTER_START [1500, 4242]
  UID_GID 1500 1500
  ID_GROUPS 1500 4242
  PY_GETGROUPS [1500, 4242]
  AFTER_SETGROUPS [4242]
  ACCESS_GROUP_R True
  ACCESS_GROUP_W True
  AFTER_INITGROUPS [1500, 4242]
  PASS  setgroups/initgroups/getgroups/access
  PASS  supplementary group child probe

PASS=1 FAIL=0
```

完整门禁 A 同轮通过：`PASS=38 FAIL=0`；同一个 loader 的挂载专项 `PASS=18 FAIL=0`。
`make check` 通过：74 个 shell 脚本、285 个 Markdown 文件、版本检查通过。这证明用户管理、
dpkg、apt、locale、路径隔离、seccomp 和挂载回归没有被补充组改动破坏。

FACT：`coverage-sweep.sh` 本轮为 `SKIP`，因为当前 rootfs 没有 `strace`；不能把这次 skip
写成 PASS。

## 4 · 边界

- FACT：上面的 `os.getgroups()`/`os.setgroups()`/`os.initgroups()` 走的是 libc 符号入口。
- FACT：`ctypes.CDLL(None).getgroups()` 这类“直接取原始 libc 符号”的调用仍然绕过 GOT
  改写，看到 Android 的真实组；这与 existing raw-call 边界一致，不是本次声称已解决的能力。
- UNKNOWN：单进程最多 32 个 guest 补充组；超过 32 个成员身份的发行版没有真机验证。
- UNKNOWN：跨 Android 版本、16 KiB/64 KiB 页、Adreno 设备和不同 `/etc/group` 写法未验证。
- UNVERIFIED：并发调用 `setgroups`/`initgroups` 与组权限检查的长时压力尚未单独跑；本次只验证
  了锁和真机探针的正常/失败边界。
