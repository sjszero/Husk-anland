# 337：外网门禁改为纯 Rust 探针

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13 / kernel 4.19.191 / arm64 / 4 KiB 页

## 1. 修前问题（FACT）

`tools/device-tests/13-egress.sh` 仍有 Python socket heredoc，并依赖 guest 中的
`awk`、`curl`、`ping`。当前测试树并不是这些命令都齐全：

```text
awk=MISSING
mawk=/usr/bin/mawk
curl=MISSING
wget=MISSING
ping=MISSING
openssl=MISSING
```

第一次真机执行旧脚本得到：

```text
awk: 未找到命令
FAIL  DNS resolves ports.ubuntu.com
awk: 未找到命令
FAIL  DNS resolves one.one.one.one
send returned 74
PASS  send() + HTTP/1.0 GET over a real socket
FAIL  curl http -> 200 (got '')
FAIL  curl https (TLS) -> 200 (got '')
PASS  ping answered: timeout: 无法运行命令 "ping": 没有那个文件或目录
PASS=2 FAIL=4
```

这里有两个独立缺陷：

- 门禁把“测试工具未安装”误算成网络失败；
- 最后一项把 `ping` 不存在误算成 PASS。

## 2. 改动（FACT）

- `runtime-probe egress` 现在用 Rust 完成两个 DNS 查询、TCP 连接、libc
  `send(2)`、HTTP/1.0 响应、rustls HTTPS/TLS 响应和 raw ICMP 能力边界检查。
- 不再要求 guest 安装 Python、awk、curl、ping 或 openssl。
- `13-egress.sh` 只负责调用 Rust 探针并汇总 PASS/FAIL。
- 新增 `run-egress-probe.sh`，负责把当前 runtime-probe 和脚本放入测试 rootfs，
  使用临时 loader 执行，结束后清理设备副本。
- `build-guest-tools.sh` 把 NDK clang、aarch64 glibc sysroot 和 llvm-ar 传给
  Rust C/汇编依赖，修复 `ring` 在干净环境找不到 `aarch64-linux-gnu-gcc` 的构建问题。

## 3. 修后正向结果（FACT）

主机单测：

```text
cargo test -p runtime-probe
3 passed; 0 failed
```

guest 工具交叉构建：

```text
runtime-probe  entry 0x941C0
runtime-probe  sha256 1a7bfb9f59d4b4f70ddd200c921a504af46e609bd22d63344de3d49271b69b7a
```

真机执行 `run-egress-probe.sh`：

```text
DNS ports.ubuntu.com 91.189.92.19:80
DNS one.one.one.one 1.0.0.1:443
send returned 74
HTTPS HTTP/1.1 200
ICMP raw socket refused (Operation not permitted (os error 1); documented platform limit)
  PASS  DNS + TCP + send() + HTTP + HTTPS/TLS + ICMP boundary
PASS=1 FAIL=0
```

完整回归：

```text
NDK=<ndk>/bin make check
PASS（整 workspace、Clang 格式检查、shell 语法、发布契约和文档链接）

DEV=<serial> ROOTFS=files/rootfs-full LD=files/rootfs-full/root/libld-test.so make gate-a
PASS=38 FAIL=0 SKIP=0
coverage-probe: PASS
coverage-sweep: PASS（1089 条候选，0 漏网）
fd-identity-probe: PASS
supplementary-groups: PASS=1 FAIL=0
identity-init-race: rounds=300 fails=0
groups-race: rounds=20 fails=0
rootfd-close-range: PASS
path-race: PASS
```

`Makefile` 的 workspace check/clippy 和 CI 的 check/clippy 现在都会把 cross C
compiler 传给需要 C/汇编的 Rust crate；否则 rustls 的 `ring` 后端在干净环境会因
找不到 `aarch64-linux-gnu-gcc` 失败。

## 4. 边界（FACT / UNVERIFIED）

- raw ICMP 在本机 Android app 权限下返回 `EPERM`；这是平台限制，门禁只验证它被
  明确拒绝，不把 ping 不通宣传成成功。
- 本轮没有 bump `VERSION`、没有 tag、没有发 Release。
- HTTPS 使用内置 webpki 根证书集合，验证的是当前公网证书链；私有 CA、代理抓包和
  企业网络仍 UNVERIFIED。
- 测试结束后临时 loader 和 `/data/local/tmp` 副本均已删除。
