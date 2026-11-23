import ctypes, os, sys, errno
libc = ctypes.CDLL(None, use_errno=True)
O_CREAT, O_RDWR, O_NONBLOCK = 0o100, 2, 0o4000
def call(fn, *a):
    ctypes.set_errno(0)
    r = fn(*a)
    return r, ctypes.get_errno()
ENAME = "/husk-mqtest"
QDIR = "/dev/mqueue/.q/husk-mqtest"
fails = []
def check(label, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + label + (("  " + detail) if detail else ""))
    if not ok: fails.append(label)

# clean slate
if os.path.isdir(QDIR):
    for f in os.listdir(QDIR): libc.unlink(os.path.join(QDIR, f).encode())
    libc.rmdir(QDIR.encode())

q, e = call(libc.mq_open, ENAME.encode(), O_CREAT|O_RDWR|O_NONBLOCK, ctypes.c_uint(0o666))
check("mq_open creates the queue", q >= 0 and e == 0, "fd=%d errno=%d" % (q, e))
check("queue is a directory", os.path.isdir(QDIR))

ok = True; det = []
for prio, msg in ((3,b"third"), (7,b"seventh"), (1,b"first")):
    r, e = call(libc.mq_send, q, msg, ctypes.c_size_t(len(msg)), ctypes.c_uint(prio))
    ok = ok and r == 0 and e == 0
    det.append("p%d->%d/%d" % (prio, r, e))
check("mq_send queues three messages", ok, " ".join(det))
check("three message files on disk", len(os.listdir(QDIR)) == 3, str(len(os.listdir(QDIR))))

buf = ctypes.create_string_buffer(8192); pr = ctypes.c_uint(0)
got = []
for _ in range(3):
    ctypes.set_errno(0)
    n = libc.mq_receive(q, buf, ctypes.c_size_t(8192), ctypes.byref(pr))
    got.append((pr.value, buf.raw[:n] if n >= 0 else None))
check("receive order is priority order (7,3,1)",
      [g[0] for g in got] == [7,3,1] and [g[1] for g in got] == [b"seventh", b"third", b"first"], str(got))
n, e = call(libc.mq_receive, q, buf, ctypes.c_size_t(8192), ctypes.byref(pr))
check("empty queue returns EAGAIN", n == -1 and e == errno.EAGAIN, "n=%d errno=%d" % (n, e))

attr = (ctypes.c_long * 8)()
r, e = call(libc.mq_getattr, q, attr)
check("mq_getattr reports curmsgs=0 after drain", r == 0 and attr[3] == 0,
      "flags=%#x maxmsg=%d msgsize=%d cur=%d" % (attr[0], attr[1], attr[2], attr[3]))
r, e = call(libc.mq_send, q, b"one", ctypes.c_size_t(3), ctypes.c_uint(5))
r, e = call(libc.mq_getattr, q, attr)
check("mq_getattr counts one queued message", r == 0 and attr[3] == 1, "cur=%d" % attr[3])

r, e = call(libc.mq_close, q)
check("mq_close", r == 0 and e == 0, "r=%d errno=%d" % (r, e))
r, e = call(libc.mq_unlink, ENAME.encode())
check("mq_unlink removes a queue that still holds a message", r == 0 and e == 0, "r=%d errno=%d" % (r, e))
check("queue directory is gone", not os.path.exists(QDIR))
r, e = call(libc.mq_unlink, ENAME.encode())
check("mq_unlink of a missing queue is ENOENT", r == -1 and e == errno.ENOENT, "r=%d errno=%d" % (r, e))

print("RESULT %s" % ("PASS" if not fails else "FAIL: " + ", ".join(fails)))
sys.exit(0 if not fails else 1)
