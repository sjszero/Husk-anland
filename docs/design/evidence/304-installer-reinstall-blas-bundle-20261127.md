# 304 · 安装器重复安装、可选 BLAS 与系统总线配置打包

日期：2026-11-27（CST）
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页
目标环境：`com.linuxhub.husk`

涉及文件：

- `tools/guest/install-husk.sh`
- `Makefile`
- `tools/guest-config/linuxhub-system-bus.conf`（原有文件，补进发行包）

## 0. 结论

- **FACT**：向非空 `--dest` 重复安装并不是可恢复升级。目标里遗留的 `bin/`、`sbin/`、`lib/` 实体目录阻止 tar 建立 usr-merge 链接，随后 pass 2 以 `/bin/sh` 不存在失败。安装器现在默认拒绝非空目录，显式 `--fresh` 才替换；`/` 永远拒绝。
- **FACT**：旧安装器只要树里有 `python3` 就无条件要求 `libblas.so.3` 和 `liblapack.so.3`，导致一棵没有 BLAS 但完全有效的 807 包桌面树以 `OSError: libblas.so.3` 退出 73。现在只验证树里实际存在的 BLAS/LAPACK。
- **FACT**：`make dist` 没有把 `tools/guest-config/linuxhub-system-bus.conf` 放进 `guest-config/`，也没有产物断言；装出的树因此缺少系统总线配置，`20-tree-integrity.sh EXPECT=desktop` 实报 `PASS=12 FAIL=1`。Makefile 已补复制与 tarball 断言，安装器缺文件时也会明确 WARNING。
- **FACT**：v2 树完成真机安装，`INSTALL_RC=0`；树体检从 `12/1` 变为 `14/0`（补入 PanVK 驱动与 ICD 后）。
- **FACT**：`make dist` 最终成功；发行 bundle 含 `guest-config/linuxhub-system-bus.conf` 和最新 loader。用该 bundle 在空目录重新安装，`INSTALL_RC=0`，树体检 `PASS=13 FAIL=0`，门禁 A `PASS=34 FAIL=0 SKIP=1`。
- **FACT**：门禁 A `PASS=34 FAIL=0 SKIP=1`；唯一 SKIP 是发布树没有 apt package index，属于预期离线状态。

## 1. 非空目标目录

### 修前负向现场

先前的坏树残留在 `files/release-tree/root`。再次安装 v2 时：

```text
tar: can't remove: ./bin: Is a directory
tar: can't remove: ./sbin: Is a directory
tar: can't remove: ./lib: Is a directory
usr/bin/env: '/bin/sh': No such file or directory
install-husk: pass 2 failed (tar rc=127); hard links are not materialised, this tree is broken
```

根因不是 tarball：目标里的实体目录与 `bin -> usr/bin`、`sbin -> usr/sbin`、`lib -> usr/lib` 冲突。旧脚本没有检查目标是否为空，也没有升级语义。

### 修复

```text
--fresh   replace a non-empty destination instead of refusing it
```

- 非空且没有 `--fresh`：退出 65，并提示改用空目录或 `--fresh`。
- 根目录 `/`：退出 64，绝不删除。
- `--fresh`：先解析成绝对路径，再删除并重建目标。

### 正向验证

主机负向测试：

```text
rc=65
install-husk: destination is not empty: .../dest
               use an empty directory or pass --fresh to replace it

root_rc=64
install-husk: refusing to use / as --dest
```

真机：

```text
== fresh target: replacing /data/user/0/com.linuxhub.husk/files/release-tree-v2/root ==
...
== done ==
INSTALL_RC=0
```

## 2. 可选 BLAS/LAPACK

### 修前负向现场

807 包树安装主体和 guest 验证已经完成，最后却失败：

```text
OSError: libblas.so.3: cannot open shared object file: No such file or directory
install-husk: soname resolution failed (libblas/liblapack) - do not ship this tree
```

该树的 dpkg 记录里没有 `libblas3` / `liblapack3`，也没有任何程序依赖它们。把“可选库不存在”判成“安装失败”是验证器错误。

### 修复

