//! husk-init - the container-style service supervisor for the Husk guest.
//!
//! It is a freestanding static binary: no libc, no dynamic loader, only raw
//! syscalls. That is on purpose. A supervisor has to keep running when the rest
//! of the container is being torn down, and it must not depend on the path shim
//! (it addresses the guest tree through the rootfs prefix itself).
//!
//! Config, `$NR_ROOTFS/etc/husk/services.conf`:
//!
//! ```text
//! # name        policy        program and arguments
//! logger        always        /usr/bin/logger -t husk
//! oneshot       once          /usr/bin/true
//! ```
//!
//! policy: `always` (restart whatever the exit status), `on-failure` (restart
//! unless it exited 0), `once` (never restart).
//!
//! Behaviour: children are spawned with their stdout/stderr appended to
//! `$NR_ROOTFS/var/log/husk/<name>.log`, the supervisor is a child subreaper so
//! orphans come back to it and get reaped, a crash is restarted after a backoff
//! that doubles up to 30s, and SIGTERM/SIGINT stop everything (SIGTERM, then
//! SIGKILL after 5s).

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { sys_exit_group(101) }
}

// -------------------------------------------------------- freestanding mem*
// A static freestanding binary has no libc, so the compiler's implicit memcpy /
// memset calls have to be provided here. The volatile accesses keep LLVM from
// turning these loops back into calls to themselves.

#[no_mangle]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0usize;
    while i < n {
        core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        i += 1;
    }
    dst
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dst as usize) < (src as usize) {
        memcpy(dst, src, n)
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
        dst
    }
}

#[no_mangle]
pub unsafe extern "C" fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8 {
    let v = c as u8;
    let mut i = 0usize;
    while i < n {
        core::ptr::write_volatile(dst.add(i), v);
        i += 1;
    }
    dst
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i < n {
        let x = core::ptr::read_volatile(a.add(i));
        let y = core::ptr::read_volatile(b.add(i));
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    memcmp(a, b, n)
}

/// core still references the panic-unwind personality even with panic=abort;
/// an empty definition satisfies the linker and is never called.
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut i = 0usize;
    while core::ptr::read_volatile(s.add(i)) != 0 {
        i += 1;
    }
    i
}

// ------------------------------------------------------------------ syscalls

const NR_READ: usize = 63;
const NR_WRITE: usize = 64;
const NR_OPENAT: usize = 56;
const NR_CLOSE: usize = 57;
const NR_EXIT_GROUP: usize = 94;
const NR_PPOLL: usize = 73;
const NR_SIGNALFD4: usize = 74;
const NR_RT_SIGPROCMASK: usize = 135;
const NR_CLONE: usize = 220;
const NR_EXECVE: usize = 221;
const NR_WAIT4: usize = 260;
const NR_KILL: usize = 129;
const NR_CHDIR: usize = 49;
const NR_PRCTL: usize = 167;

const AT_FDCWD: isize = -100;
const O_RDONLY: usize = 0;
const O_WRONLY: usize = 1;
const O_CREAT: usize = 0o100;
const O_APPEND: usize = 0o2000;
const O_CLOEXEC: usize = 0o2000000;

const SIGCHLD: usize = 17;
const SIGTERM: usize = 15;
const SIGINT: usize = 2;
const SIGQUIT: usize = 3;
const SIGKILL: usize = 9;
const SIG_BLOCK: usize = 0;
const SIG_SETMASK: usize = 2;
const WNOHANG: usize = 1;
const PR_SET_CHILD_SUBREAPER: usize = 36;

unsafe fn sc4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let ret: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        lateout("x0") ret, options(nostack));
    ret
}

unsafe fn sc5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> isize {
    let ret: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d, in("x4") e,
        lateout("x0") ret, options(nostack));
    ret
}

