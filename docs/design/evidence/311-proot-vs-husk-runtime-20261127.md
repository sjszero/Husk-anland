# 311 · PRoot 与 Husk 同机运行器对比（2026-11-27）

设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / kernel 4.19.191 / 4 KiB 页
对照：`com.linuxhub.myth`（Myth-Prometheus / PRoot）
被测：`com.linuxhub.husk`（Husk loader + path-shim）
测试脚本：`/sdcard/husk-runtime-probe.py`（临时探针，不进入发行包）

## 0 · 结论

这是单次现场采样，不是 5 轮热漂移修正基准；数字只用于本轮同机对比，不能替代
`docs/BENCHMARKS.md` 的 5 轮轮转数据。

| 项 | Husk | PRoot | 结论 |
|---|---:|---:|---|
| 打开设置到首个可映射窗口，3 次中位数 | 5,260.4 ms | 2,423.4 ms | 本轮 Husk 慢 2.17× |
| `stat` × 20,000 | 741.2 ms | 2,338.7 ms | Husk 快 3.16× |
| `open` + `close` × 20,000 | 980.9 ms | 5,676.3 ms | Husk 快 5.79× |
| shell loop × 1,000 | 18.2 ms | 51.5 ms | Husk 快 2.84× |
| `/bin/true` × 50 | 708.5 ms | 323.6 ms | 本轮 Husk 慢 2.19× |

**FACT**：Husk 的优势出现在路径和 syscall 密集的循环，符合“装载时改写 GOT、运行时不拦截
syscall”的机制；`/bin/true` 这种每次都要重新装载的短进程反而暴露了 loader 的启动成本。

**FACT**：打开设置的对照条件并不完全相同：PRoot 使用 App 自带的 X11 载体，Husk 侧为了
本轮独立树测试使用从旧树复制的 Xvfb；两者都跑 GNOME 设置，但不是同一个 X server 实现。
因此“Husk 慢 2.17×”不能归因于 PRoot 的 ptrace 或 Husk 的 GPU；它是本轮实测结果，不能包装成
Husk 在所有图形启动上更快。

## 1 · Husk 原始 JSON

```json
{"display": ":0", "python": "3.12.3", "stat_20000_ms": 741.2298460258171, "open_close_20000_ms": 980.8831539703533, "shell_loop_1000_ms": 18.15453899325803, "true_50_ms": 708.4769229986705, "x_windows": [[6291463, 2400, 1080, "mutter guard window"], [16777219, 2400, 1080, "Desktop Icons 1"]]}
```

```text
XSTART_SUMMARY {"samples": [{"run": 1, "first_frame_ms": 5703.42392398743, "window": [14680068, 1102, 762, "Settings"]}, {"run": 2, "first_frame_ms": 5260.435692965984, "window": [14680068, 1102, 762, "Settings"]}, {"run": 3, "first_frame_ms": 5157.670076994691, "window": [14680068, 1102, 762, "Settings"]}], "median_ms": 5260.435692965984}
```

## 2 · PRoot 原始 JSON

```json
{"display": ":0", "python": "3.12.3", "stat_20000_ms": 2338.744000007864, "open_close_20000_ms": 5676.319615973625, "shell_loop_1000_ms": 51.52676900615916, "true_50_ms": 323.5643079970032, "x_windows": [[12582919, 2400, 1080, "mutter guard window"], [25165827, 2400, 1080, "Desktop Icons 1"], [27262982, 866, 1026, ""]]}
```

```text
XSTART_SUMMARY {"samples": [{"run": 1, "first_frame_ms": 2423.3726150123402, "window": [29360132, 896, 762, ""]}, {"run": 2, "first_frame_ms": 1403.7268470274284, "window": [29360132, 896, 762, ""]}, {"run": 3, "first_frame_ms": 3091.756077017635, "window": [29360132, 896, 762, ""]}], "median_ms": 2423.3726150123402}
```

PRoot 设置窗口的 X11 标题为空，Husk 的窗口标题为 `Settings`；两边的判定都用
“基线窗口之外新出现一个可映射的大窗口”，没有用标题字符串作为放行条件。

## 3 · 内存快照

同轮 `dumpsys meminfo`：

```text
com.linuxhub.husk: TOTAL PSS 32,932 KB / TOTAL RSS 81,560 KB
com.linuxhub.myth: TOTAL PSS 93,938 KB / TOTAL RSS 115,592 KB
```

这是运行状态快照，不是峰值，也不是单进程 PSS 对比；PRoot 的 guest 子进程由 App 进程树持有，
不能把它解读为“PRoot 每条 syscall 的内存成本”。
