# 185 · 「窗口拖不动 / 光标不动」实测：注入到了 X，但按下点用的是上一次的光标位置

日期：2026-11-25 12:26–12:40（CST）。设备 RMX3357（MT6893 / Android 13），应用
`com.linuxhub.myth.husk`，会话 `gnome-shell --x11 --replace --mode=linuxhub-ubuntu-min`，
X 屏幕 `2400x1080`（`xwininfo`：root 0x511）。

## 1 · 先量清楚现状（FACT）

xwininfo 拿到的窗口几何（X 坐标）：

| 窗口 | 几何 |
|---|---|
| `gnome-text-editor` | `1000x780+120-21` |
| `gnome-calculator` | `482x614+998-21` → 标题栏只有 `y≈0..27`，窗口上沿在屏幕外 21 px |

三次注入实验，全部用 `adb shell input swipe/tap`（真实 MotionEvent），结果从
`xdotool getmouselocation` / `xwininfo` / X 根窗口截帧读回来：

| # | 注入 | 结果 |
|---|---|---|
| 1 | `input swipe 1200 540 → 760 1000 600`，再 `input tap 760 1000` | **光标确实到了 `760,1000`**；tap 在光标处生效（应用网格打开，根窗口 diff bbox `(3,32)-(1461,574)`） |
| 2 | `input swipe 1200 540 → 1150 60`，再 `input swipe 1150 60 → 1450 300` | 根窗口 diff bbox 只有 `(1281,11)-(1290,22)`（顶栏时钟）→ **计算器没动**；但那次 y=60 落在计算器的显示区，不是标题栏，结论无效 |
| 3 | `input swipe 760 1000 → 1200 10`（瞄准标题栏），再 `input swipe 1200 10 → 1450 400` | 计算器仍是 `+998-21`；**而且光标仍停在 `760,1000`** → 第二、三次 swipe 连光标都没动 |

所以能确定的：

- 事件通路是通的（tap 能点中、光标能被第一次 swipe 移动）；
- **连续手势里，后面的 swipe 会整条丢掉**（光标位置一点没变）；
- 光标在标题栏上时按下拖动，窗口也没动。

## 2 · 读代码：按下点用的是「上一次的光标位置」，不是手指位置（FACT，代码级）

2.0 侧的输入策略（本仓库不改它，只定位）：`LinuxHub-2.0/termux_ubuntu_rebuild/
termux_x11_source/lorie/src/main/java/com/termux/x11/input/TouchInputHandler.java`

```java
case MotionEvent.ACTION_DOWN:
    ...
    } else if (!isButtonHandled) {
        onTap = true;
        mInjector.sendMouseEvent(mRenderData.getCursorPosition(), InputStub.BUTTON_LEFT, true, false);
    }
case MotionEvent.ACTION_MOVE:
    if (mIsScrolling && isScrollingEvent(e)) ...
    else if ((mIsDragging && hasFlags(e, 0x4000000)) || onTap) {
        mRenderData.mapScreenPoint(e.getX(), e.getY(), mappedPoint);
        if (mRenderData.setCursorPosition(mappedPoint[0], mappedPoint[1]))
            mInjector.sendCursorMove(mappedPoint[0], mappedPoint[1], false);
    }
```

两件事同时成立：

1. **按下发生在 `getCursorPosition()`**，也就是**上一个手势结束时**光标所在处，而不是手指
   按下的地方。手指按在标题栏、光标在别处时，按下就落在别处——于是一个"拖窗口"的手势要么
   拖错对象，要么什么都没抓住；光标被 MOVE 拉走以后，窗口自然"不对齐"。
2. **光标的移动被 `onTap` 门控**：只有 `onTap==true`（或 `mIsDragging` 且带
   `FLAG 0x4000000`）才 `sendCursorMove`。任何让 `onTap` 提前变 false 的路径（`ACTION_UP`、
   `ACTION_CANCEL`、`ACTION_HOVER_EXIT`，或者中间插进来一个 `ACTION_BUTTON_PRESS` 把
   `onTap` 重写成 false——见 `DexListener.onTouch` 第一个 case）都会让后续 MOVE 整条丢掉，
   这就是实验 3 里"第二次 swipe 光标一点没动"的形态。
3. `SimulatedTouchInputStrategy.onTap()`（`touchMode=2`）同样用 `getCursorPosition()` 而不是
   事件坐标，所以模式 2 连"点哪落哪"都不成立（evidence 148 §2 已经测过）。

## 3 · 结论（分层）

| 层 | 结论 |
|---|---|
| 注入通路（XI2 → X server → mutter） | 通。tap 能在光标处点中，键盘能打进去（evidence 148、180） |
| 手势语义（2.0 侧 `TouchInputHandler`） | **按下点用光标位置、光标移动被门控** —— 这是"拖不动 / 不对齐 / 连续手势丢"的直接原因 |
| 运行时层（Husk 仓库） | 这一层**管不到**：事件在 Android 侧就被翻译成了"光标位置 + 按钮"，X server 收到的是已经错位的坐标 |

修法（2.0 侧，两处都小）：

1. `ACTION_DOWN` 先把 `mapScreenPoint(e.getX(), e.getY())` 的结果 `setCursorPosition` 并
   `sendCursorMove`，再用这个坐标 `sendMouseEvent(..., BUTTON_LEFT, true)`——即"按下点 = 手指
   点"，这同时修掉 trackpad（`touchMode=1`）的拖窗口和模拟触摸屏（`touchMode=2`）的点不中；
2. `onTap` 不要在 `ACTION_BUTTON_PRESS` 里被无条件重写成 `false`；改成"按钮事件只处理按钮，
   不改手势状态机"，避免连续手势后半段整条丢。

## 4 · 不能说明什么

- 没有改 2.0 代码验证（本仓库不改 2.0）；上面两条是按代码路径推出的修法，**未实测**；
- `touchMode=3`（`NullInputStrategy` → 真 XI2 touch）仍未测；
- "对齐/吸附"只确认了窗口没被拖动，没单独验 tiling-assistant 的吸附边距。
