# evidence 102：ScreenCast 取帧探测没做成，以及一次「疑似回归」的证伪

日期：2026-11-23 · RMX3357 / `untrusted_app` · rootfs `files/rootfs-full` · loader
`files/libld39.so`（含 `nr_repatch`，另加 `NR_NO_REPATCH=1` 二分开关）

## 1 想测什么

`docs/design/display-frame-prior-art.md` 的未决项：mutter 46 的 ScreenCast 流到底是
**dmabuf** 还是 **system memory**（BGRx）。这决定帧那一侧能不能零拷贝。计划：
`pipewire` + `gnome-shell --headless` + `husk-rd sc` → `pw-cli enum-params <node>
Spa:Enum:ParamId:Buffers`。

## 2 结果：没测到（FACT）

```
== husk-rd sc ==  (log 0 bytes)
node=
```

同一次运行的日志：

```
(org.gnome.Shell.Screencast:21545): Gjs-CRITICAL: Module file:///usr/share/gnome-shell/org.gnome.Shell.Screencast threw an exception
   _init@resource:///org/gnome/gjs/modules/core/overrides/Gtk.js:22:5
   @gi://Gtk?version=4.0:3:25
(gnome-shell:21497): Gjs-CRITICAL: Attempting to call back into JSAPI during the sweeping phase of GC ...
The offending signal was destroy on Gjs_ui_iconGrid_BaseIcon
double free or corruption (!prev)
```

即：Screencast 这个 Shell 模块在 import `gi://Gtk?version=4.0` 时抛异常，随后 Shell 在 GC
清扫阶段 `double free`。pipewire 本身是好的，只有预期内的警告（没有 system bus → 没有
RTKit；flatpak portal 不存在）。

## 3 这次崩溃跟 `nr_repatch` 无关（对照实验，FACT）

同一套环境，各跑 30 s（`timeout -s KILL`，exit 137 = 到点被杀 = 存活）：

```
on  (repatch 开) rc=137  crash=0  jsErr=0
off (NR_NO_REPATCH=1) rc=137  crash=0  jsErr=0
```

两次都稳。所以 `double free` 不是 repatch 引入的；它与「同时跑 pipewire + 触发
ScreenCast」这一组合相关（UNKNOWN 具体点，下一次单独复现）。为便于二分，`nr_repatch`
加了 `NR_NO_REPATCH=1` 开关（等价于 evidence 100 之前的行为）。

## 4 顺带确认的下一个缺口（FACT）

```
$ gjs -c 'const Gtk = imports.gi.Gtk; print(Gtk.get_major_version())'
(20 s 无任何输出，被 timeout 杀掉)
```

`Gtk-4.0.typelib` 在（570 KB）。所以是 typelib → `libgtk-4.so.1` 这条 import 链在 guest 里
hang 或死，而不是文件缺失。这既是 Screencast 模块起不来的原因，也是 gnome-shell 里
一切 `Gtk` 相关代码（图标网格、截图、锁屏）的前提。**下一次先解决 Gtk import**，
再回头测 Buffer 类型。

## 5 结论

- 「帧是 dmabuf 还是 shared memory」仍然 **UNKNOWN**，不能拿 evidence 99 的 BGRx 当结论。
- 下一步顺序调整：**先修 `gjs import Gtk`**（它是 Shell 内部模块的硬依赖），再测帧。