安装器先检查 `libblas.so.3` / `liblapack.so.3` 是否真的在树里；只对实际存在的 soname 运行 `ctypes.CDLL()`。都不存在时输出：

```text
skipped BLAS/LAPACK soname check (not installed in this tree)
```

### 正向验证

同一棵 807 包树：

```text
== verify ==
PRETTY_NAME="Ubuntu 24.04.5 LTS"
bins=652
== ok: hard links materialised (uncompress + perl5.38.2 are real files) ==
skipped BLAS/LAPACK soname check (not installed in this tree)

== done ==
INSTALL_RC=0
```

## 3. 系统总线配置没有进发行包

### 修前负向现场

```text
FAIL  linuxhub-system-bus.conf missing: gnome-shell aborts on 'Couldn't connect to system bus'
TREE_INTEGRITY PASS=12 FAIL=1
```

仓库里有 `tools/guest-config/linuxhub-system-bus.conf`，安装器也会复制它；缺的是 `make dist` 的打包清单和断言。

### 修复

- `dist/guest-config/` 现在复制 `linuxhub-system-bus.conf`。
- `husk-arm64.tar.gz` 产物断言该文件必须存在。
- 安装器找不到该文件时明确 WARNING，不再静默装出缺系统总线的桌面树。

### 正向验证

把配置放进设备上的测试 bundle 并重装后：

```text
== desktop session requirements ==
PASS  linuxhub-system-bus.conf present
PASS  zh_CN langpack: 413 .mo files
PASS  dbus-daemon present
TREE_INTEGRITY PASS=13 FAIL=0
```

## 4. v2 树最终真机结果

构建与安装：

```text
installed packages: 807
dpkg .list files:   807
tar size:           672 MiB
tar sha256:         1d6f08b325d895c25d764071fd759587a43e768b2d2c802409680018a6b41e4b
install rc:         0
```

加载器一致性：

```text
files/linux-clean/libld.so
96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b

previous bundle loader
b96b7860ea1735b65068e3d0dfa40a03c9aa96d6af25017224000c053782ad5d
```

GPU 硬件驱动补入 v2 测试树后：

```text
libvulkan_panfrost_frist370_execva.so
599148568d80041ed3fd3312f5c9834479bcac9a13183f2b1479048b72a29b9c

husk_panvk_guest.json
baf69606d5ef781f7f493eea83d02b0acb574961a8c04e60021a718d2e36eb6f

GPU_LOADER_DEFAULTS PASS=9 FAIL=0 SKIP=0
plain renderer: zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK))
```

树体检：

```text
TREE_INTEGRITY PASS=14 FAIL=0
```

门禁 A：

```text
PASS=34 FAIL=0 SKIP=1
```

## 5. 发行 bundle 的干净安装（FACT）

```text
dist/husk-arm64.tar.gz
size:   7139367 bytes
sha256: 7f7083e68cc0b8706a3912faf61b54d69084d93331345343dae3e0db4f76a877

bundle libld.so: 96b96bbe7895b871a65df8598e146bc4566910ce2f68a2cafaf0392e1563141b
bundle guest-config/linuxhub-system-bus.conf:
  6b5f94315477fa036684b6af6c53f599f6663316b4dd100e1218c9bd23cdaef8
```

从该 bundle 装到全新目标：

```text
INSTALL_RC=0
TREE_INTEGRITY PASS=13 FAIL=0
gate A: PASS=34 FAIL=0 SKIP=1
```

注意：这次 bundle 测试的 rootfs 尚未自动携带 PanVK 驱动，因此树体检显示 `driver=none` 并把它作为 base tree 接受；GPU 正验证是在把设备上已有的 PanVK 驱动补入同一 v2 树后完成的。发行资产仍需把“驱动如何随 rootfs 发布”定成正式契约，不能把手工补入当成已完成。

## 6. 未完成 / 不冒充

- PanVK 驱动当前是从设备已有的 2.0 根目录补入 v2 测试树；发行资产如何携带该驱动仍需在发行轮定稿。不能把当前手工补入说成 `nr-tools` 已自动打包 GPU 驱动。
- 门禁 A 的 apt 索引检查为 SKIP；没有把它计作 PASS。
