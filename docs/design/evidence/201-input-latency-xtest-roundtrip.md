# 201 · 输入延迟：X 侧往返实测（XTEST 同进程量）

日期：2026-11-25 16:31–16:38（CST）。设备 RMX3357，`:0`，发布版加载器 `libld.so`。
这一份回答"触控和键盘到底有多慢"，并明确它**不能**回答什么。

## 0 · 一句话结论

键盘/指针从注入到 X 客户端收到事件：**中位 1.0–1.2 ms（键）、0.25–3.1 ms（按钮）**，
尖峰 **8–30 ms**。⇒ path-shim 与 X server 的请求处理通路**不是**瓶颈（FACT + INFERENCE）；
尖峰与**合成器帧节拍同量级**（8.3 ms @120 Hz、30.3 ms @33 Hz），不是随机抖动（INFERENCE）。

## 1 · 方法（FACT）

- 探针：`rootfs/tmp/inputlat.c`（一次性工具，**不入库**；源码见 §5），同进程内
  `XTestFakeKeyEvent` / `XTestFakeButtonEvent` + 紧跟 `XSync`，把
  "注入 → X server 处理完 → 回到客户端" 作为往返时间。每次迭代先 `XPending` 清空队列，
  指针提前 warp 进自己创建的 400×300 窗口。
- 规模：n=100 × 3 轮（另有一轮 n=20 预热）。
- 会话：`husk-desktop.sh` 起的会话，实测 `gnome-shell=27348`、`mutter-x11-framework=27422`。
- 编译：guest 内 `usr/bin/gcc -O2 ... -l:libX11.so.6 -l:libXtst.so.6`，走 `xe-plain.sh`。

## 2 · 数据（FACT）

| 轮次 | 项 | min | median | max |
|---|---|---|---|---|
| n=20 预热 | 键 | 0.311 | 0.527 | 1.415 |
| n=20 预热 | 按钮 | 0.222 | 0.414 | 0.853 |
| n=100 第 1 轮 | 键 | 0.190 | 1.185 | 2.408 |
| n=100 第 1 轮 | 按钮 | 0.138 | 2.951 | 30.534 |
| n=100 第 2 轮 | 键 | 0.369 | 1.206 | 8.255 |
| n=100 第 2 轮 | 按钮 | 0.244 | 3.089 | 8.063 |
| n=100 第 3 轮 | 键 | 0.143 | 1.038 | 4.600 |
| n=100 第 3 轮 | 按钮 | 0.152 | 0.254 | 19.196 |

（单位 ms。三轮键中位 1.038 / 1.185 / 1.206，按钮中位 2.951 / 3.089 / 0.254；
按钮中位明显比键散，因为按钮按下会触发 mutter 的 grab 决策，见 198。）

## 3 · 解读

1. **不是转运开销。** 若 path-shim 给每个 X 请求加了往返，min 不可能是 0.14–0.19 ms。
   min 稳定在百微秒级 ⇒ 请求是直通 X server 的（FACT）。
2. **和呈现开销是两笔账。** 每帧 24.8 ms/MP 的 SHM 回读（见 GPU 证据）发生在
   "窗口内容 → X server"，与输入事件的投递通路无关，两者不能互相解释（INFERENCE）。
3. **尖峰 = 帧节拍。** 8.3 / 30.3 ms 正好是 120 Hz / 33 Hz 一帧；max 在 4.6–30.5 ms 之间跳，
   与合成器当前帧率漂移一致。⇒ 事件被批量到下一帧的 grab/frame 之后投递，属一拍延迟，
   不是"卡"（INFERENCE，未做帧计时器对拍验证）。
4. **UNKNOWN（必须说明）：**
   - 不含 `/dev/input`（Lorie touch/pointer 设备）→ X server 这一段；
   - 不含合成器 → 屏幕扫描输出这一段；
   - 因此本表**不是**用户感知的"手指到画面"延迟，只是其中一段的上界。
   - 真手指延迟只能人工掐表，或用 `/dev/input` 事件回放；`adb shell input tap` 驱动不了
     应用侧输入（见 198：应用直接读 `/dev/input`）。

## 4 · 复现

```sh
# 编译（guest 内）
cd files/linux && sh xe-plain.sh usr/bin/gcc -O2 -o /tmp/inputlat /tmp/inputlat.c \
  -L/usr/lib/aarch64-linux-gnu -l:libX11.so.6 -l:libXtst.so.6
# 跑
sh xe-plain.sh usr/bin/env DISPLAY=:0 /tmp/inputlat 100
```

## 5 · 对 0.0.6 的影响

不改代码。结论只进发版说明：**输入通路本身不是瓶颈；可观察到的尖峰来自合成器帧节拍**。
拖窗不动是另一件事（mutter 的 grab op，见 198），不要和延迟混为一谈。