unsafe fn sys_exit_group(code: usize) -> ! {
    let _: isize;
    core::arch::asm!("svc 0", in("x8") NR_EXIT_GROUP, in("x0") code, lateout("x0") _, options(nostack));
    loop {}
}

unsafe fn sys_write(fd: isize, buf: &[u8]) -> isize {
    sc4(NR_WRITE, fd as usize, buf.as_ptr() as usize, buf.len(), 0)
}

unsafe fn out(parts: &[&[u8]]) {
    for p in parts {
        sys_write(2, p);
    }
}

unsafe fn outn(prefix: &[u8], mut v: i64) {
    sys_write(2, prefix);
    let mut buf = [0u8; 24];
    let mut i = buf.len();
    let neg = v < 0;
    if neg {
        v = -v;
    }
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        buf[i] = b'-';
    }
    sys_write(2, &buf[i..]);
}

// ------------------------------------------------------------------- limits

const MAX_SERVICES: usize = 8;
const CFG_MAX: usize = 16 * 1024;
const ENV_MAX: usize = 16 * 1024;
const PATH_MAX: usize = 1024;
const NAME_MAX: usize = 40;

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Always,
    OnFailure,
    Once,
}

struct Service {
    name: [u8; NAME_MAX],
    name_len: usize,
    policy: Policy,
    /// offsets into CFG: program (host path) and its arguments
    prog: (usize, usize),
    args: [(usize, usize); 16],
    nargs: usize,
    pid: isize,
    done: bool,
    backoff: u64,
    started_ns: u64,
}

impl Service {
    const EMPTY: Service = Service {
        name: [0; NAME_MAX],
        name_len: 0,
        policy: Policy::Once,
        prog: (0, 0),
        args: [(0, 0); 16],
        nargs: 0,
        pid: 0,
        done: false,
        backoff: 1,
        started_ns: 0,
    };
}

static mut CFG: [u8; CFG_MAX] = [0; CFG_MAX];
static mut CFG_LEN: usize = 0;
static mut ENV: [u8; ENV_MAX] = [0; ENV_MAX];
static mut ENV_LEN: usize = 0;
static mut ARGV: [usize; 64] = [0; 64];
static mut ARGV_STR: [u8; 4096] = [0; 4096];
static mut SERVICES: [Service; MAX_SERVICES] = [Service::EMPTY; MAX_SERVICES];
static mut NSVC: usize = 0;
static mut ROOT: [u8; PATH_MAX] = [0; PATH_MAX];
static mut ROOT_LEN: usize = 0;
static mut LD: [u8; PATH_MAX] = [0; PATH_MAX];
static mut LD_LEN: usize = 0;
static mut LIBPATH: [u8; PATH_MAX * 2] = [0; PATH_MAX * 2];
static mut LIBPATH_LEN: usize = 0;
static STOP: AtomicBool = AtomicBool::new(false);
/// The supervisor inherits the app's environment, whose PATH is Android's. A
/// guest program started with that PATH cannot find `sleep`, let alone
/// anything in /usr/bin, so the container gets its own PATH and HOME.
static GUEST_PATH: &[u8] = b"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\0";
static GUEST_HOME: &[u8] = b"HOME=/root\0";

// -------------------------------------------------------------- small utils

fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\r'
}

/// Copy `s` into a NUL-terminated static buffer at `dst`, returning the length
/// without the NUL.
unsafe fn put(dst: &mut [u8], s: &[u8]) -> usize {
    let n = if s.len() < dst.len() - 1 {
        s.len()
    } else {
        dst.len() - 1
    };
    dst[..n].copy_from_slice(&s[..n]);
    dst[n] = 0;
    n
}

