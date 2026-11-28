# 349 · PRoot 与 Husk 同机实测对比（第一版）

日期：2026-11-27（CST）  
设备：RMX3357 / MT6893 / Mali-G77 MC9 / Android 13 / 4.19.191 / 4 KiB 页  
Husk：`com.linuxhub.husk`，`0.0.36`，提交 `c924d62`  
PRoot：`com.linuxhub.myth`，Ubuntu 24.04.4，GNOME，`libproot.so`

## 0 · 先给结论

| 场景 | PRoot | Husk | 结果 |
|---|---:|---:|---|
| 20,000 次 `stat` 中位数 | 2238.042 ms | 792.081 ms | Husk 约快 2.83× |
| 20,000 次 `open+close` 中位数 | 4417.096 ms | 755.712 ms | Husk 约快 5.85× |
| 1,000 次 shell 循环中位数 | 7.586 ms | 27.398 ms | Husk 约慢 3.61× |
| 50 次 `/bin/true` 中位数 | 309.734 ms | 369.958 ms | Husk 约慢 1.19× |
| 打开设置到首个可映射窗口 | 2810.429 ms | 无法完成 | Husk 缺少自己的 X server |

**FACT**：Husk 在文件元数据和打开/关闭路径上明显快于 PRoot。  
**FACT**：Husk 并不是所有项目都快；本轮 shell 循环和短进程启动比 PRoot 慢。  
**FACT**：当前 Husk 无法完成“打开设置”，因为 `XOpenDisplay(':0')` 失败。  
**UNVERIFIED**：本轮不是完整交替采样，不能当作发布页的最终统计结论。

## 1 · 条件

- 同一台手机、同一系统、同一 Python `3.12.3`、同一探针
  `/tmp/husk-runtime-probe-v2.py`。
- 纯用户态基准使用 `DISPLAY=null`，避免误把图形握手耗时算进文件调用。
- PRoot 桌面当时保持运行；Husk 只有 App 终端，没有自己的图形服务器。
- 每个基准连续 3 轮，取中位数。
- Husk 的干净树通过活动 `rootfs-full` 指向发行树；PRoot 使用它自己的 2.0 rootfs。

这不是完全隔离背景负载的实验室环境。PRoot 桌面本身会带来额外后台进程，Husk 侧没有对等桌面；
因此纯用户态数据只适合判断方向，不适合写“全局固定倍数”。

## 2 · PRoot 原始 3 轮

```json
{"round":1,"display":null,"python":"3.12.3","stat_20000_ms":2558.0132310278714,"open_close_20000_ms":3951.3890009839088,"shell_loop_1000_ms":7.586383959278464,"true_50_ms":301.67569196783006}
{"round":2,"display":null,"python":"3.12.3","stat_20000_ms":2238.041846023407,"open_close_20000_ms":4980.2909999853,"shell_loop_1000_ms":7.684384007006884,"true_50_ms":316.48307596333325}
{"round":3,"display":null,"python":"3.12.3","stat_20000_ms":1780.9944620239548,"open_close_20000_ms":4417.095999990124,"shell_loop_1000_ms":6.9920000387355685,"true_50_ms":309.73407701822}
```

## 3 · Husk 原始 3 轮

```json
{"round":1,"display":null,"python":"3.12.3","stat_20000_ms":710.1732309674844,"open_close_20000_ms":761.8502309778705,"shell_loop_1000_ms":23.708000022452325,"true_50_ms":371.8974619987421}
{"round":2,"display":null,"python":"3.12.3","stat_20000_ms":792.0806149486452,"open_close_20000_ms":752.7760770171881,"shell_loop_1000_ms":28.72392302379012,"true_50_ms":368.31638496369123}
{"round":3,"display":null,"python":"3.12.3","stat_20000_ms":825.1689999597147,"open_close_20000_ms":755.7117689866573,"shell_loop_1000_ms":27.397539000958204,"true_50_ms":369.95753803057596}
```

## 4 · 打开设置

PRoot 在其 GNOME/X11 桌面里连续启动 3 次 `gnome-control-center`，以首个可映射窗口为准：

```text
run 1: 2810.429 ms
run 2: 2859.779 ms
run 3: 1809.177 ms
median: 2810.429 ms
```

映射窗口多次为 `896x762`，标题字段为空。原始记录同时在设备
`/sdcard/proot-xstart.jsonl`。

Husk 跑同一个探针失败了，不是超时：

```text
XOpenDisplay failed (DISPLAY=':0')
RuntimeError: XOpenDisplay failed (DISPLAY=':0')
```

**FACT**：当前 Husk App 没有可用的 X server；Husk 用户态里只有残留的
`rootfs-full/tmp/.X11-unix/X0` socket 文件，没有对应服务。  
**FACT**：PRoot 的 X server 在另一个 Android uid 下，Husk 不能借。  
**结论**：这一项当前应写“Husk 无法完成”，不能写“Husk 更快/更慢”。

## 5 · 这些数字说明什么

1. 文件路径密集负载是 Husk 当前明显占优的部分，和“少一层 PRoot 路径翻译/ptrace”方向一致。
2. shell 循环和短进程启动并不是同一个故事。Husk 的 loader 初始化、GOT/PLT 改写和会话壳层
   仍可能让短任务变慢；不能把“文件 IO 快”扩大成“所有程序都快”。
3. 桌面场景现在被 X server 缺失卡住。继续宣传桌面 GPU 全局加速不会让这项验收变成 PASS。
4. 要成为发布页证据，下一步应做同后台负载的交替采样，并记录温度、频率、进程数、RUN 顺序
   和完整原始日志；本轮不满足这个标准。

## 6 · 发行判断

当前不能升版。发布阻塞是：

```text
H36: Husk 自己的宿主 X server 未接入，gnome-control-center 无法打开；
     因此“桌面 + 桌面 GPU + 打开设置对比”没有 Husk 侧结果。
```

只跑通没有 X 的 GPU 读回和并发长稳，不足以替代这项验收。
