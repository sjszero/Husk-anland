# 页大小（4 KiB / 16 KiB / 64 KiB）：兼容性边界与设计

> 结论先行：**不需要"4K 构建 + 16K 构建"两套产物。一套 over-align 的产物通吃 4K/16K/64K 内核。**
> 真正要做的只有两件事：构建期把对齐拉大，运行期从 `AT_PAGESZ` 读一次页大小，绝不硬编码 4096。

## 1. 机制（不是"设备属性"，是 ELF 与内核的对齐约定）

内核的 `PAGE_SIZE` 是**编译期固定**的（arm64 有 4K / 16K / 64K 三种内核），运行期不可变。

`execve()` 加载 ELF 时，对每个 `PT_LOAD` 用 `ELF_PAGESTART(p_offset)` 作为 `mmap()` 的文件偏移；
而 `mmap()` 要求文件偏移按 `PAGE_SIZE` 对齐，否则返回 `EINVAL`，整个 `execve` 失败。

因此真正的判据是 **同余关系**：

```
p_offset ≡ p_vaddr  (mod PAGE_SIZE)        # 每个 PT_LOAD 都要满足
```

以及 `p_align >= PAGE_SIZE` 时最好（`p_align` 是下界声明，越大越安全）。

推论，两件事必须分开说：

| 组合 | 结果 |
|---|---|
| 4K 内核 + 64K 对齐的 ELF | ✅ 能跑（16K/64K 对齐天然满足 4K 同余） |
| **16K 内核 + 4K 对齐的 ELF** | ⚠️ **多数失败**：多段布局下 `p_offset` 通常不是 16K 对齐 → `mmap` `EINVAL` → 加载失败。但**不是必然**：该段 offset 恰在 16K 边界、或只有一段，就能跑 |
| 16K 内核 + 16K/64K 对齐的 ELF | ✅ 能跑 |

所以用户说的"4K 设备不一定跑不了 16K、16K 设备不一定跑不了 4K"成立：
决定性的是**二进制怎么对齐**，不是设备标签。旧文档里写"只保证 4K"是**测试状态**，不是结论，已按下表订正。

## 2. 实测（2026-11-22，本机取样本）

| 产物 | `p_align` | 4K 内核 | 16K 内核 | 备注 |
|---|---|---|---|---|
| guest `usr/bin/bash`（Ubuntu noble arm64） | `0x10000` (64K) | ✅ | ✅ | GNU ld 的 aarch64 默认 max-page-size 就是 64K |
| guest `libc.so.6` | `0x10000` | ✅ | ✅ | 同上 |
| guest `ld-linux-aarch64.so.1` | `0x10000` | ✅ | ✅ | 同上 |
| 我们的 `libhuskapp.so`（NDK 29） | `0x4000` (16K) | ✅ | ✅ | NDK r28+ 默认 16K |
| 我们的 `libld.so`（改过的 loader） | `0x10000` / `0x1000` 混合 | ✅ | ✅（同余成立，但**靠运气**） | 要显式固定 |

关键事实：**Ubuntu/Debian 的 arm64 用户态本来就 64K 对齐**，所以 16K 内核上 guest 侧不需要任何改造。
这和"Android App 自己的 `.so` 必须 16K 对齐"是两套独立问题（后者是 Google Play 2025 的强制要求）。

## 3. 设计

### 3.1 构建期：over-align，单产物

所有我们自己产出的 ELF 统一：

```
-Wl,-z,max-page-size=0x4000      # 至少 16K；想覆盖 64K 内核就写 0x10000
-Wl,-z,common-page-size=0x1000   # 保持段内紧凑，体积不炸
```

代价只有文件里段间填充；收益是不再需要 per-page-size 构建矩阵。
`nr-tools embed` 加一条断言：输出的每个 `PT_LOAD` 必须满足 `p_offset ≡ p_vaddr (mod 0x4000)` 且 `p_align >= 0x4000`，
不满足直接构建失败（和现有"loader 指纹/glibc 版本断言"同一风格）。

### 3.2 运行期：只读一次 `AT_PAGESZ`

入口 stub 在内核给的初始栈上就能拿到 auxv（`AT_PAGESZ = 6`），**在任何 libc 代码之前**：

```
sp -> [argc][argv...][NULL][envp...][NULL][auxv: {type,value} ...]
```

存进一个只读全局，凡是要"按页"算的地方一律用它：
`mprotect` 的地址/长度取整、`mmap` 的 offset 与 length、shim 自身窗口的边界页填充。
禁止出现字面量 `4096`。

### 3.3 校验期：rootfs 体检

`nr-tools` 增加 `check-elf`：遍历 rootfs 里的 ELF，报告 `p_align` / 同余是否满足给定 `PAGE_SIZE`。
provision 时对 4K 和 16K 各跑一遍，输出一张表进证据目录。这样"能不能在 16K 设备上跑"是查出来的，不是猜的。

## 4. 明确否决的方案

- ❌ **双构建目标（4K / 16K 各一份）**：对齐是下界约束，over-align 已经通吃；双目标只会把验证矩阵翻倍。
- ❌ **运行期改页大小**：不可能，`PAGE_SIZE` 是内核编译期常量。
- ❌ **"设备是 16K 所以不能用 4K 产物"**：错，看 ELF 对齐。