unsafe fn read_file(path: &[u8], dst: &mut [u8]) -> usize {
    let fd = sc4(
        NR_OPENAT,
        AT_FDCWD as usize,
        path.as_ptr() as usize,
        O_RDONLY | O_CLOEXEC,
        0,
    );
    if fd < 0 {
        return 0;
    }
    let mut total = 0usize;
    loop {
        let r = sc4(
            NR_READ,
            fd as usize,
            dst[total..].as_mut_ptr() as usize,
            dst.len() - total,
            0,
        );
        if r <= 0 {
            break;
        }
        total += r as usize;
        if total >= dst.len() {
            break;
        }
    }
    sc4(NR_CLOSE, fd as usize, 0, 0, 0);
    total
}

unsafe fn env_value(key: &[u8], outb: &mut [u8]) -> usize {
    let env = &*core::ptr::addr_of!(ENV);
    let n = ENV_LEN;
    let mut i = 0usize;
    while i < n {
        let start = i;
        while i < n && env[i] != 0 {
            i += 1;
        }
        let ent = &env[start..i];
        i += 1;
        if ent.len() > key.len() && &ent[..key.len()] == key && ent[key.len()] == b'=' {
            return put(outb, &ent[key.len() + 1..]);
        }
    }
    0
}

// ------------------------------------------------------------------- config

unsafe fn parse_policy(s: &[u8]) -> Policy {
    if s == b"always" {
        Policy::Always
    } else if s == b"on-failure" {
        Policy::OnFailure
    } else {
        Policy::Once
    }
}

/// Parse the config in place. Lines are `name policy program [args...]`; `#`
/// starts a comment, blank lines are skipped. No quoting: arguments are split on
/// whitespace, which is documented in docs/design/supervisor.md.
unsafe fn parse_config() -> usize {
    let cfg = &*core::ptr::addr_of!(CFG);
    let n = CFG_LEN;
    let mut i = 0usize;
    let mut count = 0usize;
    while i < n && count < MAX_SERVICES {
        let line_start = i;
        while i < n && cfg[i] != b'\n' {
            i += 1;
        }
        let line = &cfg[line_start..i];
        i += 1;
        // trim
        let mut a = 0usize;
        let mut b = line.len();
        while a < b && is_space(line[a]) {
            a += 1;
        }
        while b > a && is_space(line[b - 1]) {
            b -= 1;
        }
        if a == b || line[a] == b'#' {
            continue;
        }
        let line = &line[a..b];
        // fields
        let mut fields: [usize; 3] = [0; 3]; // start offsets
        let mut f = 0usize;
        let mut k = 0usize;
        let mut svc = Service::EMPTY;
        while k < line.len() && f < 2 {
            while k < line.len() && is_space(line[k]) {
                k += 1;
            }
            let s = k;
            while k < line.len() && !is_space(line[k]) {
                k += 1;
            }
            if k > s {
                fields[f] = line_start + a + s;
                // record field lengths via a parallel array
                f += 1;
                if f == 1 {
                    svc.name_len = if k - s < NAME_MAX {
                        k - s
                    } else {
                        NAME_MAX - 1
                    };
                    let mut z = 0usize;
                    while z < svc.name_len {
                        svc.name[z] = line[s + z];
                        z += 1;
                    }
                } else if f == 2 {
                    svc.policy = parse_policy(&line[s..k]);
                }
            }
        }
        if f < 2 {
            continue;
        }
        // program
        while k < line.len() && is_space(line[k]) {
            k += 1;
        }
        let ps = k;
        while k < line.len() && !is_space(line[k]) {
            k += 1;
        }
        if k == ps {
            continue;
        }
        svc.prog = (line_start + a + ps, k - ps);
        // arguments
        svc.nargs = 0;
        loop {
            while k < line.len() && is_space(line[k]) {
                k += 1;
            }
            if k >= line.len() {
                break;
            }
            let s = k;
            while k < line.len() && !is_space(line[k]) {
                k += 1;
            }
            if svc.nargs < svc.args.len() {
                svc.args[svc.nargs] = (line_start + a + s, k - s);
                svc.nargs += 1;
            }
        }
        SERVICES[count] = svc;
        count += 1;
    }
    count
}

// ------------------------------------------------------------------- spawn

