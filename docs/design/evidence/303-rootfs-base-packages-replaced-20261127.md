# 303 · 自定义 `--packages` 覆盖基础系统，生成一棵没有 shell/env 的 787 包树

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Android 13
涉及文件：

- `runtime/crates/nr-tools/src/rootfs.rs`
- `tools/guest/install-husk.sh`（只作为失败现场，不修改）

## 0. 结论

- **FACT**：`nr-tools rootfs --packages "..."` 旧实现把该字符串直接赋给 `packages`，随后只解析这一串；`BASE` 被整体替换，不是追加。
- **FACT**：用桌面包清单生成的树有 `787 installed / 787 .list`，但根目录没有 `usr/bin/env`、`usr/bin/sh`、`bash`、`dash`、`coreutils` 提供的命令。
- **FACT**：设备端 `install-husk.sh` 在 pass 2 真实失败：

```text
usr/bin/env: error while loading shared libraries: usr/bin/env: cannot open shared object file: No such file or directory
install-husk: pass 2 failed (tar rc=127); hard links are not materialised, this tree is broken
```

- **FACT**：修复后 `--packages` 只追加到 `BASE`，并去重。
- **FACT**：新增单测 `rootfs::tests::custom_packages_keep_the_base_system`；`cargo test -p nr-tools` 当前 `7 passed; 0 failed`（实际命令为 `cargo test -p nr-tools`）。
- **FACT**：修复后重新构建的 v2 树为 `807 installed / 807 .list`，含 `usr/bin/env`、`bash`、`dash`、`dpkg`、`apt`。
- **FACT**：v2 树真机安装退出 0；树体检 `PASS=14 FAIL=0`；门禁 A `PASS=34 FAIL=0 SKIP=1`。

## 1. 修前负向现场

主机树：

```text
$ ls /tmp/husk-desktop/usr/bin/env /tmp/husk-desktop/usr/bin/sh
No such file or directory
```

设备安装：

```text
== pass 2: re-extract through the shim (fixes hard links) ==
usr/bin/env: error while loading shared libraries: usr/bin/env: cannot open shared object file: No such file or directory
install-husk: pass 2 failed (tar rc=127); hard links are not materialised, this tree is broken
```

这不是 Android SELinux 或 loader 的问题；树里根本没有 `/usr/bin/env`。

## 2. 根因

旧代码：

```rust
let mut packages = BASE.to_string();
...
"--packages" => packages = next(&mut i),
```

`--packages` 的语义被实现成“替换基础集”。桌面包清单没有列出 `bash`、`dash`、`coreutils`，因此基础命令全部消失。

## 3. 修复

`rootfs.rs` 现在：

- `packages` 初始为空，`--packages` 的值只追加；
- `requested_packages()` 永远先加入 `BASE`，再加入额外包；
- 以 `HashSet` 去重，保留 `BASE` 顺序；
- 单测检查 `bash`、`dash`、`coreutils`、`dpkg`、`apt` 必须存在，且重复的 `bash` 只出现一次。

关键代码：

```rust
fn requested_packages(extra: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in BASE.split_whitespace().chain(extra.split_whitespace()) {
        if seen.insert(name) {
            out.push(name.to_string());
        }
    }
    out
}
```

## 4. 正向测试（FACT，当前阶段）

```text
$ cd runtime && cargo test -p nr-tools
running 7 tests
test rootfs::tests::custom_packages_keep_the_base_system ... ok
...
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 5. v2 完整树正验证（FACT）

```text
$ grep -c '^Status: install ok installed' /tmp/husk-desktop-v2/var/lib/dpkg/status
807
$ find /tmp/husk-desktop-v2/var/lib/dpkg/info -name '*.list' | wc -l
807
$ ls /tmp/husk-desktop-v2/usr/bin/{env,bash,dash,dpkg,apt}
all present
```

真机安装：

```text
INSTALL_RC=0
TREE_INTEGRITY PASS=14 FAIL=0
gate A: PASS=34 FAIL=0 SKIP=1
```

门禁 A 的唯一 SKIP 是离线树没有 apt package index；没有计作 PASS。
