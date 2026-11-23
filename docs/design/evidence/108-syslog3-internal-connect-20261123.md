# 108 · `syslog(3)` 也通了：connect/bind 进 libc 槽位表

日期：2026-11-23　设备：RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB / `untrusted_app`

## 现象（改之前，FACT）

`logger -t husk "..."` 能落到 `/var/log/syslog`，但任何走 **libc 自己**的
`syslog(3)` 都发不出去。原因是 `syslog()` → `openlog_internal()` → `__connect()`
是 libc 同一编译单元内的直接 `bl`，没有 GOT 槽，`boot.rs` 的 GOT 重写覆盖不到；
于是内核收到的是 guest 路径 `/dev/log`，在宿主命名空间里不存在。

这是 evidence 105 同一个根因的最后一个残留：**libc 内部调用要靠构建期槽位**。

## 修法

`connect` / `bind` 加进 `tools/loader/libc-interpose.txt`，重跑

```sh
runtime/target/release/nr-tools libc-interpose \
    libc.so.6.orig /tmp/libc.slots.so.6 tools/loader/libc-interpose.txt
```

输出 `62 of 73 entries patched`（原 56/71，两个新增都落地）。

- `connect` 与 `__connect` 在本机 glibc 2.39 是**同一地址**（`0xedf90`，208 字节的
  cancellable 包装），所以 patch 一个入口即可同时覆盖程序侧和 libc 内部。
- `bind` 在 `0xedf40`（72 字节）。
- shim 的 `connect`/`bind` 只改写 **AF_UNIX 具名路径**，abstract / unnamed /
  非 AF_UNIX 一律透传；因此 `resolv` 的 `connect(AF_INET)` 不受影响。

## 结果（FACT，同一台机）

```
$ python3 -c 'import syslog;syslog.openlog("husktest");syslog.syslog(syslog.LOG_INFO,"syslog3 via libc internal connect")'
SENT
$ tail -1 /var/log/syslog
2026-11-23T10:28:03Z <14>Nov 23 10:28:03 husktest: syslog3 via libc internal connect
```

python 的 `syslog` 模块直接调 `syslog(3)`，所以这一条就是 libc 内部路径本身。

## 回归

`rt-server-parity.sh` 的 `svc syslog_libc` 从写死的 PARTIAL 改成真实测量
（python 发一条，回读 `/var/log/syslog`）。

**71 PASS / 5 PARTIAL / 6 FAIL**（70/6/6 → 71/5/6）。

## 副作用

- 打的日志里 `openlog`/`syslog` 走 libc 内部 connect，之前"静默丢日志"消失；
  systemd-journald 之外的经典 syslog 客户端（dnsmasq、cron、rsyslog 客户端）现在都能用。
- `connect` 变体多了一个 `syscall4` 的绕行，略过 cancellation point；这与程序侧
  已有的 interpose 行为一致，没有新增风险面。
