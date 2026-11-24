//! Syscall-surface semantics for the guest, one subcommand per family:
//!
//!   ipc-probe mq        POSIX message queues (mq_open/send/receive/getattr/…)
//!   ipc-probe shm       SysV shared memory (shmget/shmat/shmdt/shmctl)
//!   ipc-probe sem-msg   SysV semaphores and message queues
//!
//! These used to be three ctypes scripts (`06-posix-mqueue.py`,
//! `07-sysv-shm.py`, `08-sysv-sem-msg.py`). They call libc directly, no ctypes
//! and no python3 in the guest. Prints one `PASS `/`FAIL ` line per assertion and
//! a final `RESULT PASS|FAIL: …`, same text as before so the docs stay valid.
//!
//! The struct offsets are the ones the ctypes scripts used and that passed on
//! this kernel (aarch64 glibc 2.39, kernel 4.19): `shmid_ds.shm_segsz` at 48,
//! `.shm_nattch` at 88, `semid_ds.sem_nsems` at 64, `msqid_ds.msg_qnum` at 80.

#[cfg(target_os = "linux")]
mod linux {
    use std::os::raw::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};

    extern "C" {
        fn __errno_location() -> *mut c_int;
        fn mq_open(name: *const c_char, flags: c_int, mode: c_uint, attr: *const MqAttr) -> c_int;
        fn mq_send(mqdes: c_int, msg: *const c_char, len: usize, prio: c_uint) -> c_int;
        fn mq_receive(mqdes: c_int, msg: *mut c_char, len: usize, prio: *mut c_uint) -> isize;
        fn mq_getattr(mqdes: c_int, attr: *mut MqAttr) -> c_int;
        fn mq_close(mqdes: c_int) -> c_int;
        fn mq_unlink(name: *const c_char) -> c_int;

        fn shmget(key: c_int, size: usize, flags: c_int) -> c_int;
        fn shmat(shmid: c_int, addr: *const c_void, flags: c_int) -> *mut c_void;
        fn shmdt(addr: *const c_void) -> c_int;
        fn shmctl(shmid: c_int, cmd: c_int, buf: *mut c_void) -> c_int;

        fn semget(key: c_int, nsems: c_int, flags: c_int) -> c_int;
        fn semop(semid: c_int, ops: *const Sembuf, nops: usize) -> c_int;
        fn semctl(semid: c_int, semnum: c_int, cmd: c_int, ...) -> c_int;

        fn msgget(key: c_int, flags: c_int) -> c_int;
        fn msgsnd(msqid: c_int, msgp: *const MsgBuf, msgsz: usize, flags: c_int) -> c_int;
        fn msgrcv(
            msqid: c_int,
            msgp: *mut MsgBuf,
            msgsz: usize,
            msgtyp: c_long,
            flags: c_int,
        ) -> isize;
        fn msgctl(msqid: c_int, cmd: c_int, buf: *mut c_void) -> c_int;

        fn fork() -> c_int;
        fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct MqAttr {
        flags: c_long,
        maxmsg: c_long,
        msgsize: c_long,
        curmsgs: c_long,
        spare: [c_long; 4],
    }

    #[repr(C, packed)]
    #[derive(Default, Clone, Copy)]
    struct Sembuf {
        num: c_ushort,
        op: c_short,
        flg: c_short,
    }

    #[repr(C)]
    struct MsgBuf {
        mtype: c_long,
        mtext: [u8; 256],
    }

    impl Default for MsgBuf {
        fn default() -> Self {
            MsgBuf {
                mtype: 0,
                mtext: [0; 256],
            }
        }
    }

    const IPC_PRIVATE: c_int = 0;
    const IPC_CREAT: c_int = 0o1000;
    const IPC_EXCL: c_int = 0o2000;
    const IPC_RMID: c_int = 0;
    const IPC_SET: c_int = 1;
    const IPC_STAT: c_int = 2;
    const IPC_NOWAIT: c_int = 0o4000;
    const SETVAL: c_int = 16;
    const GETVAL: c_int = 12;
    const MSG_NOERROR: c_int = 0o10000;

    const O_CREAT: c_int = 0o100;
    const O_RDWR: c_int = 2;
    const O_NONBLOCK: c_int = 0o4000;

    fn zero_errno() {
        unsafe { *__errno_location() = 0 };
    }

    fn errno() -> c_int {
        unsafe { *__errno_location() }
    }

    const EAGAIN: c_int = 11;
    const EEXIST: c_int = 17;
    const ENOENT: c_int = 2;
    const EINVAL: c_int = 22;
    const ENOMSG: c_int = 42;
    const E2BIG: c_int = 7;

    struct Report {
        fails: Vec<String>,
    }

    impl Report {
        fn new() -> Report {
            Report { fails: Vec::new() }
        }

        fn check(&mut self, label: &str, ok: bool, detail: &str) {
            if ok {
                print!("PASS {label}");
            } else {
                print!("FAIL {label}");
                self.fails.push(label.to_string());
            }
            if !detail.is_empty() {
                print!("  {detail}");
            }
            println!();
        }

        fn finish(self) -> i32 {
            if self.fails.is_empty() {
                println!("RESULT PASS");
                0
            } else {
                println!("RESULT FAIL: {}", self.fails.join(", "));
                1
            }
        }
    }

    fn cstr(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).expect("no NUL in names")
    }

    /// Read a `c_ulong` out of a byte buffer at a fixed offset (the offsets the
    /// ctypes version used, verified on this kernel).
    fn ulong_at(buf: &[u8], offset: usize) -> c_ulong {
        let mut word = [0u8; std::mem::size_of::<c_ulong>()];
        word.copy_from_slice(&buf[offset..offset + std::mem::size_of::<c_ulong>()]);
        c_ulong::from_ne_bytes(word)
    }

    // ------------------------------------------------------------------ mqueue

    fn run_mq() -> i32 {
        const NAME: &str = "/husk-mqtest";
        const QDIR: &str = "/dev/mqueue/.q/husk-mqtest";
        let mut r = Report::new();

        // Clean slate: the queue dir holds one file per message.
        if std::path::Path::new(QDIR).is_dir() {
            if let Ok(entries) = std::fs::read_dir(QDIR) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
            let _ = std::fs::remove_dir(QDIR);
        }

        zero_errno();
        let name = cstr(NAME);
        let q = unsafe {
            mq_open(
                name.as_ptr(),
                O_CREAT | O_RDWR | O_NONBLOCK,
                0o666,
                std::ptr::null(),
            )
        };
        r.check(
            "mq_open creates the queue",
            q >= 0 && errno() == 0,
            &format!("fd={} errno={}", q, errno()),
        );
        r.check(
            "queue is a directory",
            std::path::Path::new(QDIR).is_dir(),
            "",
        );

        let mut all_ok = true;
        let mut detail = String::new();
        for (prio, msg) in [(3u32, "third"), (7, "seventh"), (1, "first")] {
            zero_errno();
            let rc = unsafe { mq_send(q, msg.as_ptr() as *const c_char, msg.len(), prio) };
            all_ok &= rc == 0 && errno() == 0;
            detail.push_str(&format!("p{prio}->{rc}/{} ", errno()));
        }
        r.check("mq_send queues three messages", all_ok, detail.trim_end());
        let count = std::fs::read_dir(QDIR).map(|d| d.count()).unwrap_or(0);
        r.check(
            "three message files on disk",
            count == 3,
            &count.to_string(),
        );

        let mut buf = [0u8; 8192];
        let mut order: Vec<(u32, Vec<u8>)> = Vec::new();
        for _ in 0..3 {
            zero_errno();
            let mut prio: c_uint = 0;
            let n = unsafe { mq_receive(q, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut prio) };
            let payload = if n >= 0 {
                buf[..n as usize].to_vec()
            } else {
                Vec::new()
            };
            order.push((prio, payload));
        }
        let prios: Vec<u32> = order.iter().map(|o| o.0).collect();
        let payloads_ok =
            order
                .iter()
                .map(|o| o.1.as_slice())
                .eq([b"seventh".as_slice(), b"third", b"first"]);
        r.check(
            "receive order is priority order (7,3,1)",
            prios == vec![7, 3, 1] && payloads_ok,
            &format!("{prios:?}"),
        );
        zero_errno();
        let mut prio: c_uint = 0;
        let n = unsafe { mq_receive(q, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut prio) };
        r.check(
            "empty queue returns EAGAIN",
            n == -1 && errno() == EAGAIN,
            &format!("n={n} errno={}", errno()),
        );

        let mut attr = MqAttr::default();
        zero_errno();
        let rc = unsafe { mq_getattr(q, &mut attr) };
        r.check(
            "mq_getattr reports curmsgs=0 after drain",
            rc == 0 && attr.curmsgs == 0,
            &format!(
                "flags={:#x} maxmsg={} msgsize={} cur={}",
                attr.flags, attr.maxmsg, attr.msgsize, attr.curmsgs
            ),
        );
        zero_errno();
        unsafe { mq_send(q, b"one".as_ptr() as *const c_char, 3, 5) };
        zero_errno();
        let rc = unsafe { mq_getattr(q, &mut attr) };
        r.check(
            "mq_getattr counts one queued message",
            rc == 0 && attr.curmsgs == 1,
            &format!("cur={}", attr.curmsgs),
        );

        zero_errno();
        let rc = unsafe { mq_close(q) };
        r.check(
            "mq_close",
            rc == 0 && errno() == 0,
            &format!("r={rc} errno={}", errno()),
        );
        zero_errno();
        let rc = unsafe { mq_unlink(name.as_ptr()) };
        r.check(
            "mq_unlink removes a queue that still holds a message",
            rc == 0 && errno() == 0,
            &format!("r={rc} errno={}", errno()),
        );
        r.check(
            "queue directory is gone",
            !std::path::Path::new(QDIR).exists(),
            "",
        );
        zero_errno();
        let rc = unsafe { mq_unlink(name.as_ptr()) };
        r.check(
            "mq_unlink of a missing queue is ENOENT",
            rc == -1 && errno() == ENOENT,
            &format!("r={rc} errno={}", errno()),
        );
        r.finish()
    }

    // ------------------------------------------------------------------- SysV shm

    fn shmat_ok(addr: *mut c_void) -> bool {
        !addr.is_null() && addr as isize != -1
    }

    fn run_shm() -> i32 {
        let mut r = Report::new();

        zero_errno();
        let sid = unsafe { shmget(IPC_PRIVATE, 4096, 0o600 | IPC_CREAT) };
        r.check(
            "shmget(IPC_PRIVATE) returns an id",
            sid > 0,
            &format!("id={sid} errno={}", errno()),
        );
        let p = unsafe { shmat(sid, std::ptr::null(), 0) };
        r.check(
            "shmat maps the segment",
            shmat_ok(p),
            &format!("addr={:#x}", p as usize),
        );
        unsafe { std::ptr::copy_nonoverlapping(b"HELLO-SHM\0".as_ptr(), p as *mut u8, 10) };
        unsafe { shmdt(p) };
        let p2 = unsafe { shmat(sid, std::ptr::null(), 0) };
        let mut got = [0u8; 10];
        if shmat_ok(p2) {
            unsafe { std::ptr::copy_nonoverlapping(p2 as *const u8, got.as_mut_ptr(), 10) };
        }
        r.check(
            "data survives a detach/attach cycle",
            &got == b"HELLO-SHM\0",
            &format!("{:?}", String::from_utf8_lossy(&got)),
        );
        let mut buf = [0u8; 112];
        zero_errno();
        let rc = unsafe { shmctl(sid, IPC_STAT, buf.as_mut_ptr() as *mut c_void) };
        let segsz = ulong_at(&buf, 48);
        let nattch = ulong_at(&buf, 88);
        r.check(
            "shmctl(IPC_STAT) reports size and attachments",
            rc == 0 && segsz == 4096 && nattch == 1,
            &format!("r={rc} segsz={segsz} nattch={nattch}"),
        );
        unsafe { shmdt(p2) };

        // Key-based, then cross-process.
        const KEY: c_int = 0x484B5331;
        zero_errno();
        let kid = unsafe { shmget(KEY, 8192, 0o600 | IPC_CREAT) };
        r.check(
            "shmget(key) creates",
            kid > 0,
            &format!("id={kid} errno={}", errno()),
        );
        zero_errno();
        let same = unsafe { shmget(KEY, 0, 0o600) };
        r.check(
            "shmget(same key) returns the same segment",
            same == kid,
            &format!("{same} vs {kid} errno={}", errno()),
        );
        zero_errno();
        let rc = unsafe { shmget(KEY, 8192, 0o600 | IPC_CREAT | IPC_EXCL) };
        r.check(
            "IPC_CREAT|IPC_EXCL on an existing key is EEXIST",
            rc == -1 && errno() == EEXIST,
            &format!("r={rc} errno={}", errno()),
        );

        let pid = unsafe { fork() };
        if pid == 0 {
            let pp = unsafe { shmat(kid, std::ptr::null(), 0) };
            if !shmat_ok(pp) {
                std::process::exit(9);
            }
            unsafe {
                std::ptr::copy_nonoverlapping(b"FROM-CHILD\0".as_ptr(), pp as *mut u8, 11);
                shmdt(pp);
            }
            std::process::exit(0);
        }
        let mut status: c_int = 0;
        unsafe { waitpid(pid, &mut status, 0) };
        let exited = (status & 0x7f) == 0;
        r.check(
            "child process attached and exited cleanly",
            exited && (status >> 8) == 0,
            &format!("status={status}"),
        );
        let pp = unsafe { shmat(kid, std::ptr::null(), 0) };
        if !shmat_ok(pp) {
            r.check("parent attaches the child's segment", false, "shmat -> -1");
            println!("RESULT FAIL: parent could not attach");
            return 1;
        }
        let mut child = [0u8; 11];
        unsafe { std::ptr::copy_nonoverlapping(pp as *const u8, child.as_mut_ptr(), 11) };
        r.check(
            "parent sees what the child wrote (real sharing)",
            &child == b"FROM-CHILD\0",
            &format!("{:?}", String::from_utf8_lossy(&child)),
        );

        zero_errno();
        let rc = unsafe { shmctl(kid, IPC_RMID, std::ptr::null_mut()) };
        r.check(
            "shmctl(IPC_RMID) while attached returns 0",
            rc == 0,
            &format!("r={rc} errno={}", errno()),
        );
        let mut still = [0u8; 11];
        unsafe { std::ptr::copy_nonoverlapping(pp as *const u8, still.as_mut_ptr(), 11) };
        r.check(
            "segment stays readable while still attached",
            &still == b"FROM-CHILD\0",
            &format!("{:?}", String::from_utf8_lossy(&still)),
        );
        unsafe { shmdt(pp) };
        std::thread::sleep(std::time::Duration::from_millis(100));
        let node = format!("/run/husk/ipc/shm/{kid}");
        r.check(
            "segment file disappears with the last detach",
            !std::path::Path::new(&node).exists(),
            &format!("id={kid}"),
        );
        zero_errno();
        let rc = unsafe { shmget(KEY, 0, 0o600) };
        r.check(
            "key is gone after the last detach",
            rc == -1 && errno() == ENOENT,
            &format!("r={rc} errno={}", errno()),
        );

        unsafe { shmctl(sid, IPC_RMID, std::ptr::null_mut()) };
        r.finish()
    }

    // --------------------------------------------------------- SysV semaphores

    fn semop_nowait(semid: c_int, ops: &[(u16, i16)], nowait: bool) -> (c_int, c_int) {
        let packed: Vec<Sembuf> = ops
            .iter()
            .map(|(n, o)| Sembuf {
                num: *n,
                op: *o,
                flg: if nowait { IPC_NOWAIT as c_short } else { 0 },
            })
            .collect();
        zero_errno();
        let rc = unsafe { semop(semid, packed.as_ptr(), packed.len()) };
        (rc, errno())
    }

    fn sem_setval(semid: c_int, num: c_int, value: c_int) -> c_int {
        unsafe { semctl(semid, num, SETVAL, value) }
    }

    fn sem_getval(semid: c_int, num: c_int) -> c_int {
        unsafe { semctl(semid, num, GETVAL) }
    }

    fn run_sem_msg() -> i32 {
        let mut r = Report::new();

        println!("== semaphores ==");
        zero_errno();
        let sid = unsafe { semget(IPC_PRIVATE, 4, 0o600 | IPC_CREAT) };
        r.check(
            "semget(IPC_PRIVATE, 4)",
            sid > 0,
            &format!("id={sid} errno={}", errno()),
        );
        let rc = sem_setval(sid, 0, 2);
        r.check("semctl(SETVAL) sets 2", rc == 0, &format!("r={rc}"));
        let value = sem_getval(sid, 0);
        r.check(
            "semctl(GETVAL) reads back 2",
            value == 2,
            &format!("v={value}"),
        );

        let (rc, _) = semop_nowait(sid, &[(0, -1)], false);
        let value = sem_getval(sid, 0);
        r.check(
            "P() once decrements to 1",
            rc == 0 && value == 1,
            &format!("r={rc} v={value}"),
        );
        let (rc, _) = semop_nowait(sid, &[(0, -1)], false);
        let value = sem_getval(sid, 0);
        r.check(
            "P() twice empties it",
            rc == 0 && value == 0,
            &format!("r={rc} v={value}"),
        );
        let (rc, err) = semop_nowait(sid, &[(0, -1)], true);
        r.check(
            "P() with IPC_NOWAIT on an empty semaphore is EAGAIN",
            rc == -1 && err == EAGAIN,
            &format!("r={rc} errno={err}"),
        );
        let (rc, _) = semop_nowait(sid, &[(0, 1)], false);
        let value = sem_getval(sid, 0);
        r.check(
            "V() raises it again",
            rc == 0 && value == 1,
            &format!("r={rc} v={value}"),
        );

        // all-or-nothing: an op on an empty semaphore must not apply the other op
        sem_setval(sid, 0, 0);
        sem_setval(sid, 1, 5);
        let (rc, _) = semop_nowait(sid, &[(1, -1), (0, -1)], true);
        let v1 = sem_getval(sid, 1);
        r.check(
            "a blocked op leaves the whole semop unapplied",
            rc == -1 && v1 == 5,
            &format!("r={rc} sem1={v1}"),
        );

        // cross-process blocking wakeup
        sem_setval(sid, 0, 0);
        let pid = unsafe { fork() };
        if pid == 0 {
            let (rc, _) = semop_nowait(sid, &[(0, -1)], false);
            std::process::exit(if rc == 0 { 0 } else { 1 });
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        sem_setval(sid, 0, 1);
        let mut status: c_int = 0;
        unsafe { waitpid(pid, &mut status, 0) };
        r.check(
            "a blocked P() wakes up when another process posts",
            (status & 0x7f) == 0 && (status >> 8) == 0,
            &format!("status={status}"),
        );

        let mut ds = [0u8; 128];
        zero_errno();
        let rc = unsafe { semctl(sid, 0, IPC_STAT, ds.as_mut_ptr() as *mut c_void) };
        let nsems = ulong_at(&ds, 64);
        r.check(
            "semctl(IPC_STAT) reports nsems",
            rc == 0 && nsems == 4,
            &format!("r={rc} nsems={nsems}"),
        );
        let rc = unsafe { semctl(sid, 0, IPC_RMID) };
        r.check("semctl(IPC_RMID)", rc == 0, &format!("r={rc}"));
        zero_errno();
        let rc = sem_getval(sid, 0);
        r.check(
            "a removed set is gone",
            rc == -1 && errno() == EINVAL,
            &format!("r={rc} errno={}", errno()),
        );

        println!("== messages ==");
        zero_errno();
        let q = unsafe { msgget(IPC_PRIVATE, 0o600 | IPC_CREAT) };
        r.check(
            "msgget(IPC_PRIVATE)",
            q > 0,
            &format!("id={q} errno={}", errno()),
        );
        for (mtype, text) in [(2i64, "two"), (1, "one"), (3, "three")] {
            let mut msg = MsgBuf::default();
            msg.mtype = mtype as c_long;
            msg.mtext[..text.len()].copy_from_slice(text.as_bytes());
            zero_errno();
            let rc = unsafe { msgsnd(q, &msg, text.len(), 0) };
            r.check(
                &format!("msgsnd type {mtype}"),
                rc == 0 && errno() == 0,
                &format!("r={rc} errno={}", errno()),
            );
        }
        let mut order: Vec<(i64, Vec<u8>)> = Vec::new();
        for _ in 0..3 {
            let mut msg = MsgBuf::default();
            zero_errno();
            let n = unsafe { msgrcv(q, &mut msg, 256, 0, 0) };
            let text = if n > 0 {
                msg.mtext[..n as usize].to_vec()
            } else {
                Vec::new()
            };
            order.push((msg.mtype as i64, text));
        }
        let types: Vec<i64> = order.iter().map(|o| o.0).collect();
        let texts_ok =
            order
                .iter()
                .map(|o| o.1.as_slice())
                .eq([b"one".as_slice(), b"two", b"three"]);
        r.check(
            "msgrcv(typ=0) returns by type: 1,2,3",
            types == vec![1, 2, 3] && texts_ok,
            &format!("{types:?}"),
        );
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 256, 0, IPC_NOWAIT) };
        r.check(
            "msgrcv on an empty queue with IPC_NOWAIT is ENOMSG",
            n == -1 && errno() == ENOMSG,
            &format!("r={n} errno={}", errno()),
        );

        for (mtype, text) in [(5i64, "five"), (9, "nine"), (1, "one-again")] {
            let mut msg = MsgBuf::default();
            msg.mtype = mtype as c_long;
            msg.mtext[..text.len()].copy_from_slice(text.as_bytes());
            unsafe { msgsnd(q, &msg, text.len(), 0) };
        }
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 256, 9, 0) };
        r.check(
            "msgrcv(typ=9) picks that type only",
            n == 4 && msg.mtype == 9 && &msg.mtext[..4] == b"nine",
            &format!("r={n} type={}", msg.mtype),
        );
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 256, -5, 0) };
        r.check(
            "msgrcv(typ=-5) picks the lowest type <= 5",
            n == 9 && msg.mtype == 1,
            &format!("r={n} type={}", msg.mtype),
        );
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 256, -5, 0) };
        r.check(
            "then the next one <= 5 is type 5",
            n == 4 && msg.mtype == 5,
            &format!("r={n} type={}", msg.mtype),
        );

        let mut msg = MsgBuf::default();
        msg.mtype = 7;
        for byte in msg.mtext[..100].iter_mut() {
            *byte = b'x';
        }
        unsafe { msgsnd(q, &msg, 100, 0) };
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 10, 7, 0) };
        r.check(
            "a message longer than the buffer is E2BIG and stays",
            n == -1 && errno() == E2BIG,
            &format!("r={n} errno={}", errno()),
        );
        let mut msg = MsgBuf::default();
        zero_errno();
        let n = unsafe { msgrcv(q, &mut msg, 10, 7, MSG_NOERROR) };
        r.check(
            "MSG_NOERROR truncates and delivers",
            n == 10 && &msg.mtext[..10] == b"xxxxxxxxxx",
            &format!("r={n}"),
        );

        let mut ds = [0u8; 120];
        zero_errno();
        let rc = unsafe { msgctl(q, IPC_STAT, ds.as_mut_ptr() as *mut c_void) };
        let qnum = ulong_at(&ds, 80);
        r.check(
            "msgctl(IPC_STAT) counts queued messages",
            rc == 0 && qnum == 0,
            &format!("r={rc} qnum={qnum}"),
        );
        let rc = unsafe { msgctl(q, IPC_RMID, std::ptr::null_mut()) };
        r.check("msgctl(IPC_RMID)", rc == 0, &format!("r={rc}"));
        let node = format!("/run/husk/ipc/msg/{q}");
        r.check(
            "queue directory is gone",
            !std::path::Path::new(&node).exists(),
            "",
        );
        let _ = IPC_SET;
        r.finish()
    }

    pub fn run() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("mq") => run_mq(),
            Some("shm") => run_shm(),
            Some("sem-msg") => run_sem_msg(),
            _ => {
                eprintln!("usage: ipc-probe <mq | shm | sem-msg>");
                2
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(linux::run());
}

/// The SysV IPC calls do not exist on macOS, so the binary is guest-only; the
/// host build still succeeds and says why it is not runnable there.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ipc-probe is a guest binary: build with --target aarch64-unknown-linux-gnu");
    std::process::exit(2);
}
