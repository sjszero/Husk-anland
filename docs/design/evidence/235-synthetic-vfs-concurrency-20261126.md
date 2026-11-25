# 235 · 合成 proc/sys 文件的并发串内容

日期：2026-11-26 02:08（CST）。设备 RMX3357 / Android 13 / kernel 4.19.191 / 4 KiB 页。
对象：`path-shim` 的 `virt_open()`，覆盖 `/proc/mounts`、`/proc/self/status`、
`/proc/sysvipc/shm`、`/proc/uptime`。本轮没有修改 2.0 源码，也没有更换 X11 合成器。

## 1 · 结论

| 项 | 结果 | 证据等级 |
|---|---|---|
| 旧 loader 并发打开不同合成文件 | **FAIL**：64,000 次里 35,348 次读到另一个文件的内容 | FACT，真机 |
| 根因 | 所有合成文件共用无锁 `VTEXT[4096]`，填充写到一半就被另一个线程覆盖 | FACT，读码 + 复现 |
| 修后 loader | **PASS**：64,000/64,000，bad=0 | FACT，真机 |
| 同一线程在信号处理器里重入 | 返回 `EAGAIN`，不自锁；本轮没有构造信号重入探针 | INFERENCE / UNKNOWN |

这不是“日志难看”：`/proc/mounts` 可能返回 `/proc/self/status` 的 `Name:/Uid:` 文本，
挂载解析器会把它当成坏挂载表；`/proc/self/status` 也可能返回 `ipcs` 的表头。
多线程程序（GNOME、GIO、Python）在启动阶段会同时读这些文件。

## 2 · 复现

探针：`tools/device-tests/virt-concurrency-probe.rs`。
16 个线程同时运行，每个线程 4000 轮，在四个路径之间轮转；每次都检查返回内容是否
属于请求的那个文件。旧 loader 输出：

```text
BAD path=/proc/uptime round=0 len=15 head="       key     "
BAD path=/proc/mounts round=0 len=317 head="       key      shmid perms       size  cpid  lpid nattch   uid   gid  cuid  cgid      atime      dtime      ctime      "
BAD path=/proc/sysvipc/shm round=0 len=137 head="/data/user/0/com.linuxhub.husk/files/rootfs-full / f2fs rw,relatime 0 0\nproc /proc proc rw,nosuid,nodev,noexec,relatime "
BAD path=/proc/mounts round=0 len=317 head="Name:\tvirt-concurrenc\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t29343\n..."
RESULT bad=35348 ok=28652 threads=16 rounds=4000 opens=64000
RC=1
```

修复前 loader 的实测路径是应用 APK 解包目录里的 `files/libdir/libld.so`；同一个探针、
同一个 rootfs。这个结果不是缓存或测试注入器造成的：四个路径由同一进程的普通
`read_to_string()` 打开。

## 3 · 修复

`runtime/crates/path-shim/src/lib.rs`：

- 新增 `VIRT_BUSY: AtomicBool` 和 `VIRT_OWNER: AtomicI32`；
- `virt_open()` 从生成内容开始到 memfd 写完整段持锁；
- 该锁只覆盖合成文件，不进入普通路径解析；
- 用 owner tid 识别同一线程重入；该路径返回 `errno=EAGAIN`，避免在信号处理器里
  等自己；
- 所有早退分支都显式解锁。

最终 loader（含 evidence 236 的身份原子化补丁）SHA-256：

```text
d708257b1b1c80e8c4570c07a7f861897fbb4551cd5831f96dd31d6004a74e44
```

## 4 · 修后复测

```text
RESULT bad=0 ok=64000 threads=16 rounds=4000 opens=64000
RC=0
```

复现入口：

```sh
DEV=<serial> PKG=com.linuxhub.husk \
ROOTFS=files/rootfs-full LD=files/rootfs-full/root/libld-virtfix.so \
  make virt-race
```

要求：设备侧存在 `/data/local/tmp/husk-session.sh`（脚本会自动推送最新版本），
宿主机有 `aarch64-unknown-linux-gnu` 和 `/tmp/nr-toolchain/root`。

## 5 · 边界

- FACT：本轮修的是同一进程内多个线程打开不同合成文件时的内容串写。
- FACT：修后 64,000 次并发打开没有出现混读。
- UNKNOWN：信号处理函数在 `virt_open()` 持锁期间重入的具体行为没有实机注入验证；
  代码路径按 owner tid 明确返回 `EAGAIN`，但没有把它写成“信号场景已通过”。
- UNKNOWN：不同 Android 版本的 `/proc`/sys 文件集合与内核返回内容可能不同；探针只覆盖
  当前设备实际走 `virt_open()` 的四个文件。
