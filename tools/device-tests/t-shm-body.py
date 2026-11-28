#!/usr/bin/env python3
import ctypes, ctypes.util, os
libc = ctypes.CDLL("libc.so.6", use_errno=True)
IPC_PRIVATE = 0
IPC_CREAT = 0o1000
shmget = libc.shmget
shmget.restype = ctypes.c_int
shmget.argtypes = [ctypes.c_int, ctypes.c_size_t, ctypes.c_int]
ctypes.set_errno(0)
r = shmget(IPC_PRIVATE, 4096, IPC_CREAT | 0o600)
print("shmget ->", r, "errno=", ctypes.get_errno(), os.strerror(ctypes.get_errno()) if ctypes.get_errno() else "")
ctl = libc.shmat if hasattr(libc, "shmat") else None
if r >= 0:
    shmat = libc.shmat
    shmat.restype = ctypes.c_void_p
    shmat.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
    p = shmat(r, None, 0)
    print("shmat ->", hex(p) if p and p != ctypes.c_void_p(-1).value else p, "errno=", ctypes.get_errno())
# MIT-SHM 扩展是否真的可用
try:
    x11 = ctypes.CDLL("libX11.so.6")
    xext = ctypes.CDLL("libXext.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    d = x11.XOpenDisplay(b":0")
    print("XOpenDisplay ->", hex(d) if d else None)
    xext.XShmQueryExtension.restype = ctypes.c_int
    xext.XShmQueryExtension.argtypes = [ctypes.c_void_p]
    print("XShmQueryExtension ->", xext.XShmQueryExtension(ctypes.c_void_p(d)))
except Exception as e:
    print("X probe failed:", e)