/// Build argv for one service: loader, --library-path, paths, program, args.
/// Strings live in ARGV_STR, pointers in ARGV; both are static because the child
/// execs immediately and the parent only rebuilds them for the next spawn.
unsafe fn spawn(idx: usize) -> isize {
    let svc = &SERVICES[idx];
    let cfg = &*core::ptr::addr_of!(CFG);
    let root = &*core::ptr::addr_of!(ROOT);
    let ld = &*core::ptr::addr_of!(LD);
    let libp = &*core::ptr::addr_of!(LIBPATH);
    let rs = ROOT_LEN;
    let ls = LD_LEN;
    let mut off = 0usize;
    let mut argc = 0usize;

    // argv[0]: loader
    ARGV_STR[off..off + ls].copy_from_slice(&ld[..ls]);
    ARGV_STR[off + ls] = 0;
    ARGV[argc] = ARGV_STR.as_ptr().add(off) as usize;
    argc += 1;
    off += ls + 1;

    // --library-path
    let opt = b"--library-path\0";
    ARGV_STR[off..off + opt.len()].copy_from_slice(opt);
    ARGV[argc] = ARGV_STR.as_ptr().add(off) as usize;
    argc += 1;
    off += opt.len();

    let lp = LIBPATH_LEN;
    ARGV_STR[off..off + lp].copy_from_slice(&libp[..lp]);
    ARGV_STR[off + lp] = 0;
    ARGV[argc] = ARGV_STR.as_ptr().add(off) as usize;
    argc += 1;
    off += lp + 1;

    // program: rootfs + guest path, one NUL terminated string
    let prog_at = off;
    ARGV_STR[off..off + rs].copy_from_slice(&root[..rs]);
    off += rs;
    let (pstart, pl) = svc.prog;
    ARGV_STR[off..off + pl].copy_from_slice(&cfg[pstart..pstart + pl]);
    ARGV_STR[off + pl] = 0;
    ARGV[argc] = ARGV_STR.as_ptr().add(prog_at) as usize;
    argc += 1;
    off += pl + 1;

    let mut a = 0usize;
    while a < svc.nargs && argc < ARGV.len() - 1 {
        let (s, l) = svc.args[a];
        ARGV_STR[off..off + l].copy_from_slice(&cfg[s..s + l]);
        ARGV_STR[off + l] = 0;
        ARGV[argc] = ARGV_STR.as_ptr().add(off) as usize;
        argc += 1;
        off += l + 1;
        a += 1;
    }
    ARGV[argc] = 0;

    // log path: <root>/var/log/husk/<name>.log
    let mut logp = [0u8; PATH_MAX];
    let head = b"/var/log/husk/";
    logp[..rs].copy_from_slice(&root[..rs]);
    let mut k = rs;
    logp[k..k + head.len()].copy_from_slice(head);
    k += head.len();
    let mut j = 0usize;
    while j < svc.name_len {
        logp[k] = svc.name[j];
        k += 1;
        j += 1;
    }
    let tail = b".log\0";
    logp[k..k + tail.len()].copy_from_slice(tail);

    let pid = sc5(NR_CLONE, SIGCHLD, 0, 0, 0, 0);
    if pid == 0 {
        // ---- child
        let fd = sc4(
            NR_OPENAT,
            AT_FDCWD as usize,
            logp.as_ptr() as usize,
            O_WRONLY | O_CREAT | O_APPEND,
            0o644,
        );
        if fd >= 0 {
            sc4(24 /* dup3 */, fd as usize, 1, 0, 0);
            sc4(24, fd as usize, 2, 0, 0);
            sc4(NR_CLOSE, fd as usize, 0, 0, 0);
        }
        sc4(NR_CHDIR, root.as_ptr() as usize, 0, 0, 0);
        // The supervisor blocks SIGTERM/SIGINT/SIGQUIT/SIGCHLD to read them from
        // its signalfd, and a blocked mask is inherited across exec - so without
        // this the services would ignore the SIGTERM we send them on stop.
        let empty: u64 = 0;
        sc4(
            NR_RT_SIGPROCMASK,
            SIG_SETMASK,
            core::ptr::addr_of!(empty) as usize,
            0,
            8,
        );
        let env = &*core::ptr::addr_of!(ENV);
        // ENV is NUL separated; rebuild pointer list in place is not safe, so
        // hand the raw environment through a static pointer array built here.
        let mut envp = [0usize; 129];
        let mut ec: isize = 0;
        {
            let mut i = 0usize;
            let n = ENV_LEN;
            let mut c = 0usize;
            while i < n && c < envp.len() - 3 {
                let start = i;
                while i < n && env[i] != 0 {
                    i += 1;
                }
                let ent = &env[start..i];
                i += 1;
                let shadowed = (ent.len() >= 5 && &ent[..5] == b"PATH=")
                    || (ent.len() >= 5 && &ent[..5] == b"HOME=");
                if shadowed {
                    continue;
                }
                envp[c] = env.as_ptr().add(start) as usize;
                c += 1;
            }
            envp[c] = GUEST_PATH.as_ptr() as usize;
            c += 1;
            envp[c] = GUEST_HOME.as_ptr() as usize;
            c += 1;
            envp[c] = 0;
            // NR_INIT_DEBUG=1 prints the exact exec argv, which is the only
            // thing that can be wrong when the loader refuses the program.
            {
                let mut dbg = [0u8; 8];
                if env_value(b"NR_INIT_DEBUG", &mut dbg) > 0 {
                    out(&[b"husk-init: exec argv:"]);
                    let mut q = 0usize;
                    while q < ARGV.len() && ARGV[q] != 0 {
                        let p = ARGV[q] as *const u8;
                        let l = strlen(p);
                        sys_write(2, core::slice::from_raw_parts(p, l));
                        out(&[b" |"]);
                        q += 1;
                    }
                    out(&[b"\n"]);
                }
            }
            ec = sc4(
                NR_EXECVE,
                ARGV[0],
                ARGV.as_ptr() as usize,
                envp.as_ptr() as usize,
                0,
            );
        }
        out(&[
            b"husk-init: exec failed for ",
            &svc.name[..svc.name_len],
            b" errno=",
        ]);
        outn(b"", ec as i64);
        out(&[b"\n"]);
        sys_exit_group(127);
    }
    pid
}

