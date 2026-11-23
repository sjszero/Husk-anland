import ctypes, os, sys, errno, time
libc = ctypes.CDLL(None, use_errno=True)
IPC_PRIVATE, IPC_CREAT, IPC_EXCL = 0, 0o1000, 0o2000
IPC_RMID, IPC_SET, IPC_STAT = 0, 1, 2
IPC_NOWAIT = 0o4000
GETVAL, SETVAL, GETALL, SETALL = 12, 16, 13, 17
MSG_NOERROR = 0o10000
fails = []
def check(label, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + label + (("  " + detail) if detail else ""))
    if not ok: fails.append(label)
def err(): return ctypes.get_errno()

class Sembuf(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("sem_num", ctypes.c_ushort), ("sem_op", ctypes.c_short), ("sem_flg", ctypes.c_short)]
assert ctypes.sizeof(Sembuf) == 6, ctypes.sizeof(Sembuf)

def semop(semid, ops, nowait=False):
    arr = (Sembuf * len(ops))(*[Sembuf(n, o, IPC_NOWAIT if nowait else 0) for n, o in ops])
    ctypes.set_errno(0)
    r = libc.semop(semid, ctypes.byref(arr), ctypes.c_size_t(len(ops)))
    return r, err()

print("== semaphores ==")
ctypes.set_errno(0)
sid = libc.semget(IPC_PRIVATE, 4, 0o600 | IPC_CREAT)
check("semget(IPC_PRIVATE, 4)", sid > 0, "id=%d errno=%d" % (sid, err()))
r = libc.semctl(sid, 0, SETVAL, ctypes.c_int(2))
check("semctl(SETVAL) sets 2", r == 0, "r=%d" % r)
n = libc.semctl(sid, 0, GETVAL)
check("semctl(GETVAL) reads back 2", n == 2, "v=%d" % n)

r, e = semop(sid, [(0, -1)])
v = libc.semctl(sid, 0, GETVAL)
check("P() once decrements to 1", r == 0 and v == 1, "r=%d v=%d" % (r, v))
r, e = semop(sid, [(0, -1)])
v = libc.semctl(sid, 0, GETVAL)
check("P() twice empties it", r == 0 and v == 0, "r=%d v=%d" % (r, v))
r, e = semop(sid, [(0, -1)], nowait=True)
check("P() with IPC_NOWAIT on an empty semaphore is EAGAIN", r == -1 and e == errno.EAGAIN,
      "r=%d errno=%d" % (r, e))
r, e = semop(sid, [(0, 1)])
v = libc.semctl(sid, 0, GETVAL)
check("V() raises it again", r == 0 and v == 1, "r=%d v=%d" % (r, v))

# all-or-nothing: op on an empty semaphore must not apply the other op
libc.semctl(sid, 0, SETVAL, ctypes.c_int(0))
libc.semctl(sid, 1, SETVAL, ctypes.c_int(5))
r, e = semop(sid, [(1, -1), (0, -1)], nowait=True)
v1 = libc.semctl(sid, 1, GETVAL)
check("a blocked op leaves the whole semop unapplied", r == -1 and v1 == 5, "r=%d sem1=%d" % (r, v1))

# cross-process blocking wakeup
libc.semctl(sid, 0, SETVAL, ctypes.c_int(0))
pid = os.fork()
if pid == 0:
    rr, ee = semop(sid, [(0, -1)])
    os._exit(0 if rr == 0 else 1)
time.sleep(0.3)
libc.semctl(sid, 0, SETVAL, ctypes.c_int(1))
_, status = os.waitpid(pid, 0)
check("a blocked P() wakes up when another process posts", os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0,
      "status=%d" % status)

class SemidDs(ctypes.Structure):
    _fields_ = [("pad", ctypes.c_ubyte * 64), ("sem_nsems", ctypes.c_ulong)]
