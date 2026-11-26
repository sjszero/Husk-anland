# 261 · session locale：env -i 边界的中文默认值

日期：2026-11-26（CST）。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB。
被测物：App 内 `files/linux/distributions/ubuntu-24.04/husk-session.sh`，通过
`tools/device-tests/run-session-locale-probe.sh` 启动真实 guest session。

## 0 · 结论

FACT：修前，默认启动会话的进程环境是 `HOME=/root`、`USER=root`、`LANG=C.UTF-8`。这与
rootfs 内已有的 `zh_CN.UTF-8` locale 不一致，也解释了为什么同一棵在本机应显示中文的
2.0 改造树进入 Husk 后仍以英文环境启动。

FACT：修后，默认启动会话得到 `LANG=zh_CN.UTF-8`、`LANGUAGE=zh_CN:zh`；调用方显式传入
`LANG=en_US.UTF-8 LANGUAGE=en_US:en` 时两项保持原值，没有被默认逻辑覆盖。

## 1 · 修前原始输出（FACT）

```text
HOME=/root
USER=root
LANG=C.UTF-8
```

## 2 · 修复内容

`tools/guest/husk-session.sh`：

- 在 `env -i` 边界之前选择 `SESSION_LANG` 和 `SESSION_LANGUAGE`。
- 调用方显式给出的 `LANG`/`LANGUAGE` 保留。
- `LANG` 为空或是 `POSIX`、`C`、`C.UTF-8` 时，rootfs 有 locale archive 或
  `zh_CN.utf8` 目录则使用 `zh_CN.UTF-8`，否则回退 `C.UTF-8`。
- 未显式给出 `LANGUAGE` 时使用 `zh_CN:zh`。
- 两个值通过 `env -i` 明确传入，保证登录 shell、桌面和普通子进程拿到同一环境。

新增 `tools/device-tests/run-session-locale-probe.sh`，并接入 `make gate-a`。探针不是检查
脚本里的字符串，而是实际启动 session 后读取 `/usr/bin/env`。

## 3 · 修后原始输出（FACT）

```text
PASS default=zh_CN.UTF-8 explicit=en_US.UTF-8
```

同一轮探针还逐行断言：

```text
LANG=zh_CN.UTF-8
LANGUAGE=zh_CN:zh
LANG=en_US.UTF-8
LANGUAGE=en_US:en
```

完整门禁 A 同轮通过：`PASS=38 FAIL=0`。`make check` 通过：75 个 shell 脚本、286 个
Markdown 文件、0 个坏链，版本检查为 `0.0.32`。`coverage-sweep` 仍因 rootfs 没有 `strace`
为 `SKIP`，不能算 PASS。

## 4 · 边界

- FACT：证据来自上述真机和 App 内 guest rootfs；不是纯源码推断。
- FACT：修复在 Husk 的 session 运行层，不是 LinuxHub 2.0 桌面补丁，也不依赖改桌面应用。
- UNKNOWN：其他 rootfs 若存在 `locale-archive` 但不包含 `zh_CN`，当前只按 archive 文件存在
  选择中文，实际字符集可用性仍需该 rootfs 的 `locale -a` 验证。
- UNKNOWN：不同 Android 版本、16 KiB/64 KiB 页和 Adreno 设备的同一探针未验证。
