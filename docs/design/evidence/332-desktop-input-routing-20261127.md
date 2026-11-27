# 332：Android 输入路由接入内嵌 X11 桌面

日期：2026-11-27

## 结论

- **FACT**：修前 Android `drain_input()` 把所有键盘和触摸事件都送往
  `crate::input::key/motion`（evdev socket / 终端路径）。编译器的
  `desktop::key/touch/mouse is never used` 警告直接证明 Xlorie 的原生输入 ABI
  没有接入应用输入回调。
- **FACT**：现在 `desktop::active()` 为真时，键盘按下/抬起和单指/多指触摸都直接进入
  Xlorie；终端输入只会在内嵌桌面未激活时接收事件，避免键盘重复注入。
- **FACT**：多指处理不再固定使用 pointer 0。`ACTION_POINTER_DOWN/UP` 使用
  `AMotionEvent_getActionIndex()` 找到事件指针，`ACTION_MOVE/CANCEL` 遍历全部活跃指针，
  每个事件携带 `AMotionEvent_getPointerId()`。
- **FACT**：鼠标/触控板输入另走 Xlorie pointer ABI：绝对鼠标发送屏幕坐标，相对鼠标读取
  `AXIS_RELATIVE_X/Y`，滚轮按 120 单位映射到按钮 4；不再把鼠标移动误当触摸屏。
- **FACT**：触摸点击结束后触发 NativeActivity 软键盘；键盘事件再通过正常 input queue 送到
  X11。拖动超过 8 px 不弹软键盘。
- **FACT**：BACK 键在桌面模式下只负责结束 Activity，不会同时发送给 X11 客户端。
- **UNVERIFIED**：真机点击、拖动、双指缩放、相对触控板、滚轮和软键盘文本输入尚未验证；
  设备仍处于异常高负载，本轮没有启动 Husk。

## 修前问题

修前编译 `app-rs` 时的明确证据：

```text
warning: field `send_touch`, `send_mouse`, and `send_key` are never read
warning: function `key` is never used
warning: function `touch` is never used
warning: function `mouse` is never used
```

旧 `drain_input()` 对所有事件无条件执行：

```rust
crate::input::key(kc, action == AKEY_EVENT_ACTION_DOWN);
crate::input::motion(action, x, y, pointers);
```

这条路径是给独立的 evdev/socket 输入消费者用的。内嵌桌面已经成为主会话后，它既不知道
X11 的 pointer id，也不会调用 Xlorie 的 `husk_xlorie_send_touch/key`。

## 修复

1. 键盘在桌面模式下映射 Android keycode → Linux keycode，再调用
   `husk_xlorie_send_key(code, down)`；只发送一次，不叠加 evdev。
2. 单指 `DOWN/MOVE/UP`、多指 `POINTER_DOWN/MOVE/POINTER_UP` 和 `CANCEL` 直接送到
   Xlorie touch device。
3. pointer down/up 使用 action index；move/cancel 遍历全部 pointer；坐标和 id 都来自当前
   Android MotionEvent。
4. `AINPUT_SOURCE_MOUSE` 使用绝对坐标；`AINPUT_SOURCE_MOUSE_RELATIVE` 读取相对轴；
   `ACTION_SCROLL` 映射到 Xlorie 的滚轮路径。
5. 无拖动的触摸点击在抬起后显示 NativeActivity 软键盘。
6. BACK 在桌面模式下关闭 Activity，避免把系统返回键误送给 GUI。
7. 新增 `tools/ci/check-desktop-input-routing.sh`，要求路由分支、Linux keycode 传递、鼠标/
   相对轴和多指针 FFI 持续存在；已接入 `make check`。

## 验证

```text
cargo check app-rs --target aarch64-linux-android --lib --bin husk-session-guard  PASS
make check                                                                        PASS
desktop input routing: PASS (touch + mouse + multi-pointer + key edges)
make apk                                                                          PASS
audit-apk-version.sh                                                               PASS  0.0.36 / 36
audit-apk-contents.sh                                                              PASS  4/4 native pieces
audit-xlorie-asset.sh                                                              PASS 76b4abc…
app-debug.apk sha256  9b6f8c6215d2c662fcc3cb1bb116026864199bc36639441e806803769601a0c8
```

编译后 `desktop::key`、`desktop::touch` 和 `desktop::mouse` 的未使用警告全部消失；
`x11_fd` 仍是未接入的遗留接口，不能据此宣称独立 X11 fd 轮询已验证。

## 真机健康检查

仅执行只读检查，没有启动 Husk：

```text
/proc/loadavg     22.54 38.52 41.91 1/31765 2816
MemAvailable      3,271,020 kB
process/threads   31,765
Husk PID          absent
abnormalboot      2026-11-27 10:43:54:zygote reboot by netd
```

设备仍不具备可靠启动完整桌面的条件。不得为了本次测试清理或杀死
`com.linuxhub.myth`、`com.linuxhub.myth:x11` 或 `u0_a300`。

## 待验证

- 单击、拖动窗口、双指缩放/滚动、相对触控板与滚轮；
- 软键盘与硬件键盘的按键、修饰键、文本输入；
- BACK 退出行为；
- 输入期间 Xlorie 帧率和 CPU 占用；
- 进入桌面后 `screencap` 非黑及 MIT-SHM 行为。

## 边界

本次没有 bump `VERSION`、没有 tag、没有 Release。输入直连只覆盖 Xlorie 内嵌桌面；
独立 evdev socket 仍是另一条路径，二者不能同时向同一会话注入。
