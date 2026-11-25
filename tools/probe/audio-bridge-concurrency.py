#!/usr/bin/env python3
"""音频桥的并发入口探针（在 guest 里跑，需要应用进程已经在跑）。

判据是「第二个客户端还能不能立刻进来」，不是「命令返回 0」：

  1. 客户端 A 连上 /run/husk/audio-out.sock，**什么都不发**（模拟一个卡住的调用方）；
  2. 客户端 B 连同一个 socket，发 PING，最多等 3 秒；
  3. B 收到 PONG  -> PASS（accept 循环没被 A 堵住）
     B 超时       -> FAIL（head-of-line blocking：A 占着循环，直到它的 30 秒读超时）
     socket 不在  -> SKIP（应用没在跑 / marker 没开，不是这轮要判的东西）

同一份逻辑对采集口 /run/husk/audio-in.sock 也成立：那边连读超时都没有，
一个 idle 客户端能把它永久堵死。

用法（guest 内）：  python3 /root/audio-bridge-concurrency.py [timeout-s]
"""
import socket
import sys
import time

SOCKETS = ["/run/husk/audio-out.sock", "/run/husk/audio-in.sock"]
WAIT = float(sys.argv[1]) if len(sys.argv) > 1 else 3.0

p = f = 0
for path in SOCKETS:
    a = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        a.connect(path)
    except OSError as e:
        print(f"  SKIP  {path}: {e}")
        a.close()
        continue

    b = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        b.connect(path)
        b.settimeout(WAIT)
        b.sendall(b"PING\n")
        data = b.recv(256)
        if b"PONG" in data:
            print(f"  PASS  {path}: second client answered while the first was idle")
            p += 1
        else:
            print(f"  FAIL  {path}: odd answer {data!r}")
            f += 1
    except socket.timeout:
        print(f"  FAIL  {path}: second client blocked {WAIT:.0f}s (head-of-line blocking)")
        f += 1
    except OSError as e:
        print(f"  FAIL  {path}: {e}")
        f += 1
    finally:
        b.close()
        a.close()

print(f"audio-bridge: PASS={p} FAIL={f}")
sys.exit(0 if f == 0 else 1)