// ---------------------------------------------------------------- main loop

unsafe fn reap(now_ms: u64) {
    let mut status: i32 = 0;
    loop {
        let pid = sc4(
            NR_WAIT4,
            (-1isize) as usize,
            core::ptr::addr_of_mut!(status) as usize,
            WNOHANG,
            0,
        );
        if pid <= 0 {
            return;
        }
        let mut i = 0usize;
        while i < NSVC {
            if SERVICES[i].pid == pid {
                let svc = &mut SERVICES[i];
                svc.pid = 0;
                let code = (status >> 8) & 0xff;
                let sig = status & 0x7f;
                let clean = sig == 0 && code == 0;
                let uptime_ms = now_ms - svc.started_ns;
                outn(b"husk-init: ", pid as i64);
                out(&[b" exited (", if sig != 0 { b"signal " } else { b"code " }]);
                outn(b"", if sig != 0 { sig as i64 } else { code as i64 });
                out(&[b") svc=", &svc.name[..svc.name_len], b" uptime_ms="]);
                outn(b"", uptime_ms as i64);
                let restart = match svc.policy {
                    Policy::Once => false,
                    Policy::Always => true,
                    Policy::OnFailure => !clean,
                };
                if restart && STOP.load(Ordering::Relaxed) {
                    out(&[b" -> stopping\n"]);
                } else if restart {
                    // reset backoff if the service stayed up long enough
                    if uptime_ms > 10_000 {
                        svc.backoff = 1;
                    }
                    out(&[b" -> restart in "]);
                    outn(b"", svc.backoff as i64);
                    out(&[b"s\n"]);
                    svc.started_ns = now_ms;
                    svc.pid = -((now_ms + svc.backoff * 1000) as isize); // negative pid = restart deadline
                    if svc.backoff < 30 {
                        svc.backoff *= 2;
                    }
                } else {
                    out(&[b" -> done\n"]);
                    svc.done = true;
                }
                break;
            }
            i += 1;
        }
    }
}

