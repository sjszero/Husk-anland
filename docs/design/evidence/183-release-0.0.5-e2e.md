# 183 · 0.0.5 发布产物端到端：发布包装出来的树 + 发布包 loader

日期：2026-11-25 11:45–12:15（CST）。设备 RMX3357（MT6939 更正为 MT6893 / Mali-G77 MC9 /
Android 13 / 4 KiB 页），应用 `com.linuxhub.myth.husk`。全部在设备上的新目录 `files/linux/e2e050`
里做，装完没有留在发布树里。

## 1 · 产物

| 文件 | 字节 | sha256 |
|---|---:|---|
| `dist/husk-arm64.tar.gz` | 1 915 761 | `8b28fe6ead965319db95a83f975e61f6d9f979aca2815290430153df53b3ea45` |
| `dist/husk-rootfs-ubuntu-24.04.tar.gz` | 1 297 297 954 | `e04b8a9db790b92572843d814f0a31d865c5076b5a63a6f3e514228594fa59a0` |

- 启动器包由 `make dist` 产出（`NDK=…/29.0.14206865/…/bin`）。包内 `libld.so`
  = `d0153751ca94…`，与设备上装的那份逐个一致（`tools/loader/ld-embedded.so` 同 sha）。
- 树包由**正在跑的那棵树**在设备上打（`make-release.sh`）：102 127 个条目、`gzip -t` 通过；
  传回主机后 sha 与设备侧一致（`e04b8a9d…`），1.30 GB 没有传坏。

## 2 · 用发布包在全新目录装一遍

```
sh install-husk.sh --rootfs ../husk-rootfs-0.0.5.tar.gz --dest ./root   → rc=0
   wrote etc/resolv.conf (2 nameservers)
   wrote etc/profile.d/husk-gpu.sh (GPU defaults, escape: HUSK_GPU=llvmpipe)
   wrote etc/husk/gpu-env (loader-level defaults: /usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json)
   links=29476 absolute=5 rewritten=4 dangling=1 left=0
   libc 69ceef3759f4 matches the bundle
   PRETTY_NAME="Ubuntu 24.04.4 LTS"   bins=1035
   ok: hard links materialised (perl5.38.2 + uncompress are real files)
```

`etc/husk/gpu-env` 是这一版新加的步骤，装出来的内容是安装器当场解析的 ICD 路径，不是硬编码。

## 3 · 新树门禁 A

`PASS=34 FAIL=0`（比主树少 2 项：树包里没有 apt 缓存 deb，与 0.0.4 同一差异）。
`real SIGSYS traps (0)`、`host paths only in internal NR_ vars (0)` 都在。

## 4 · 新树 + 发布包 loader，非登录执行也吃 GPU

```
=== 默认值（来自 /etc/husk/gpu-env，安装器写的） ===
GL_VERSION=OpenGL ES 3.2 Mesa 27.0.0-devel (git-b84f02a451)
  | RENDERER=zink Vulkan 1.3(Mali-G77 MC9 (MESA_PANVK)) | GLSL=OpenGL ES GLSL ES 3.20
RESULT PASS
=== HUSK_GPU=llvmpipe ===
  | RENDERER=llvmpipe (LLVM 20.1.2, 128 bits) |
RESULT PASS
```

两行都是 `env -i` 起的最小环境：除了 `NR_*` / `HOME` / `PATH` 之外一条图形变量都没有。

## 5 · 这一轮踩的坑（写下来免得再犯）

给 loader 的**程序参数**只能是「宿主绝对路径」或「cwd 下的相对路径」。给
`/usr/bin/id` 这种 **guest 绝对路径**会直接
`cannot open shared object file`——内核在 shim 起来之前就在宿主根上打开它，这一步拦不到。
`tools/device-tests/14-gate-a-acceptance.sh` 里的 `husk-exec.sh` 之所以一直用
`usr/bin/bash`（相对、`cd` 到 rootfs），就是这个原因；测试里第一次写错成 `/tmp/gl-probe`
（guest 视角的绝对路径）也踩了同一个坑。
