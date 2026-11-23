//! husk-logd - the sink behind `/dev/log`.
//!
//! syslog(3) does not write a file: it connects an `AF_UNIX` `SOCK_DGRAM` socket
//! to `/dev/log` and sends one datagram per message. With no listener every
//! `syslog()` in the guest fails, so daemons (dbus-daemon, pipewire, pppd,
//! anything using the standard helper) log nowhere - measured here as
//! `R|svc|syslog|FAIL|no /dev/log => syslog() unusable`.
//!
//! It is written `no_std` and links statically with no libc at all, for two
//! reasons. The build machine has no glibc sysroot for the guest target (the
//! guest carries libc6 but not libc6-dev, and installing libc6-dev would drag
//! libc6 - the build the runtime has patched - out of place). And the program
//! deliberately talks *host* paths: it takes the guest root as argv[1] and
//! prepends it itself, so it does not need the shim's path translation to
//! exist before the socket does.
//!
//! Usage: `husk-logd <guest-root>` (the runtime launcher does this).

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};

const SYS_WRITE: usize = 64;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_UNLINKAT: usize = 35;
const SYS_MKDIRAT: usize = 34;
const SYS_FCHMODAT: usize = 53;
const SYS_SOCKET: usize = 198;
const SYS_BIND: usize = 200;
const SYS_RECVFROM: usize = 207;
const SYS_EXIT: usize = 93;
const AT_FDCWD: isize = -100;
const O_WRONLY: usize = 0o1;
const O_CREAT: usize = 0o100;
const O_APPEND: usize = 0o2000;
const AF_UNIX: usize = 1;
const SOCK_DGRAM: usize = 2;
const PATH_MAX: usize = 4096;

type Ssize = isize;

// The four symbols LLVM lowers our own loops into. Written with volatile
// accesses on purpose: a plain loop here would be recognised as string.h again
// and turn into a call to itself.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut n = 0usize;
    while core::ptr::read_volatile(s.add(n)) != 0 {
        n += 1;
    }
    n
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    let mut i = 0usize;
    while i < n {
        core::ptr::write_volatile(d.add(i), core::ptr::read_volatile(s.add(i)));
        i += 1;
    }
    d
}

#[no_mangle]
pub unsafe extern "C" fn memmove(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    if (d as usize) < (s as usize) {
        memcpy(d, s, n)
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            core::ptr::write_volatile(d.add(i), core::ptr::read_volatile(s.add(i)));
        }
        d
    }
}

#[no_mangle]
pub unsafe extern "C" fn memset(d: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0usize;
    while i < n {
        core::ptr::write_volatile(d.add(i), c as u8);
        i += 1;
    }
    d
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i < n {
        let (x, y) = (
            core::ptr::read_volatile(a.add(i)),
            core::ptr::read_volatile(b.add(i)),
        );
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

/// panic=abort, so this is never called; core's DWARF reference still wants it.
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { exit(1) }
}

global_asm!(
    r#"
    .text
    .globl _start
_start:
    mov  x0, sp
    bl   nr_main
    mov  x0, #0
    mov  x8, #93
    svc  #0
"#
);

unsafe fn sys(n: usize, a: usize, b: usize, c: usize, d: usize) -> Ssize {
    let ret: Ssize;
    asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
         lateout("x0") ret, options(nostack));
    ret
}

unsafe fn exit(code: usize) -> ! {
    sys(SYS_EXIT, code, 0, 0, 0);
    loop {}
}

/// Copy `s` into `dst` at `at`, NUL-terminated. Returns the new offset.
unsafe fn put(dst: *mut u8, at: &mut usize, s: &[u8]) -> bool {
    if *at + s.len() + 1 >= PATH_MAX {
        return false;
    }
    let mut i = 0;
    while i < s.len() {
        *dst.add(*at + i) = s[i];
        i += 1;
    }
    *at += s.len();
    *dst.add(*at) = 0;
    true
}

unsafe fn cstr_len(mut p: *const u8) -> usize {
    let mut n = 0;
    while *p != 0 {
        n += 1;
        p = p.add(1);
    }
    n
}