unsafe fn start_due(now_ms: u64) {
    let mut i = 0usize;
    while i < NSVC {
        if SERVICES[i].pid == 0 && !SERVICES[i].done {
            SERVICES[i].started_ns = now_ms;
            SERVICES[i].pid = spawn(i);
            out(&[
                b"husk-init: started ",
                &SERVICES[i].name[..SERVICES[i].name_len],
                b" pid=",
            ]);
            outn(b"", SERVICES[i].pid as i64);
            out(&[b"\n"]);
        } else if SERVICES[i].pid < 0 {
            let deadline = -SERVICES[i].pid as u64;
            if now_ms >= deadline {
                SERVICES[i].started_ns = now_ms;
                SERVICES[i].pid = spawn(i);
                out(&[
                    b"husk-init: restarted ",
                    &SERVICES[i].name[..SERVICES[i].name_len],
                    b" pid=",
                ]);
                outn(b"", SERVICES[i].pid as i64);
                out(&[b"\n"]);
            }
        }
        i += 1;
    }
}

unsafe fn stop_all() {
    out(&[b"husk-init: stopping services\n"]);
    let mut i = 0usize;
    while i < NSVC {
        if SERVICES[i].pid > 0 {
            sc4(NR_KILL, SERVICES[i].pid as usize, SIGTERM, 0, 0);
        }
        i += 1;
    }
    // wait up to 5s for a clean exit
    let mut rounds = 0usize;
    while rounds < 50 {
        let mut live = 0usize;
        let mut q = 0usize;
        while q < NSVC {
            if SERVICES[q].pid > 0 {
                live += 1;
            }
            q += 1;
        }
        if live == 0 {
            break;
        }
        let mut status: i32 = 0;
        let pid = sc4(
            NR_WAIT4,
            (-1isize) as usize,
            core::ptr::addr_of_mut!(status) as usize,
            WNOHANG,
            0,
        );
        if pid > 0 {
            let mut j = 0usize;
            while j < NSVC {
                if SERVICES[j].pid == pid {
                    SERVICES[j].pid = 0;
                }
                j += 1;
            }
            continue;
        }
        usleep(100_000);
        rounds += 1;
    }
    let mut i = 0usize;
    while i < NSVC {
        if SERVICES[i].pid > 0 {
            sc4(NR_KILL, SERVICES[i].pid as usize, SIGKILL, 0, 0);
        }
        i += 1;
    }
}

