import ctypes, os, sys, errno, time
libc = ctypes.CDLL(None, use_errno=True)
libc.shmat.restype = ctypes.c_void_p
libc.shmat.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
libc.shmdt.argtypes = [ctypes.c_void_p]
libc.shmctl.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
IPC_PRIVATE, IPC_CREAT, IPC_EXCL, IPC_RMID, IPC_STAT = 0, 0o1000, 0o2000, 0, 2
fails = []
def check(label, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + label + (("  " + detail) if detail else ""))
    if not ok: fails.append(label)
def err(): return ctypes.get_errno()

ctypes.set_errno(0)
sid = libc.shmget(IPC_PRIVATE, 4096, 0o600 | IPC_CREAT)
check("shmget(IPC_PRIVATE) returns an id", sid > 0, "id=%d errno=%d" % (sid, err()))
p = libc.shmat(sid, None, 0)
check("shmat maps the segment", bool(p) and p != ctypes.c_void_p(-1).value, "addr=%s" % hex(p or 0))
ctypes.memmove(p, b"HELLO-SHM\0", 10)
libc.shmdt(p)
p2 = libc.shmat(sid, None, 0)
check("data survives a detach/attach cycle", ctypes.string_at(p2, 10) == b"HELLO-SHM\0",
      repr(ctypes.string_at(p2, 10)))
buf = ctypes.create_string_buffer(112)
ctypes.set_errno(0)
r = libc.shmctl(sid, IPC_STAT, buf)
segsz = ctypes.c_ulong.from_buffer(buf, 48).value
nattch = ctypes.c_ulong.from_buffer(buf, 88).value
check("shmctl(IPC_STAT) reports size and attachments", r == 0 and segsz == 4096 and nattch == 1,
      "r=%d segsz=%d nattch=%d" % (r, segsz, nattch))
libc.shmdt(p2)

# key-based, and cross-process
key = 0x484B5331
ctypes.set_errno(0)
kid = libc.shmget(key, 8192, 0o600 | IPC_CREAT)
check("shmget(key) creates", kid > 0, "id=%d errno=%d" % (kid, err()))
ctypes.set_errno(0)
same = libc.shmget(key, 0, 0o600)
check("shmget(same key) returns the same segment", same == kid, "%d vs %d errno=%d" % (same, kid, err()))
ctypes.set_errno(0)
r = libc.shmget(key, 8192, 0o600 | IPC_CREAT | IPC_EXCL)
check("IPC_CREAT|IPC_EXCL on an existing key is EEXIST", r == -1 and err() == errno.EEXIST,
      "r=%d errno=%d" % (r, err()))

pid = os.fork()
if pid == 0:
    pp = libc.shmat(kid, None, 0)
    if not pp or pp == ctypes.c_void_p(-1).value:
        os._exit(9)
    ctypes.memmove(pp, b"FROM-CHILD\0", 11)
    libc.shmdt(pp)
    os._exit(0)
_, status = os.waitpid(pid, 0)
check("child process attached and exited cleanly", os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0,
      "status=%d" % status)
pp = libc.shmat(kid, None, 0)
if not pp or pp == ctypes.c_void_p(-1).value:
    check("parent attaches the child's segment", False, "shmat -> -1")
    print("RESULT FAIL: parent could not attach")
    sys.exit(1)
got = ctypes.string_at(pp, 11)
check("parent sees what the child wrote (real sharing)", got == b"FROM-CHILD\0", repr(got))

ctypes.set_errno(0)
r = libc.shmctl(kid, IPC_RMID, None)
check("shmctl(IPC_RMID) while attached returns 0", r == 0, "r=%d errno=%d" % (r, err()))
got2 = ctypes.string_at(pp, 11)
check("segment stays readable while still attached", got2 == b"FROM-CHILD\0", repr(got2))
libc.shmdt(pp)
time.sleep(0.1)
gone = not os.path.exists("/run/husk/ipc/shm/%d" % kid)
check("segment file disappears with the last detach", gone, "id=%d" % kid)
ctypes.set_errno(0)
r = libc.shmget(key, 0, 0o600)
check("key is gone after the last detach", r == -1 and err() == errno.ENOENT, "r=%d errno=%d" % (r, err()))

libc.shmctl(sid, IPC_RMID, None)
print("RESULT %s" % ("PASS" if not fails else "FAIL: " + ", ".join(fails)))
sys.exit(0 if not fails else 1)