/// "2026-11-23T18:30:05Z" in UTC - no tzdata, no libc, no /etc/localtime.
fn stamp(mut secs: u64, out: &mut [u8; 21]) -> usize {
    let mut days = secs / 86_400;
    secs %= 86_400;
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    days += 719_468;
    let era = days / 146_097;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    let mut w = 0usize;
    let mut num = |mut v: u64, width: usize, out: &mut [u8; 21], w: &mut usize| {
        let mut i = width;
        while i > 0 {
            i -= 1;
            out[*w + i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        *w += width;
    };
    num(y, 4, out, &mut w);
    out[w] = b'-';
    w += 1;
    num(m, 2, out, &mut w);
    out[w] = b'-';
    w += 1;
    num(d, 2, out, &mut w);
    out[w] = b'T';
    w += 1;
    num(h, 2, out, &mut w);
    out[w] = b':';
    w += 1;
    num(mi, 2, out, &mut w);
    out[w] = b':';
    w += 1;
    num(s, 2, out, &mut w);
    out[w] = b'Z';
    w + 1
}

unsafe fn now() -> u64 {
    let mut ts = [0u64; 2];
    // clock_gettime(CLOCK_REALTIME)
    sys(113, 0, ts.as_mut_ptr() as usize, 0, 0);
    ts[0]
}

/// Byte offset of the last '/' in `p[..n]`, for mkdir -p over the log directory.
unsafe fn mkdir_p(root: *const u8, rel: &[u8], full: *mut u8) {
    let mut at = 0usize;
    if !put(full, &mut at, &[]) {
        return;
    }
    // copy the root
    let rl = cstr_len(root);
    core::ptr::copy_nonoverlapping(root, full, rl);
    at = rl;
    let mut i = 0;
    while i < rel.len() {
        *full.add(at) = rel[i];
        at += 1;
        i += 1;
        *full.add(at) = 0;
        if i < rel.len() && rel[i] == b'/' {
            sys(SYS_MKDIRAT, AT_FDCWD as usize, full as usize, 0o755, 0);
        }
    }
}

#[no_mangle]
pub unsafe fn nr_main(sp: *const usize) -> isize {
    let argc = *sp;
    let argv = sp.add(1);
    let mut root: *const u8 = core::ptr::null();
    if argc > 1 {
        root = *argv.add(1) as *const u8;
    } else {
        // NR_ROOTFS from the environment, when the launcher did not pass a path.
        let envp = argv.add(argc + 1);
        let mut i = 0usize;
        while *envp.add(i) != 0 {
            let e = *envp.add(i) as *const u8;
            if cstr_len(e) > 10 {
                let mut k = 0usize;
                let mut ok = true;
                while k < 10 {
                    if *e.add(k) != b"NR_ROOTFS="[k] {
                        ok = false;
                        break;
                    }
                    k += 1;
                }
                if ok {
                    root = e.add(10);
                    break;
                }
            }
            i += 1;
        }
    }
    if root.is_null() {
        let m = b"husk-logd: usage: husk-logd <guest-root>\n";
        sys(SYS_WRITE, 2, m.as_ptr() as usize, m.len(), 0);
        return 1;
    }

    // <root>/dev/log
    let mut sock_path = [0u8; 120];
    let mut at = 0usize;
    let rl = cstr_len(root);
    if rl + 9 >= 108 {
        return 1;
    }
    core::ptr::copy_nonoverlapping(root, sock_path.as_mut_ptr(), rl);
    at = rl;
    put(sock_path.as_mut_ptr(), &mut at, b"/dev/log");
    let sock_path_len = at;

    // <root>/var/log/syslog (+ its directory once)
    let mut log_path = [0u8; PATH_MAX];
    let mut log_rel = [0u8; 20];
    log_rel[..15].copy_from_slice(b"/var/log/syslog");
    mkdir_p(root, b"/var/log", log_path.as_mut_ptr());
    sys(SYS_MKDIRAT, AT_FDCWD as usize, log_path.as_ptr() as usize, 0o755, 0);
    let mut lat = rl;
    core::ptr::copy_nonoverlapping(root, log_path.as_mut_ptr(), rl);
    put(log_path.as_mut_ptr(), &mut lat, b"/var/log/syslog");
    let log_len = lat;

    let fd = sys(SYS_SOCKET, AF_UNIX, SOCK_DGRAM, 0, 0);
    if fd < 0 {
        return 1;
    }
    // A stale socket file from a previous run is ours to clear.
    sys(SYS_UNLINKAT, AT_FDCWD as usize, sock_path.as_ptr() as usize, 0, 0);
    let mut sa = [0u8; 110];
    sa[0] = AF_UNIX as u8;
    sa[1] = 0;
    core::ptr::copy_nonoverlapping(sock_path.as_ptr(), sa.as_mut_ptr().add(2), sock_path_len);
    let bound = sys(
        SYS_BIND,
        fd as usize,
        sa.as_ptr() as usize,
        2 + sock_path_len,
        0,
    );
    if bound < 0 {
        // Another instance owns it; that is the normal "already running" case.
        return 0;
    }
    // Every uid in the guest must be able to send, like a real /dev/log.
    sys(SYS_FCHMODAT, AT_FDCWD as usize, sock_path.as_ptr() as usize, 0o666, 0);

    let mut buf = [0u8; 8192];
    loop {
        let n = sys(
            SYS_RECVFROM,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
            0,
        );
        if n <= 0 {
            continue;
        }
        let mut n = n as usize;
        while n > 0 && buf[n - 1] == 0 {
            n -= 1;
        }
        if n == 0 {
            continue;
        }
        let ofd = sys(
            SYS_OPENAT,
            AT_FDCWD as usize,
            log_path.as_ptr() as usize,
            O_WRONLY | O_CREAT | O_APPEND,
            0o644,
        );
        if ofd < 0 {
            continue;
        }
        let mut ts = [0u8; 21];
        let tn = stamp(now(), &mut ts);
        sys(SYS_WRITE, ofd as usize, ts.as_ptr() as usize, tn, 0);
        sys(SYS_WRITE, ofd as usize, b" ".as_ptr() as usize, 1, 0);
        sys(SYS_WRITE, ofd as usize, buf.as_ptr() as usize, n, 0);
        sys(SYS_WRITE, ofd as usize, b"\n".as_ptr() as usize, 1, 0);
        sys(SYS_CLOSE, ofd as usize, 0, 0, 0);
    }
}