unsafe fn usleep(us: usize) {
    let ts = [us / 1_000_000, (us % 1_000_000) * 1000];
    sc4(101 /* nanosleep */, ts.as_ptr() as usize, 0, 0, 0);
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    // environment (raw: we are before any libc)
    ENV_LEN = read_file(b"/proc/self/environ\0", &mut *core::ptr::addr_of_mut!(ENV));
    ROOT_LEN = env_value(b"NR_ROOTFS", &mut *core::ptr::addr_of_mut!(ROOT));
    LD_LEN = env_value(b"NR_LD", &mut *core::ptr::addr_of_mut!(LD));
    if ROOT_LEN == 0 || LD_LEN == 0 {
        out(&[b"husk-init: NR_ROOTFS and NR_LD must be set\n"]);
        sys_exit_group(2);
    }
    // library path: two guest lib dirs, both under the rootfs
    {
        let root = &*core::ptr::addr_of!(ROOT);
        let rs = ROOT_LEN;
        let a = b"/usr/lib/aarch64-linux-gnu:";
        let b = b"/lib/aarch64-linux-gnu";
        let libp = &mut *core::ptr::addr_of_mut!(LIBPATH);
        let mut k = 0usize;
        libp[k..k + rs].copy_from_slice(&root[..rs]);
        k += rs;
        libp[k..k + a.len()].copy_from_slice(a);
        k += a.len();
        libp[k..k + rs].copy_from_slice(&root[..rs]);
        k += rs;
        libp[k..k + b.len()].copy_from_slice(b);
        k += b.len();
        libp[k] = 0;
        LIBPATH_LEN = k;
    }
    // config
    {
        let root = &*core::ptr::addr_of!(ROOT);
        let mut cfgp = [0u8; PATH_MAX];
        let rs = ROOT_LEN;
        cfgp[..rs].copy_from_slice(&root[..rs]);
        let tail = b"/etc/husk/services.conf\0";
        cfgp[rs..rs + tail.len()].copy_from_slice(tail);
        CFG_LEN = read_file(&cfgp, &mut *core::ptr::addr_of_mut!(CFG));
    }
    if CFG_LEN == 0 {
        out(&[b"husk-init: no /etc/husk/services.conf inside the rootfs\n"]);
        sys_exit_group(3);
    }
    NSVC = parse_config();
    if NSVC == 0 {
        out(&[b"husk-init: no services in the config\n"]);
        sys_exit_group(4);
    }
    outn(b"husk-init: services=", NSVC as i64);
    out(&[b" rootfs=", &ROOT[..ROOT_LEN], b"\n"]);

    // orphans must come back to us so nothing is ever left as a zombie
    sc5(NR_PRCTL, PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);

    // SIGCHLD / SIGTERM / SIGINT / SIGQUIT through a signalfd: no handlers, no
    // async-signal-safety questions, and the loop wakes up on any of them.
    let mut set: u64 = 0;
    set |= 1u64 << (SIGCHLD - 1);
    set |= 1u64 << (SIGTERM - 1);
    set |= 1u64 << (SIGINT - 1);
    set |= 1u64 << (SIGQUIT - 1);
    sc4(
        NR_RT_SIGPROCMASK,
        SIG_BLOCK,
        core::ptr::addr_of!(set) as usize,
        0,
        8,
    );
    let sfd = sc4(
        NR_SIGNALFD4,
        (-1isize) as usize,
        core::ptr::addr_of!(set) as usize,
        8,
        0,
    );
    if sfd < 0 {
        out(&[b"husk-init: signalfd4 failed\n"]);
        sys_exit_group(5);
    }

    let mut now_ms: u64 = 0;
    start_due(now_ms);
    loop {
        // 200 ms tick keeps restarts and reaping prompt without burning CPU
        let mut pfd = PollFd {
            fd: sfd as i32,
            events: 1, // POLLIN
            revents: 0,
        };
        let mut ts = [0i64, 200_000_000i64];
        let r = sc5(
            NR_PPOLL,
            core::ptr::addr_of_mut!(pfd) as usize,
            1,
            ts.as_mut_ptr() as usize,
            0,
            8,
        );
        if r > 0 {
            let mut si = [0u8; 128];
            let n = sc4(NR_READ, sfd as usize, si.as_mut_ptr() as usize, si.len(), 0);
            if n >= 4 {
                let signo = u32::from_ne_bytes([si[0], si[1], si[2], si[3]]) as usize;
                if signo == SIGTERM || signo == SIGINT || signo == SIGQUIT {
                    STOP.store(true, Ordering::Relaxed);
                }
            }
        }
        now_ms += 200;
        reap(now_ms);
        if STOP.load(Ordering::Relaxed) {
            stop_all();
            out(&[b"husk-init: done\n"]);
            sys_exit_group(0);
        }
        start_due(now_ms);
    }
}