ds = SemidDs()
ctypes.set_errno(0)
r = libc.semctl(sid, 0, IPC_STAT, ctypes.c_void_p(ctypes.addressof(ds)))
check("semctl(IPC_STAT) reports nsems", r == 0 and ds.sem_nsems == 4, "r=%d nsems=%d" % (r, ds.sem_nsems))
r = libc.semctl(sid, 0, IPC_RMID)
check("semctl(IPC_RMID)", r == 0, "r=%d" % r)
ctypes.set_errno(0)
r = libc.semctl(sid, 0, GETVAL)
check("a removed set is gone", r == -1 and err() == errno.EINVAL, "r=%d errno=%d" % (r, err()))

print("== messages ==")
class Msgbuf(ctypes.Structure):
    _fields_ = [("mtype", ctypes.c_long), ("mtext", ctypes.c_char * 256)]
def send(q, t, text):
    b = Msgbuf(); b.mtype = t; b.mtext = text
    ctypes.set_errno(0)
    r = libc.msgsnd(q, ctypes.byref(b), ctypes.c_size_t(len(text)), 0)
    return r, err()
def recv(q, sz=256, typ=0, flg=0):
    b = Msgbuf(); ctypes.set_errno(0)
    r = libc.msgrcv(q, ctypes.byref(b), ctypes.c_size_t(sz), ctypes.c_long(typ), flg)
    return r, err(), b

ctypes.set_errno(0)
q = libc.msgget(IPC_PRIVATE, 0o600 | IPC_CREAT)
check("msgget(IPC_PRIVATE)", q > 0, "id=%d errno=%d" % (q, err()))
for t, txt in ((2, b"two"), (1, b"one"), (3, b"three")):
    r, e = send(q, t, txt)
    check("msgsnd type %d" % t, r == 0 and e == 0, "r=%d errno=%d" % (r, e))
order = []
for _ in range(3):
    r, e, b = recv(q)
    order.append((b.mtype, b.mtext[:r] if r > 0 else None))
check("msgrcv(typ=0) returns by type: 1,2,3", [o[0] for o in order] == [1, 2, 3] and
      [o[1] for o in order] == [b"one", b"two", b"three"], str(order))
r, e, b = recv(q, flg=IPC_NOWAIT)
check("msgrcv on an empty queue with IPC_NOWAIT is ENOMSG", r == -1 and e == errno.ENOMSG,
      "r=%d errno=%d" % (r, e))

send(q, 5, b"five"); send(q, 9, b"nine"); send(q, 1, b"one-again")
r, e, b = recv(q, typ=9)
check("msgrcv(typ=9) picks that type only", r == 4 and b.mtype == 9 and b.mtext[:4] == b"nine",
      "r=%d type=%d text=%r" % (r, b.mtype, b.mtext[:4]))
r, e, b = recv(q, typ=-5)
check("msgrcv(typ=-5) picks the lowest type <= 5", r == 9 and b.mtype == 1,
      "r=%d type=%d" % (r, b.mtype))
r, e, b = recv(q, typ=-5)
check("then the next one <= 5 is type 5", r == 4 and b.mtype == 5, "r=%d type=%d" % (r, b.mtype))

send(q, 7, b"x" * 100)
r, e, b = recv(q, 10, 7)
check("a message longer than the buffer is E2BIG and stays", r == -1 and e == errno.E2BIG,
      "r=%d errno=%d" % (r, e))
r, e, b = recv(q, 10, 7, MSG_NOERROR)
check("MSG_NOERROR truncates and delivers", r == 10 and b.mtext[:10] == b"x" * 10,
      "r=%d" % r)

buf = ctypes.create_string_buffer(120)
qnum = ctypes.c_ulong.from_buffer(buf, 80).value
ctypes.set_errno(0)
r = libc.msgctl(q, IPC_STAT, buf)
qnum = ctypes.c_ulong.from_buffer(buf, 80).value
check("msgctl(IPC_STAT) counts queued messages", r == 0 and qnum == 0, "r=%d qnum=%d" % (r, qnum))
r = libc.msgctl(q, IPC_RMID)
check("msgctl(IPC_RMID)", r == 0, "r=%d" % r)
check("queue directory is gone", not os.path.exists("/run/husk/ipc/msg/%d" % q))

print("RESULT %s" % ("PASS" if not fails else "FAIL: " + ", ".join(fails)))
sys.exit(0 if not fails else 1)
