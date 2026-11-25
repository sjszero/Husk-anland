# 输入 / 窗口管理 现场探针

这四个小工具是查"窗口拖不动"（evidence 196）时写下来的，全部走命令行取值，
不依赖看图，也不依赖 `screenrecord`（这机器上它是死的）。

| 文件 | 干什么 |
|---|---|
| `grabprobe.c` | `LD_PRELOAD` 进 `gnome-shell`：记录 `XIGrabDevice` / `XGrabPointer` / `XGrabKeyboard` 的调用与返回值，并把 mutter 装的 X error handler 包一层，把异步错误也记下来 |
| `movetest.c` | 自己建窗口，按下时向 root 发标准 `_NET_WM_MOVERESIZE`（MOVE / button 1 / source=1）：把 GTK/CSD 那条链路整个排除，只看 mutter |
| `xi2probe.c` | 二分"抓取期间的指针 motion 丢在哪一侧"：`XISelectEvents(XI_Motion)` → 可选 `XIGrabDevice(dev=2)` → 报 `xi_motion` / `xi_button` 计数 |
| `dragfinal.sh` | 依次试"客户端请求 / 标题栏 / 键盘 move op"，每次前后取窗口绝对坐标 |
| `xi2run2.sh` | 跑 `xi2probe` 的 nograb / grab 两组并对比 |
| `kbdmove2.sh` | `alt+F7` + 方向键推窗（证明 grab op 本身是好的） |
| `xevtest.sh` | 用 `xev` 证明指针事件能正常投递到客户端 |

## 编译（guest 里 gcc，rootfs 没有 X11 头，原型都是手写的）

```sh
gcc -O1 -shared -fPIC -o tmp/grabprobe.so tmp/grabprobe.c -ldl
gcc -O1 -o tmp/movetest  tmp/movetest.c  -L/usr/lib/aarch64-linux-gnu -l:libX11.so.6
gcc -O1 -o tmp/xi2probe  tmp/xi2probe.c  -L/usr/lib/aarch64-linux-gnu -l:libXi.so.6 -l:libX11.so.6
```

`XISelectEvents` / `XIGrabDevice` / `XUngrabDevice` 在 `libXi.so.6`，不在 `libX11`。

## 探针怎么挂到 shell 上

`files/linux/xe-husk.sh` 把 `LD_PRELOAD` 指到 `rootfs/tmp/xigrab.so`，所以：

```sh
cp rootfs/tmp/grabprobe.so rootfs/tmp/xigrab.so   # 换成抓取探针
echo -n > rootfs/tmp/xigrab.log
# 重起会话后再操作，再看 rootfs/tmp/xigrab.log
```

## 结论（FACT，2026-11-25）

- `alt+F7` 之后 mutter 真的抓到了设备：`XIGrabDevice dev=2 ret=0` / `dev=3 ret=0`。
- 同一个 grab op，方向键能把窗口从 `1073,464` 推到 `1250,551`。
- 指针拖动，窗口一动不动。
- 自建 XI2 客户端在**持有** `XIGrabDevice(dev=2)` 时，第一条 `XI_Motion` 是收得到的
  （`ev type=35 ext=131 evtype=6`）。

⇒ 抓取、事件投递都成立，问题在 mutter 收到 motion 之后没有作用到窗口上（或者按
deviceid / sourceid 过滤掉了）。下一步是把 XI 事件的 `deviceid/sourceid` 打出来对一遍。
