# 301 · nr-tools 下载静默挂死：限时 DNS + 按 Content-Length 收尾

日期：2026-11-27（CST）
设备/主机：macOS 主机构建，ADB 设备 `UKHMMB8PYTGECMNV` 在线
涉及文件：`runtime/crates/nr-tools/src/provision.rs`

## 0. 结论

- **FACT**：旧的 `http_get_once()` 在发出 GET 后调用 `TcpStream::read_to_end()`。TCP 对端即使已经发完一个带 `Content-Length` 的完整 HTTP 响应，只要不主动关连接，进程就会一直等 EOF。
- **FACT**：完整桌面树重建曾在 718 个 deb 处静默停 1 个多小时；采样栈落在 `http_get_once()` → `TcpStream::read_to_end()` → `__recvfrom`，不是 CPU 忙等。
- **FACT**：本次改为 DNS 辅助线程 + 10 秒接收超时、连接 20 秒超时、每响应 120 秒总时限、最大 512 MiB、最多 5 次重定向；读到 `Content-Length` 指定长度后立即返回，不再等对端关闭。
- **FACT**：`cargo test -p nr-tools`：`7 passed; 0 failed`。
- **FACT**：修复后的版本完成 807 包 v2 桌面树构建；设备安装退出 0，树体检 `PASS=14 FAIL=0`，门禁 A `PASS=34 FAIL=0 SKIP=1`。

## 1. 修前负向现场

旧构建进程在下载阶段停住时：

```text
PID    STAT  %CPU
4283   Ss    0.0
```

采样栈（关键帧）：

```text
http_get_once
TcpStream::read_to_end
std::sys::unix::net::...
__recvfrom
```

同时 `ls /tmp/nr-rootfs/deb | wc -l` 长时间不增长。说明阻塞发生在已建立连接后的读取，不是 DNS 查询、索引解析或 dpkg 解包。

旧代码形状：

```rust
let mut raw = Vec::new();
stream.read_to_end(&mut raw).ok()?;
```

`read_to_end()` 的成功条件包含 EOF；HTTP/1.0 服务器通常关闭连接，但不能把这个行为当成协议保证。

## 2. 修复内容

`provision.rs` 现在：

1. `host_addr()` 把无超时的 `ToSocketAddrs` 放进辅助线程，调用方最多等 10 秒；结果用 `OnceLock` 缓存，地址排序为 IPv4 优先。
2. 连接、读、写分别设 20 秒超时；连接后切换为非阻塞读取。
3. `read_response()` 每块最多读 64 KiB，单响应总时限 120 秒，20 秒无进展失败，响应上限 512 MiB。
4. 启动 10 秒后要求平均至少 8 KiB/s；低于该速度即失败并轮换下一个解析地址，防止镜像“偶尔滴一个包”把整棵构建挂住。
5. 响应头到达后解析大小写不敏感的 `Content-Length`；字节数满足后立即返回。
6. 重定向最多 5 跳，超过则失败，避免递归失控。
7. 仍然保留整请求 3 次重试；内容长度校验仍由上层按索引 `Size` 执行。

## 3. 正向测试（FACT）

```text
$ cd runtime && cargo test -p nr-tools
running 7 tests
test provision::tests::response_length_parser_handles_header_case ... ok
test provision::tests::response_reader_stops_at_content_length_without_eof ... ok
...
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

第二个测试由本地 TCP 服务器发送完整的 `Content-Length: 7` 响应后故意保持连接 500 ms；旧实现会等服务器关闭，新实现在读完 `payload` 后立即返回。

## 4. 完整树与真机正验证（FACT）

```text
installed packages: 807
dpkg .list files:   807
tar size:           672 MiB
tar sha256:         1d6f08b325d895c25d764071fd759587a43e768b2d2c802409680018a6b41e4b
install rc:         0

TREE_INTEGRITY PASS=14 FAIL=0
PASS=34 FAIL=0 SKIP=1
```

唯一 SKIP 是离线发行树没有 `/var/lib/apt/lists`，因此 `apt-cache policy` / `apt-get -s` 无法执行；没有把它算作 PASS。
