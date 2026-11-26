# 259 · 文件创建 mode、虚构所有权与 `access/faccessat` 主体

日期：2026-11-26。设备：RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 /
4 KiB。主要探针：`tools/probe/coverage-probe.c`、`runtime/crates/fd-identity-probe`、
`tools/device-tests/run-dpkg-conffile-mode-probe.sh`。

## 0 · 修前事实（FACT）

### 0.1 `O_CREAT` 改变文件模式

guest 请求创建 `0440` 文件，旧 loader 读回 `0640`：

```text
CREATE_MODE open_fd=4 errno=0 mode=640 write=1 expected=440
```

根因是 `openat_inner()` 对所有带 `O_CREAT` 的调用执行 `mode | 0600`。这是为 dpkg 重开
`0440` conffile 加的历史补丁，但它改变了所有程序创建的所有文件。

### 0.2 写放宽后没有恢复模式

旧 loader 为假 root 重开 `0440` 文件时永久改成 `0600`：

```text
OWNER_RESTORE open_fd=4 errno=0 mode_open=600 write=1 mode_final=600
```

### 0.3 `fchown(fd)` 没有记录虚构所有权

旧实现返回成功，但同一 fd 的 `fstat` 和后续进程的路径 `stat` 都还是真实 app uid/gid：

```text
FCHOWN_SAME 0 0
FCHOWN_CROSS 0:0
```

### 0.4 `access/faccessat` 使用真实 Android uid

旧 loader 把路径直接交给内核，假 root 对 `0000` 文件的可读写检查错误返回 EACCES：

```text
ACCESS_ROOT read=-1/13 write=-1/13 exec0=-1/13 exec5=0/0
```

非 root 检查也不成立。第一版探针放在 `/root` 下，无法把父目录权限与最终对象权限分开；把测试
改到 `/tmp` 的两个隔离目录后，只检查最终对象的实现会把 `0700 root` 父目录里的文件误判为
可读。修前该场景输出为：

```text
ACCESS_USER parent=0/0 owned=0/0
FAIL 非 root 的 access 检查父目录与虚构 owner
```

## 1 · 修法

1. `openat_inner()` 保留调用方请求的 mode，不再追加 `0600`。
2. `widen_owner_write()` 先用 `target_fd()` 钉住 inode，再临时增加 owner-write；重新
   `open()` 成功后立即恢复到原 `07777`。恢复失败时关闭新 fd 并返回错误。
3. `fake_access_mode()` 按 guest uid/gid 与虚构所有权计算：
   - 假 root：读写使用 `CAP_DAC_OVERRIDE` 语义；普通文件执行仍要求至少一个 X 位。
   - 非 root：按 owner / group / other 模式位判断。
4. `fake_access_path()` 先钉住最终对象，再从最终 fd 的真实宿主路径逐级检查父目录 X 权限；
   父目录通过 `O_PATH | O_DIRECTORY | O_NOFOLLOW` 固定，元数据按虚构身份判断。
5. 只读模拟挂载上的 `W_OK` 保持 `EROFS` 语义。
6. `fchown(fd)` 写入虚构所有权表，`fstat(fd)` 与跨进程路径 `stat` 读取同一份数据。
7. 新增 dpkg 探针：构建一个带 `0440` conffile 的临时 deb，真实执行 `dpkg -i`，检查最终
   落盘模式并清理包。

## 2 · 修后真机结果（FACT）

```text
CREATE_MODE open_fd=4 errno=0 mode=440 write=1 expected=440
OWNER_RESTORE open_fd=5 errno=13 mode_open=440 write=1 mode_final=440
ACCESS_ROOT read=0/0 write=0/0 exec0=-1/13 exec5=0/0
ACCESS_USER parent=-1/13 owned=0/0
fdidentity|pathshim-fchown-owner|PASS|chown_rc=0 chown_errno=0 same=1500:1600 cross=1500:1600 expected=1500:1600
DPKG_CONFFILE_MODE|PASS|mode=440 expected=440
```

`OWNER_RESTORE` 行里的 `errno=13` 是成功 `open()` 前一次失败尝试留下的值；本次调用返回了
有效 fd 且 `write=1`，POSIX 只在失败时定义 errno。

## 3 · 边界

- FACT：动态验证范围是 RMX3357 / MT6893、Mali-G77、Android 13 / 4 KiB。
- FACT：三项 path-shim 修复都在发布包 `libld.so` 中。
- UNKNOWN：其他 Android 版本、16 KiB/64 KiB 真机与 Adreno 设备上的同一行为。
- UNKNOWN：多线程目录替换、符号链接竞态与只读挂载组合压力未做专项动态验证。
- UNVERIFIED：写放宽后的模式恢复失败分支没有在真机上人为注入故障。

## 4 · 回归（FACT）

### 4.1 完整门禁 A

```text
PASS=38 FAIL=0
--- coverage-probe: PASS
--- coverage-sweep: PASS（1400 条候选路径，没有发现未翻译的 guest 绝对路径）
--- dpkg-conffile-mode-probe: PASS
V4L2_REF|PASS
--- drm-fd-probe: PASS
--- fd-identity-probe: PASS（18/18）
```

### 4.2 `make check`

```text
cargo fmt --all --check                 PASS
aarch64 cargo check                     PASS
cargo clippy                            PASS
shell syntax: 73 script(s) checked      PASS
checked 284 markdown files, 0 broken    PASS
version ok: 0.0.32                      PASS
```

### 4.3 发布包

`make dist` 成功，包内 `VERSION=0.0.32`：

```text
ba57c0419c4d8bfcd299ecea45660ccf3bb6ba6b746449c3a3e9430874c4f639  husk-arm64.tar.gz
b37a336d5e6319675f9490f39a32e34194ee0c405e46b96efb7313a6de3c1e33  libld.so
69ceef3759f456d80b4db4e013712e933a8c8d876e9af639102d8c88fc80f77c  libc.so.6
```

0.0.31 的 loader SHA256 是
`9c8b931f0793ff3ec6d840aefea8ac09d28569f70678dd3225a03c43faeed118`；0.0.32 变为
`b37a336d...`，证明本轮修复进入了发布环境，不是只改探针或实验 preload。

以上动态结果仅适用于 RMX3357 / MT6893 / Mali-G77 / Android 13 / 4 KiB。其他设备族和并发
压力仍是 UNKNOWN。
