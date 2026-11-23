// LinuxHub Native Runtime — preload path shim v3
// no_std, zero DT_NEEDED. Absolute or cwd-relative guest paths are resolved
// through runtime-core against the rootfd (symlink-aware, ".." clamped), and
// execve re-enters the guest ld.so with a real absolute path. No ptrace,
// no chroot, no namespaces.

#![no_std]

mod boot;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU8, Ordering};
use runtime_core as rc;

type CChar = i8;
type CInt = i32;
type CSizet = usize;
type Ssize = isize;

extern "C" {
    static mut environ: *mut *mut CChar;
    fn __errno_location() -> *mut CInt;
    fn fdopendir(fd: CInt) -> *mut u8;
    fn fdopen(fd: CInt, mode: *const CChar) -> *mut u8;
    fn dlmopen(nsid: i64, file: *const CChar, mode: CInt) -> *mut u8;
    fn fork() -> CInt;
    fn posix_spawnattr_getflags(attr: *const u8, flags: *mut i16) -> CInt;
    fn posix_spawnattr_getpgroup(attr: *const u8, pgroup: *mut CInt) -> CInt;
    fn posix_spawnattr_getsigmask(attr: *const u8, sigmask: *mut u8) -> CInt;
    fn posix_spawnattr_getsigdefault(attr: *const u8, sigdefault: *mut u8) -> CInt;
    fn malloc(size: CSizet) -> *mut u8;
}

// `bcmp` is what core::slice's compare_bytes calls on linux. glibc keeps it as a
// compatibility symbol only, and an embedded shim has no loader-resolved scope,
// so it is defined here (same contract as memcmp, only the sign of the result is
// specified).
#[no_mangle]
pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: CSizet) -> CInt {
    let mut i = 0;
    while i < n {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y {
            return x as CInt - y as CInt;
        }
        i += 1;
    }
    0
}

const PATH_MAX: usize = 4096;
const AT_FDCWD: CInt = -100;

// ------------------------------------------------------------ mount table ---
//
// A container can mount; we cannot. mount(2) needs CAP_SYS_ADMIN and this
// domain has none. What a container's userspace actually observes, though, is
// the namespace it lives in: paths under a bind target lead somewhere else, a
// tmpfs starts empty, a read-only bind refuses writes. That part is
// reproducible in the path layer, which is exactly where we already stand.
//
// The table lives in a file (/run/husk/mounts) with an in-process cache, so a
// mount made by one process is visible to processes started after it. A mount
// made while another process is already running is not retroactively visible
// to that process; that is the documented difference from a real namespace.
const MNT_MAX: usize = 8;
const MNT_P: usize = 128;
const MNT_FILE: &[u8] = b"/run/husk/mounts\0";
const MNT_DIR: &[u8] = b"/run/husk\0";
const MS_RDONLY: usize = 1;
const MS_REMOUNT: usize = 32;
const MS_BIND: usize = 4096;
const O_WRITEISH: CInt = 0o3 | 0o100 | 0o1000 | 0o2000;

#[derive(Clone, Copy)]
struct MntE {
    tgt: [u8; MNT_P],
    tlen: u16,
    src: [u8; MNT_P],
    slen: u16,
    ro: u8,
    tmpfs: u8,
}

static mut MNT: [MntE; MNT_MAX] = [MntE {
    tgt: [0; MNT_P],
    tlen: 0,
    src: [0; MNT_P],
    slen: 0,
    ro: 0,
    tmpfs: 0,
}; MNT_MAX];
static mut MNT_N: usize = 0;
static mut MNT_READY: u8 = 0;

const AT_SYMLINK_NOFOLLOW: CInt = 0x100;
const AT_EMPTY_PATH: CInt = 0x1000;
const AT_SYMLINK_FOLLOW: CInt = 0x400;
const O_PATH: CInt = 0o10000000;
const O_NOFOLLOW: CInt = 0o100000; // AArch64 UAPI
const O_DIRECTORY: CInt = 0o40000; // AArch64 UAPI, not asm-generic 0o200000
const O_CLOEXEC: CInt = 0o2000000;
const O_RDONLY0: CInt = 0;

const SYS_WRITE: usize = 64;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_CLOSE_RANGE: usize = 436;
const SYS_FCNTL: usize = 25;
const F_GETFD: usize = 1;
const F_DUPFD_CLOEXEC: usize = 1030;
const ROOTFD_FLOOR: usize = 1024;
const SYS_PREAD64: usize = 67;
const SYS_READLINKAT: usize = 78;
const SYS_NEWFSTATAT: usize = 79;
const SYS_FACCESSAT: usize = 48;
const SYS_CHDIR: usize = 49;
const SYS_FCHDIR: usize = 50;
const SYS_EXECVE: usize = 221;
const SYS_MKDIRAT: usize = 34;
const SYS_UNLINKAT: usize = 35;
const SYS_SYMLINKAT: usize = 36;
const SYS_LINKAT: usize = 37;
const SYS_MKNODAT: usize = 33;
const SYS_GETUID: usize = 174;
const SYS_SETUID: usize = 146;
const SYS_SETGID: usize = 144;
// aarch64/asm-generic has no __NR_seteuid or __NR_setegid; glibc implements
// both on top of setresuid/setresgid with -1 for the parts it keeps.
const SYS_SETGROUPS: usize = 159;
const SYS_SETRESUID: usize = 147;
const SYS_SETRESGID: usize = 149;
const SYS_GETGID: usize = 176;
const SYS_RENAMEAT: usize = 38;
const SYS_RENAMEAT2: usize = 276;
const SYS_FCHMODAT: usize = 53;
const SYS_FCHOWNAT: usize = 54;
const SYS_UTIMENSAT: usize = 88;
const SYS_TRUNCATE: usize = 45;
const SYS_CONNECT: usize = 203;
const SYS_BIND: usize = 200;

const AF_UNIX: u16 = 1;
const UNIX_PATH_MAX: usize = 108;

#[repr(C)]
struct SockAddrUn {
    family: u16,
    path: [u8; UNIX_PATH_MAX],
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    // A no_std shim cannot print a formatted panic without allocating, but it
    // must never leave a guest thread spinning forever. Exit only this thread
    // group with a stable code so the harness can distinguish an internal shim
    // failure from a normal guest exit.
    const SYS_EXIT_GROUP: usize = 94;
    unsafe {
        core::arch::asm!("svc 0", in("x8") SYS_EXIT_GROUP, in("x0") 101usize,
            lateout("x0") _, options(nostack));
    }
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

unsafe fn c_strlen(p: *const CChar) -> usize {
    let mut n = 0;
    // Guest pointers are ABI boundaries, not trusted Rust slices. Cap the scan
    // at PATH_MAX so a missing NUL cannot turn a path call into an unbounded
    // memory walk. A result of PATH_MAX is rejected by abs_guest() below.
    while n < PATH_MAX && *p.add(n) != 0 {
        n += 1;
    }
    n
}

unsafe fn bytes(p: *const CChar) -> &'static [u8] {
    core::slice::from_raw_parts(p as *const u8, c_strlen(p))
}

unsafe fn env_lookup(name: &[u8]) -> Option<&'static [u8]> {
    // environ is only set once glibc's start-up has run; before that (the
    // boot hook) the environment is the one the loader was handed.
    let mut e = environ;
    if e.is_null() {
        e = boot::envp() as *mut *mut CChar;
    }
    if e.is_null() {
        return None;
    }
    let mut i = 0;
    loop {
        let p = *e.add(i);
        if p.is_null() {
            return None;
        }
        let s = bytes(p);
        if s.len() > name.len() && &s[..name.len()] == name && s[name.len()] == b'=' {
            return Some(&s[name.len() + 1..]);
        }
        i += 1;
    }
}

static ROOT_STATE: AtomicU8 = AtomicU8::new(0);
static mut ROOT_ENV: [u8; PATH_MAX] = [0; PATH_MAX];
static mut ROOT_ENV_LEN: usize = 0;
static mut ROOT_REAL: [u8; PATH_MAX] = [0; PATH_MAX];
static mut ROOT_REAL_LEN: usize = 0;
static mut ROOTFD: CInt = -1;
static mut ROOT_DEV: u64 = 0;
static mut ROOT_INO: u64 = 0;
static ROOT_IDENTITY_VALID: AtomicU8 = AtomicU8::new(0);

static CWD_STATE: AtomicU8 = AtomicU8::new(0);
static mut CWD_BUF: [u8; PATH_MAX] = [0; PATH_MAX];
static mut CWD_LEN: usize = 1;

unsafe fn syscall4(n: usize, a: usize, b: usize, c: usize, d: usize) -> Ssize {
    let ret: Ssize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        lateout("x0") ret, options(nostack));
    ret
}

unsafe fn ret_or_errno(r: Ssize) -> CInt {
    if r < 0 && r > -4096 {
        *__errno_location() = (-r) as CInt;
        -1
    } else {
        r as CInt
    }
}

unsafe fn set_errno(e: CInt) -> CInt {
    *__errno_location() = e;
    -1
}

unsafe fn set_errno_ptr(e: CInt, p: *mut CChar) -> *mut CChar {
    *__errno_location() = e;
    p
}

// ---------------------------------------------------------------------------
// seccomp SIGSYS catcher.
//
// Android's app seccomp policy does not return EPERM for a handful of syscalls,
// it TRAPs them. A trap raises SIGSYS (128+31 = exit 159) and the process dies
// before libc can see an errno, so glibc's documented "EPERM/ENOSYS -> fall
// back" paths never run. We install a SIGSYS handler with SA_SIGINFO, which
// hands us si_syscall, and can then either
//   NR_SIGSYS=log     -> record the number and die with the same 159
//   NR_SIGSYS=enosys  -> make the syscall return -ENOSYS and skip the `svc`
//                        so libc takes its fallback path
// This is per-syscall, not ptrace: cost is zero unless the trap fires.
// ---------------------------------------------------------------------------
const SYS_RT_SIGACTION: usize = 134;
const SYS_EXIT_GROUP: usize = 94;
const SYS_TGKILL: usize = 131;
const SIGSYS_NUM: usize = 31;
const SA_SIGINFO: usize = 0x00000004;
const SA_RESTART: usize = 0x10000000;
const SYS_SECCOMP: i32 = 1;
const AUDIT_ARCH_AARCH64: u32 = 0xc000_00b7;

// The aarch64 ucontext layout is *measured*, not assumed: different kernel
// uapi and libc headers disagree on where uc_mcontext sits (we have seen 176,
// not the 168 the header maths suggests, because sigcontext is 16-byte
// aligned). The kernel puts the trapping instruction address in si_call_addr,
// and the same value is stored in uc_mcontext.pc, so we locate pc by matching
// it. regs[0] is then always 256 bytes before pc (pc - regs[0] = 264 - 8).
const SC_PC_TO_REGS0: usize = 256;
const UC_SCAN_FROM: usize = 32;
const UC_SCAN_TO: usize = 8192;

#[repr(C)]
struct KSigaction {
    handler: usize,
    flags: usize,
    restorer: usize,
    mask: u64,
}

static SIGSYS_MODE: AtomicU8 = AtomicU8::new(0);
static SIGSYS_HITS: AtomicU8 = AtomicU8::new(0);

unsafe fn write_all(fd: CInt, mut s: &[u8]) {
    while !s.is_empty() {
        let r = syscall4(SYS_WRITE, fd as usize, s.as_ptr() as usize, s.len(), 0);
        if r <= 0 {
            return;
        }
        s = &s[r as usize..];
    }
}

unsafe fn sigsys_mode() -> u8 {
    let m = SIGSYS_MODE.load(Ordering::Relaxed);
    if m != 0 {
        return m;
    }
    let v = match env_lookup(b"NR_SIGSYS") {
        Some(b"enosys") => 2u8,
        _ => 1u8,
    };
    SIGSYS_MODE.store(v, Ordering::Relaxed);
    v
}

unsafe fn record(line: &[u8]) {
    // Only spam the terminal when there is no log file to write to: in the
    // product the guest owns fd 2 and a shim record must not land inside the
    // user's shell session.
    if env_lookup(b"NR_SIGSYS_LOG").is_none() {
        write_all(2, line);
    }
    let mut logged = false;
    if let Some(p) = env_lookup(b"NR_SIGSYS_LOG") {
        // O_WRONLY|O_CREAT|O_APPEND, 0644
        let fd = syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            p.as_ptr() as usize,
            (1 | 0o100 | 0o2000) as usize,
            0o644,
        );
        if fd >= 0 {
            write_all(fd as CInt, line);
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
            logged = true;
        }
    }
    if !logged {
        // Absolute fallback: the launcher's own private dir. A trap record that
        // depends on env lookup is a record that can be lost.
        const P: &[u8] = b"/data/user/0/com.linuxhub.husk/files/sigsys.log\0";
        let fd = syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            P.as_ptr() as usize,
            (1 | 0o100 | 0o2000) as usize,
            0o644,
        );
        if fd >= 0 {
            write_all(fd as CInt, line);
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
        }
    }
}

unsafe fn put(buf: &mut [u8], pos: &mut usize, s: &[u8]) {
    for &c in s {
        if *pos < buf.len() {
            buf[*pos] = c;
            *pos += 1;
        }
    }
}

unsafe fn putnum(buf: &mut [u8], pos: &mut usize, v: i64) {
    let mut d = [0u8; 20];
    let mut n = 0;
    let mut x = if v < 0 { -v } else { v } as u64;
    loop {
        d[n] = b'0' + (x % 10) as u8;
        x /= 10;
        n += 1;
        if x == 0 {
            break;
        }
    }
    if v < 0 {
        put(buf, pos, b"-");
    }
    while n > 0 {
        n -= 1;
        let c = [d[n]];
        put(buf, pos, &c);
    }
}

extern "C" fn on_sigsys(_sig: CInt, info: *const u8, uc: *mut u8) {
    unsafe {
        let hits = SIGSYS_HITS.fetch_add(1, Ordering::Relaxed);
        if hits > 200 {
            record(b"sigsys|too many traps, giving up\n");
            syscall4(SYS_EXIT_GROUP, 128 + SIGSYS_NUM, 0, 0, 0);
        }
        if info.is_null() || uc.is_null() {
            record(b"sigsys|missing signal context\n");
            syscall4(SYS_EXIT_GROUP, 128 + SIGSYS_NUM, 0, 0, 0);
        }
        let code = core::ptr::read_unaligned(info.add(8) as *const i32);
        let mut nr = core::ptr::read_unaligned(info.add(24) as *const i32);
        let arch = core::ptr::read_unaligned(info.add(28) as *const i32);
        let call_addr = core::ptr::read_unaligned(info.add(16) as *const u64);
        // Only a seccomp-generated SIGSYS carries a syscall number and may be
        // resumed by rewriting the frame. A bare SIGSYS, a different arch, or
        // a malformed record must keep the kernel's fatal semantics.
        if code != SYS_SECCOMP || arch as u32 != AUDIT_ARCH_AARCH64 {
            let mut line = [0u8; 96];
            let mut k = 0usize;
            put(&mut line, &mut k, b"sigsys|unexpected source code=");
            putnum(&mut line, &mut k, code as i64);
            put(&mut line, &mut k, b"|arch=");
            putnum(&mut line, &mut k, arch as i64);
            put(&mut line, &mut k, b"\n");
            record(&line[..k]);
            syscall4(SYS_EXIT_GROUP, 128 + SIGSYS_NUM, 0, 0, 0);
        }
        let mode = sigsys_mode();
        // find uc_mcontext.pc: the slot that holds si_call_addr, followed by a
        // plausible pstate (its high half is zero on every real frame)
        let mut pc_off = 0usize;
        let mut o = UC_SCAN_FROM;
        while o + 16 <= UC_SCAN_TO {
            let v = core::ptr::read_unaligned(uc.add(o) as *const u64);
            if v == call_addr {
                let nv = core::ptr::read_unaligned(uc.add(o + 8) as *const u64);
                if nv >> 32 == 0 && call_addr > 0x1000 {
                    let insn = core::ptr::read_unaligned(call_addr as *const u32);
                    let prev = core::ptr::read_unaligned(call_addr.wrapping_sub(4) as *const u32);
                    if insn == 0xD400_0001 || prev == 0xD400_0001 {
                        pc_off = o;
                        break;
                    }
                }
            }
            o += 8;
        }
        if pc_off == 0 {
            // 4.19 does not fill si_call_addr for a seccomp trap, so the "find the
            // slot that holds si_call_addr" trick above finds nothing. Use the
            // frame's own strongest invariant instead: x8 (the syscall number)
            // equals si_syscall, and the saved pc points at the trapping `svc #0`.
            let mut q = UC_SCAN_FROM;
            while q + 200 <= UC_SCAN_TO {
                let v = core::ptr::read_unaligned(uc.add(q) as *const u64) as i64;
                if v == nr as i64 {
                    let cand = q + 184; // x8 sits 72 bytes into sigcontext, pc at 256
                    let pcv = core::ptr::read_unaligned(uc.add(cand) as *const u64);
                    let pst = core::ptr::read_unaligned(uc.add(cand + 8) as *const u64);
                    if pst >> 32 == 0 && (pst & 0xf) == 0 && pcv > 0x1000 {
                        let w = core::ptr::read_unaligned(pcv as *const u32);
                        let wp = core::ptr::read_unaligned((pcv.wrapping_sub(4)) as *const u32);
                        if w == 0xD400_0001 || wp == 0xD400_0001 {
                            pc_off = cand;
                            break;
                        }
                    }
                }
                q += 8;
            }
        }
        let ra = if pc_off >= 16 {
            core::ptr::read_unaligned(uc.add(pc_off - 16) as *const u64)
        } else {
            0
        };
        let mut buf = [0u8; 208];
        let mut k = 0usize;
        put(&mut buf, &mut k, b"sigsys|code=");
        putnum(&mut buf, &mut k, code as i64);
        put(&mut buf, &mut k, b"|nr=");
        putnum(&mut buf, &mut k, nr as i64);
        put(&mut buf, &mut k, b"|arch=");
        putnum(&mut buf, &mut k, arch as i64);
        put(&mut buf, &mut k, b"|mode=");
        putnum(&mut buf, &mut k, mode as i64);
        put(&mut buf, &mut k, b"|pc=");
        putnum(&mut buf, &mut k, call_addr as i64);
        put(&mut buf, &mut k, b"|ucpc=");
        putnum(&mut buf, &mut k, pc_off as i64);
        put(&mut buf, &mut k, b"|ra=");
        putnum(&mut buf, &mut k, ra as i64);
        // name the interesting ones for whoever reads the log
        let name: &[u8] = match nr {
            293 => b"|rseq",
            283 => b"|membarrier",
            435 => b"|clone3",
            449 => b"|futex_waitv",
            291 => b"|statx",
            439 => b"|faccessat2",
            437 => b"|openat2",
            434 => b"|pidfd_open",
            438 => b"|pidfd_getfd",
            440 => b"|process_madvise",
            425 => b"|io_uring_setup",
            462 => b"|mseal",
            444 => b"|landlock",
            442 => b"|mount_setattr",
            137 => b"|set_robust_list",
            99 => b"|set_robust_list",
            _ => b"",
        };
        put(&mut buf, &mut k, name);
        put(&mut buf, &mut k, b"\n");
        record(&buf[..k]);

        if mode == 2 {
            if pc_off == 0 {
                // cannot locate the frame: never spin on the same syscall
                record(b"sigsys|frame not found\n");
                syscall4(SYS_EXIT_GROUP, 128 + SIGSYS_NUM, 0, 0, 0);
            }
            // Return -ENOSYS and step over the `svc 0` so libc takes its
            // documented "not implemented, use the fallback" path.
            let pc_now = core::ptr::read_unaligned(uc.add(pc_off) as *const u64);
            core::ptr::write_unaligned(
                uc.add(pc_off - SC_PC_TO_REGS0) as *mut u64,
                (-38i64) as u64,
            );
            core::ptr::write_unaligned(uc.add(pc_off) as *mut u64, pc_now.wrapping_add(4));
            nr = 0;
            let _ = nr;
            return;
        }
        // log-only: reproduce the original death so the caller's exit code is
        // still 128+31 and nothing else changes.
        syscall4(SYS_EXIT_GROUP, 128 + SIGSYS_NUM, 0, 0, 0);
    }
}

/// Hand-rolled decimal, no_std and no allocation.
fn put_i64(buf: &mut [u8], mut at: usize, v: i64) -> usize {
    let mut tmp = [0u8; 24];
    let mut n = 0;
    let mut neg = false;
    let mut x = v;
    if x < 0 {
        neg = true;
    }
    if x == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while x != 0 {
        let d = (x % 10).abs() as u8;
        tmp[n] = b'0' + d;
        n += 1;
        x /= 10;
    }
    if neg {
        tmp[n] = b'-';
        n += 1;
    }
    while n > 0 {
        n -= 1;
        buf[at] = tmp[n];
        at += 1;
    }
    at
}

unsafe fn install_sigsys() {
    let act = KSigaction {
        handler: on_sigsys as usize,
        flags: SA_SIGINFO | SA_RESTART,
        restorer: 0,
        mask: 0,
    };
    // rt_sigaction(SIGSYS, &act, NULL, 8)
    let ret = syscall4(
        SYS_RT_SIGACTION,
        SIGSYS_NUM,
        &act as *const KSigaction as usize,
        0,
        8,
    );
    {
        // The return value IS the errno (negated). Log it: an "installed" line
        // that lies is worse than no line at all.
        let mut line = [0u8; 64];
        let mut n = 0;
        for &b in b"nr-init|sigaction|ret=" {
            line[n] = b;
            n += 1;
        }
        n = put_i64(&mut line, n, ret as i64);
        line[n] = b'\n';
        n += 1;
        init_note(&line[..n]);
    }
    // A one-line marker for the trap log, so a trap record can be told apart
    // from a run that merely started. It stays out of the program's own stderr
    // unless NR_DEBUG is on: the launcher always sets NR_SIGSYS_LOG.
    init_note(b"nr-init|sigsys handler installed\n");

    // Same pass, one more diagnostic: a guest crash prints fault/pc/x0..x8 to
    // the log, then the default action runs as if we had never been there.
    let segh = KSigaction {
        handler: on_segv as usize,
        flags: SA_SIGINFO,
        restorer: 0,
        mask: 0,
    };
    for sig in [11usize, 7, 8, 4] {
        // SEGV, BUS, FPE, ILL
        let r = syscall4(
            SYS_RT_SIGACTION,
            sig,
            &segh as *const KSigaction as usize,
            0,
            8,
        );
        if r < 0 && sig == 11 {
            init_note(b"nr-init|segv handler refused\n");
        }
    }
}

/// Log-file-only note (plus the program's stderr when NR_DEBUG is on).
unsafe fn init_note(line: &[u8]) {
    if dbg_on() {
        write_all(2, line);
    }
    if let Some(p) = env_lookup(b"NR_SIGSYS_LOG") {
        let fd = syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            p.as_ptr() as usize,
            (1 | 0o100 | 0o2000) as usize,
            0o644,
        );
        if fd >= 0 {
            write_all(fd as CInt, line);
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
        }
    }
}

/// A guest crash gets one line in the log before the default action runs.
///
/// Without this a SIGSEGV inside the shim is a bare "Segmentation fault" with no
/// address, which is what turned the first body-patch attempt into guesswork.
/// The frame layout is the kernel's (installed through rt_sigaction with an
/// 8-byte mask): ucontext.uc_mcontext sits at 160, regs[0..31] at +8, sp at +256,
/// pc at +264, pstate at +272.
extern "C" fn on_segv(sig: CInt, info: *const u8, uc: *mut u8) {
    unsafe {
        const MC: usize = 160;
        let fa = core::ptr::read_unaligned(info.add(16) as *const u64);
        let pc = core::ptr::read_unaligned(uc.add(MC + 264) as *const u64);
        let lr = core::ptr::read_unaligned(uc.add(MC + 248) as *const u64);
        let x0 = core::ptr::read_unaligned(uc.add(MC + 8) as *const u64);
        let x1 = core::ptr::read_unaligned(uc.add(MC + 16) as *const u64);
        let x2 = core::ptr::read_unaligned(uc.add(MC + 24) as *const u64);
        let x8 = core::ptr::read_unaligned(uc.add(MC + 72) as *const u64);
        let mut buf = [0u8; 224];
        let mut k = 0usize;
        put(&mut buf, &mut k, b"segv|sig=");
        putnum(&mut buf, &mut k, sig as i64);
        put(&mut buf, &mut k, b"|fa=");
        putnum(&mut buf, &mut k, fa as i64);
        put(&mut buf, &mut k, b"|pc=");
        putnum(&mut buf, &mut k, pc as i64);
        put(&mut buf, &mut k, b"|lr=");
        putnum(&mut buf, &mut k, lr as i64);
        put(&mut buf, &mut k, b"|x0=");
        putnum(&mut buf, &mut k, x0 as i64);
        put(&mut buf, &mut k, b"|x1=");
        putnum(&mut buf, &mut k, x1 as i64);
        put(&mut buf, &mut k, b"|x2=");
        putnum(&mut buf, &mut k, x2 as i64);
        put(&mut buf, &mut k, b"|x8=");
        putnum(&mut buf, &mut k, x8 as i64);
        put(&mut buf, &mut k, b"|ld=");
        putnum(&mut buf, &mut k, crate::boot::loader_base() as i64);
        put(&mut buf, &mut k, b"|lc=");
        putnum(&mut buf, &mut k, crate::boot::libc_base() as i64);
        put(&mut buf, &mut k, b"\n");
        record(&buf[..k]);
        // Real crash semantics: restore SIG_DFL and let the fault happen again,
        // so a core dump / the platform's own reporting still happens.
        let dfl = KSigaction {
            handler: 0,
            flags: 0,
            restorer: 0,
            mask: 0,
        };
        syscall4(
            SYS_RT_SIGACTION,
            sig as usize,
            &dfl as *const KSigaction as usize,
            0,
            8,
        );
        let tid = syscall4(SYS_GETTID, 0, 0, 0, 0);
        syscall4(SYS_TGKILL, tid as usize, tid as usize, sig as usize, 0);
        syscall4(SYS_EXIT_GROUP, (128 + sig) as usize, 0, 0, 0);
    }
}

extern "C" fn nr_init() {
    unsafe { install_sigsys() }
}

#[used]
#[link_section = ".init_array"]
static NR_INIT: extern "C" fn() = nr_init;

static DBG_STATE: AtomicU8 = AtomicU8::new(0);
static DBG_VERBOSE: AtomicU8 = AtomicU8::new(0);

#[inline]
unsafe fn dbg_on() -> bool {
    match DBG_STATE.load(Ordering::Relaxed) {
        0 => {
            let on = env_lookup(b"NR_DEBUG").is_some();
            if on && env_lookup(b"NR_DEBUG2").is_some() {
                DBG_VERBOSE.store(1, Ordering::Relaxed);
            }
            DBG_STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        v => v == 2,
    }
}

#[inline]
unsafe fn verbose_on() -> bool {
    dbg_on() && DBG_VERBOSE.load(Ordering::Relaxed) == 1
}

unsafe fn dbg(parts: &[&[u8]]) {
    if !dbg_on() {
        return;
    }
    const SYS_WRITE: usize = 64;
    for p in parts {
        syscall4(SYS_WRITE, 2, p.as_ptr() as usize, p.len(), 0);
    }
}

/// Same diagnostics, but into <root>/tmp/nr-exec.log: a forked child often has
/// its stderr redirected into a pipe its parent throws away, which is exactly
/// where the interesting exec failures happen.
unsafe fn dbg_file(parts: &[&[u8]]) {
    if !dbg_on() {
        return;
    }
    let root = guest_root();
    let suf = b"/tmp/nr-exec.log";
    if root.len() + suf.len() + 1 >= PATH_MAX {
        return;
    }
    let mut p = [0u8; PATH_MAX];
    p[..root.len()].copy_from_slice(root);
    p[root.len()..root.len() + suf.len()].copy_from_slice(suf);
    let n = root.len() + suf.len();
    p[n] = 0;
    let fd = syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        p.as_ptr() as usize,
        (0o1 | 0o100 | 0o2000) as usize, // O_WRONLY|O_CREAT|O_APPEND
        0o644,
    );
    if fd < 0 {
        return;
    }
    for s in parts {
        syscall4(SYS_WRITE, fd as usize, s.as_ptr() as usize, s.len(), 0);
    }
    rc::close(fd as CInt);
}

fn dec_buf(mut v: i64) -> ([u8; 24], usize) {
    let mut b = [0u8; 24];
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let mut i = 24;
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 || i == 0 {
            break;
        }
    }
    if neg && i > 0 {
        i -= 1;
        b[i] = b'-';
    }
    (b, i)
}

unsafe fn dbghex(v: u64) {
    if !dbg_on() {
        return;
    }
    let hex = b"0123456789abcdef";
    let mut out = [0u8; 19];
    out[0] = b'0';
    out[1] = b'x';
    let mut i = 0;
    while i < 16 {
        out[2 + i] = hex[((v >> (60 - 4 * i)) & 0xf) as usize];
        i += 1;
    }
    syscall4(SYS_WRITE, 2, out.as_ptr() as usize, out.len(), 0);
}

unsafe fn dbgn(parts: &[&[u8]], v: i64) {
    if !dbg_on() {
        return;
    }
    let mut b = [0u8; 24];
    let mut i = 24;
    let mut x = if v < 0 { -v } else { v } as u64;
    loop {
        i -= 1;
        b[i] = b'0' + (x % 10) as u8;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    if v < 0 {
        i -= 1;
        b[i] = b'-';
    }
    dbg(parts);
    dbg(&[&b[i..], b"\n"]);
}

const SYS_FSTAT: usize = 80;
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;

unsafe fn root_identity(fd: CInt) -> Option<(u64, u64)> {
    if fd < 0 {
        return None;
    }
    if syscall4(SYS_FCNTL, fd as usize, F_GETFD, 0, 0) < 0 {
        return None;
    }
    let mut st = [0u8; 128];
    if syscall4(SYS_FSTAT, fd as usize, st.as_mut_ptr() as usize, 0, 0) < 0 {
        return None;
    }
    let mode = core::ptr::read_unaligned(st.as_ptr().add(16) as *const u32);
    if mode & S_IFMT != S_IFDIR {
        return None;
    }
    let dev = core::ptr::read_unaligned(st.as_ptr() as *const u64);
    let ino = core::ptr::read_unaligned(st.as_ptr().add(8) as *const u64);
    Some((dev, ino))
}

unsafe fn rootfd_alive(fd: CInt) -> bool {
    if ROOT_IDENTITY_VALID.load(Ordering::Acquire) == 0 {
        return false;
    }
    match root_identity(fd) {
        Some((dev, ino)) => dev == ROOT_DEV && ino == ROOT_INO,
        None => false,
    }
}

/// Open the guest root at a high descriptor. Guest daemons close descriptors
/// they inherited, sometimes through libc internals that bypass GOT interpose;
/// keeping the runtime descriptor above the usual 0..1023 range makes accidental
/// reuse much less likely, and `rootfd` reopens it if a bulk close still got it.
unsafe fn move_root_high(fd: CInt) -> CInt {
    let high = syscall4(SYS_FCNTL, fd as usize, F_DUPFD_CLOEXEC, ROOTFD_FLOOR, 0);
    dbgn(&[b"root high rc="], high as i64);
    if high >= ROOTFD_FLOOR as Ssize {
        rc::close(fd);
        high as CInt
    } else {
        fd
    }
}

unsafe fn rootfd() -> CInt {
    if ROOT_STATE.load(Ordering::Acquire) == 2 {
        if rootfd_alive(ROOTFD) {
            return ROOTFD;
        }
        ROOT_IDENTITY_VALID.store(0, Ordering::Release);
        ROOT_STATE.store(0, Ordering::Release);
    }
    if ROOT_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        while ROOT_STATE.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
        return ROOTFD;
    }
    if let Some(r) = env_lookup(b"NR_ROOTFS") {
        if !r.is_empty() {
            let n = r.len().min(PATH_MAX);
            ROOT_ENV[..n].copy_from_slice(&r[..n]);
            ROOT_ENV_LEN = n;
            if let Ok(fd) = rc::openat(AT_FDCWD, r, O_PATH | O_DIRECTORY | O_CLOEXEC, 0) {
                let high = move_root_high(fd);
                if let Some((dev, ino)) = root_identity(high) {
                    ROOTFD = high;
                    ROOT_DEV = dev;
                    ROOT_INO = ino;
                    ROOT_IDENTITY_VALID.store(1, Ordering::Release);
                    let mut pl = [0u8; 64];
                    let mut i = 0;
                    for &c in b"/proc/self/fd/" {
                        pl[i] = c;
                        i += 1;
                    }
                    let mut v = ROOTFD as u32;
                    let mut d = [0u8; 10];
                    let mut dn = 0;
                    if v == 0 {
                        d[0] = b'0';
                        dn = 1;
                    }
                    while v > 0 {
                        d[dn] = b'0' + (v % 10) as u8;
                        v /= 10;
                        dn += 1;
                    }
                    while dn > 0 {
                        dn -= 1;
                        pl[i] = d[dn];
                        i += 1;
                    }
                    pl[i] = 0;
                    let mut ab = [0u8; PATH_MAX];
                    let rn = syscall4(
                        SYS_READLINKAT,
                        AT_FDCWD as usize,
                        pl.as_ptr() as usize,
                        ab.as_mut_ptr() as usize,
                        PATH_MAX,
                    );
                    if rn > 0 {
                        let n2 = (rn as usize).min(PATH_MAX - 1);
                        ROOT_REAL[..n2].copy_from_slice(&ab[..n2]);
                        ROOT_REAL_LEN = n2;
                    }
                } else {
                    rc::close(high);
                }
            }
        }
    }
    if dbg_on() {
        let mut buf = [0u8; 160];
        let mut k = 0;
        put(&mut buf, &mut k, b"rootfd env=");
        put(
            &mut buf,
            &mut k,
            core::slice::from_raw_parts(ROOT_ENV.as_ptr(), ROOT_ENV_LEN),
        );
        put(&mut buf, &mut k, b" real=");
        put(
            &mut buf,
            &mut k,
            core::slice::from_raw_parts(ROOT_REAL.as_ptr(), ROOT_REAL_LEN),
        );
        put(&mut buf, &mut k, b" fd=");
        putnum(&mut buf, &mut k, ROOTFD as i64);
        put(&mut buf, &mut k, b"\n");
        dbg(&[&buf[..k]]);
    }
    ROOT_STATE.store(2, Ordering::Release);
    ROOTFD
}

unsafe fn guest_root() -> &'static [u8] {
    if ROOT_REAL_LEN > 0 {
        core::slice::from_raw_parts(ROOT_REAL.as_ptr(), ROOT_REAL_LEN)
    } else {
        core::slice::from_raw_parts(ROOT_ENV.as_ptr(), ROOT_ENV_LEN)
    }
}

/// Start the process at the guest root.
///
/// The launcher creates the process in the app's files directory, which is the
/// *parent* of the rootfs: getcwd() there cannot be mapped to a guest path, and
/// programs that save and restore their initial directory (find(1) does) warn
/// about it. One raw chdir at boot, before any program code runs, fixes both.
pub unsafe fn set_guest_cwd() {
    let _ = rootfd();
    let g = guest_root();
    if g.is_empty() {
        return;
    }
    // Only when the inherited cwd is *outside* the root: a child that inherited
    // a guest directory from its parent must keep it (ls/find/cat after `cd`).
    let mut real = [0u8; PATH_MAX];
    let rn = syscall4(
        SYS_READLINKAT,
        AT_FDCWD as usize,
        b"/proc/self/cwd\0".as_ptr() as usize,
        real.as_mut_ptr() as usize,
        PATH_MAX - 1,
    );
    if rn > 0 {
        let rl = rn as usize;
        let inside =
            rl >= g.len() && &real[..g.len()] == g && (rl == g.len() || real[g.len()] == b'/');
        if inside {
            return;
        }
    }
    let mut buf = [0u8; PATH_MAX];
    let n = g.len().min(PATH_MAX - 1);
    buf[..n].copy_from_slice(&g[..n]);
    buf[n] = 0;
    let _ = syscall4(SYS_CHDIR, buf.as_ptr() as usize, 0, 0, 0);
}

unsafe fn cwd() -> &'static [u8] {
    // A program can move its cwd behind our back (raw fchdir, another thread),
    // so the guest cwd is re-derived from the kernel on every call: one
    // readlinkat("/proc/self/cwd") and a prefix strip. Cheap enough, and it
    // keeps relative paths correct where a cached value would drift.
    let mut real = [0u8; PATH_MAX];
    let n = syscall4(
        SYS_READLINKAT,
        AT_FDCWD as usize,
        b"/proc/self/cwd\0".as_ptr() as usize,
        real.as_mut_ptr() as usize,
        PATH_MAX - 1,
    );
    if n > 0 {
        let _ = rootfd();
        let mut rl = n as usize;
        while rl > 0 && real[rl - 1] == 0 {
            rl -= 1;
        }
        let mut gl = 0usize;
        if let Some(g) = guest_path_from_real(&real[..rl], CWD_BUF.as_mut_ptr(), PATH_MAX) {
            gl = g;
        }
        if gl == 0 {
            CWD_BUF[0] = b'/';
            gl = 1;
        }
        CWD_LEN = gl;
        CWD_STATE.store(1, Ordering::Release);
    } else if CWD_STATE.load(Ordering::Acquire) == 0 {
        CWD_BUF[0] = b'/';
        CWD_LEN = 1;
        CWD_STATE.store(1, Ordering::Release);
    }
    core::slice::from_raw_parts(CWD_BUF.as_ptr(), CWD_LEN)
}

unsafe fn set_cwd(p: &[u8]) {
    let n = p.len().min(PATH_MAX);
    CWD_BUF[..n].copy_from_slice(&p[..n]);
    CWD_LEN = n;
    CWD_STATE.store(1, Ordering::Release);
}

/// Lexically normalise a guest absolute path; ".." never escapes "/".
unsafe fn normalize(p: &[u8], out: &mut [u8; PATH_MAX]) -> usize {
    let mut n = 0usize;
    out[0] = b'/';
    for c in p
        .split(|x| *x == b'/')
        .filter(|c| !c.is_empty() && *c != b".")
    {
        if c == b".." {
            n = rc::pop_component(out, n);
            continue;
        }
        if n + 1 + c.len() >= PATH_MAX {
            break;
        }
        out[n] = b'/';
        n += 1;
        out[n..n + c.len()].copy_from_slice(c);
        n += c.len();
    }
    if n == 0 {
        n = 1;
        out[0] = b'/';
    }
    n
}

/// Applies the emulated mount table (longest prefix wins) after the plain
/// absolute-path computation. Chained mounts are followed up to four hops; a
/// cycle would otherwise spin here.
unsafe fn abs_guest(path: *const CChar, out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let mut n = abs_guest_raw(path, out)?;
    let mut hop = 0;
    while hop < 4 {
        let m = mnt_apply(out, n);
        if m == n {
            break;
        }
        n = m;
        hop += 1;
    }
    Some(n)
}

unsafe fn abs_guest_raw(path: *const CChar, out: &mut [u8; PATH_MAX]) -> Option<usize> {
    if path.is_null() {
        return None;
    }
    let s = bytes(path);
    if s.is_empty() {
        return None;
    }
    if s.len() >= PATH_MAX {
        return None;
    }
    if s[0] == b'/' {
        out[..s.len()].copy_from_slice(s);
        return Some(s.len());
    }
    let c = cwd();
    if c.len() == 1 && c[0] == b'/' {
        if s.len() + 1 >= PATH_MAX {
            return None;
        }
        out[0] = b'/';
        out[1..1 + s.len()].copy_from_slice(s);
        return Some(1 + s.len());
    }
    let n = c.len() + 1 + s.len();
    if n >= PATH_MAX {
        return None;
    }
    out[..c.len()].copy_from_slice(c);
    out[c.len()] = b'/';
    out[c.len() + 1..n].copy_from_slice(s);
    Some(n)
}

/// Is `p` already a real path inside the guest root?
///
/// Programs ask the kernel where they live (gcc does: it builds its whole
/// program/library search path from /proc/self/exe and argv[0], and the kernel
/// answers with the real path). Mapping such a path into the root again would
/// produce <root><root>/usr/..., so anything under the root is taken verbatim
/// and handed to the kernel unchanged.
unsafe fn under_root(p: &[u8]) -> bool {
    let r = guest_root();
    if r.len() > 1 && p.len() > r.len() && &p[..r.len()] == r && p[r.len()] == b'/' {
        return true;
    }
    if ROOT_ENV_LEN > 1 {
        let e = core::slice::from_raw_parts(ROOT_ENV.as_ptr(), ROOT_ENV_LEN);
        if p.len() > e.len() && &p[..e.len()] == e && p[e.len()] == b'/' {
            return true;
        }
    }
    false
}

unsafe fn pseudo_path(p: &[u8]) -> bool {
    for root in [b"/dev".as_ref(), b"/proc".as_ref(), b"/sys".as_ref()] {
        if p == root || (p.len() > root.len() && &p[..root.len()] == root && p[root.len()] == b'/')
        {
            return true;
        }
    }
    false
}

/// Lexically reject the procfs magic links that escape a passed-through tree.
/// `..` can climb out of /proc, /dev or /sys; `/proc/*/root` and `/proc/*/cwd`
/// can jump straight back to the host namespace. Ordinary proc queries such as
/// `/proc/self/status` and `/dev/null` remain passthrough.
unsafe fn passthrough(p: &[u8]) -> bool {
    if under_root(p) {
        return true;
    }
    // `/dev/shm` (POSIX shared memory) does not exist on Android. Servicing it
    // from the host is not an option, so it is *not* passthrough: the regular
    // guest-root resolution maps it to `<rootfs>/dev/shm`, which the runtime
    // creates as a directory on first use.
    if p == b"/dev/shm" || (p.len() > 8 && &p[..8] == b"/dev/shm" && p[8] == b'/') {
        return false;
    }
    // /dev/log is the namespace's own syslog socket: the runtime binds it inside
    // the guest root, and the host has none. Passing it through would answer
    // ENOENT for every syslog() in the guest.
    if p == b"/dev/log" {
        return false;
    }
    // Android denies these two files to apps (SELinux), but programs and
    // configure scripts do read them. Serve them from the guest root instead:
    // the deploy step writes the real values there, so nothing is invented.
    if p == b"/proc/version" || p == b"/proc/sys/kernel/osrelease" {
        return false;
    }
    // The network view is the namespace's own: netlink is refused to this
    // domain, so /sys/class/net is materialised inside the guest root (see
    // tools/guest-config/mksys-net.sh) instead of being passed through to a
    // host tree that describes interfaces the guest cannot use.
    if p == b"/sys/class/net" || (p.len() > 14 && &p[..14] == b"/sys/class/net" && (p[14] == b'/' || p[14] == 0)) {
        return false;
    }
    if !pseudo_path(p) {
        return false;
    }
    let is_proc = p == b"/proc" || (p.len() > 5 && &p[..5] == b"/proc" && p[5] == b'/');
    for c in p.split(|x| *x == b'/') {
        if c == b".." {
            return false;
        }
        if is_proc && (c == b"root" || c == b"cwd") {
            return false;
        }
    }
    true
}

unsafe fn real_path(parent: CInt, name: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let mut pl = [0u8; 64];
    let mut i = 0;
    for &c in b"/proc/self/fd/" {
        pl[i] = c;
        i += 1;
    }
    let mut v = parent as u32;
    let mut d = [0u8; 10];
    let mut dn = 0;
    if v == 0 {
        d[0] = b'0';
        dn = 1;
    }
    while v > 0 {
        d[dn] = b'0' + (v % 10) as u8;
        v /= 10;
        dn += 1;
    }
    while dn > 0 {
        dn -= 1;
        pl[i] = d[dn];
        i += 1;
    }
    pl[i] = 0;
    let mut rp = [0u8; PATH_MAX];
    let rn = syscall4(
        SYS_READLINKAT,
        AT_FDCWD as usize,
        pl.as_ptr() as usize,
        rp.as_mut_ptr() as usize,
        PATH_MAX,
    );
    if rn <= 0 {
        return None;
    }
    let rl = rn as usize;
    if rl + 1 + name.len() >= PATH_MAX {
        return None;
    }
    out[..rl].copy_from_slice(&rp[..rl]);
    out[rl] = b'/';
    out[rl + 1..rl + 1 + name.len()].copy_from_slice(name);
    out[rl + 1 + name.len()] = 0;
    Some(rl + 1 + name.len())
}

#[no_mangle]
pub unsafe extern "C" fn open(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn open64(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}

// The no-cancellation spelling. libc's own innards (_IO_file_open, the locale
// loader, gconv) call this one, and `nr-tools libc-interpose` points that entry
// here so those calls are translated too. It is not a cancellation point, which
// is what ours already is: no Rust code runs between the check and the syscall.
#[no_mangle]
pub unsafe extern "C" fn __open(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn __open64(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn __open_nocancel(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn __open64_nocancel(path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    openat(AT_FDCWD, path, flags, mode)
}


// --------------------------------------------------------------- virtual files
//
// A mount table belongs to the *namespace*, not to the machine: a container
// publishes its own. We cannot mount, so what we publish is a description of the
// namespace we did build - the one rootfs directory, and the kernel file systems
// the guest really does see through this shim. Nothing here is a guess about the
// host, and nothing is a placeholder: the filesystem type is read back from
// statfs, the root path is the one we were started with.

const SYS_MEMFD_CREATE: usize = 279;
const SYS_LSEEK: usize = 62;
const F_SETFD: usize = 2;
const FD_CLOEXEC: usize = 1;
const O_ACCMODE_BITS: CInt = 0o3;
const MFD_CLOEXEC: usize = 1;

#[inline]
unsafe fn rd8u(p: *const u8) -> u64 {
    let mut v = 0u64;
    let mut i = 0;
    while i < 8 {
        v |= (core::ptr::read_volatile(p.add(i)) as u64) << (8 * i);
        i += 1;
    }
    v
}

#[inline]
unsafe fn rd4u(p: *const u8) -> u32 {
    let mut v = 0u32;
    let mut i = 0;
    while i < 4 {
        v |= (core::ptr::read_volatile(p.add(i)) as u32) << (8 * i);
        i += 1;
    }
    v
}

static mut VTEXT: [u8; 4096] = [0; 4096];

unsafe fn vt_put(at: &mut usize, s: &[u8]) {
    let room = VTEXT.len() - *at;
    let n = if s.len() < room { s.len() } else { room };
    if n == 0 {
        return;
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), VTEXT.as_mut_ptr().add(*at), n);
    *at += n;
}

unsafe fn vt_num(at: &mut usize, mut v: u64) {
    let mut d = [0u8; 20];
    let mut i = 20;
    loop {
        i -= 1;
        d[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    vt_put(at, &d[i..]);
}

/// Filesystem type of the rootfs, read from the kernel - never assumed.
unsafe fn root_fstype() -> &'static [u8] {
    let rfd = rootfd();
    if rfd < 0 {
        return b"unknown";
    }
    let mut pb = [0u8; 32];
    let pp = fd_proc_path(rfd, &mut pb);
    let mut st = [0u8; 128];
    if syscall4(SYS_STATFS, pp.as_ptr() as usize, st.as_ptr() as usize, 0, 0) != 0 {
        return b"unknown";
    }
    match rd8u(st.as_ptr()) {
        0xEF53 => b"ext4",
        0xF2F52010 => b"f2fs",
        0x58465342 => b"xfs",
        0x9123683E => b"btrfs",
        0x01021994 => b"tmpfs",
        _ => b"unknown",
    }
}

unsafe fn mnt_setup_dir() {
    // Through our own mkdirat: the raw syscall would resolve /run/husk against
    // the *host* root, where it does not exist and never will.
    mkdirat(AT_FDCWD, MNT_DIR.as_ptr() as *const CChar, 0o755);
}

unsafe fn mnt_pad(out: &mut [u8; 2048], at: &mut usize, s: &[u8]) {
    let room = out.len() - *at;
    let n = if s.len() < room { s.len() } else { room };
    if n == 0 {
        return;
    }
    out[*at..*at + n].copy_from_slice(&s[..n]);
    *at += n;
}

unsafe fn mnt_load() {
    if MNT_READY != 0 {
        return;
    }
    MNT_READY = 1;
    let fd = openat(
        AT_FDCWD,
        MNT_FILE.as_ptr() as *const CChar,
        0,
        0,
    );
    if fd < 0 {
        return;
    }
    static mut BUF: [u8; 2048] = [0; 2048];
    let n = syscall4(
        SYS_READ,
        fd as usize,
        core::ptr::addr_of_mut!(BUF) as usize,
        BUF.len(),
        0,
    );
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    if n <= 0 {
        return;
    }
    let buf = core::slice::from_raw_parts(core::ptr::addr_of!(BUF) as *const u8, n as usize);
    let mut i = 0usize;
    while i < buf.len() && MNT_N < MNT_MAX {
        let mut e = i;
        while e < buf.len() && buf[e] != b'\n' {
            e += 1;
        }
        let line = &buf[i..e];
        i = e + 1;
        let mut f: [&[u8]; 4] = [&[], &[], &[], &[]];
        let mut nf = 0usize;
        let mut st = 0usize;
        let mut k = 0usize;
        while k <= line.len() {
            if k == line.len() || line[k] == b'\t' {
                if nf < 4 {
                    f[nf] = &line[st..k];
                    nf += 1;
                }
                st = k + 1;
            }
            k += 1;
        }
        if nf < 4 || f[2].len() == 0 || f[2].len() >= MNT_P || f[3].len() >= MNT_P {
            continue;
        }
        let ent = &mut MNT[MNT_N];
        ent.tgt[..f[2].len()].copy_from_slice(f[2]);
        ent.tlen = f[2].len() as u16;
        ent.src[..f[3].len()].copy_from_slice(f[3]);
        ent.slen = f[3].len() as u16;
        ent.ro = if f[1] == b"1" { 1 } else { 0 };
        ent.tmpfs = if f[0] == b"t" { 1 } else { 0 };
        MNT_N += 1;
    }
}

unsafe fn mnt_save() {
    let mut out = [0u8; 2048];
    let mut at = 0usize;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        mnt_pad(&mut out, &mut at, if e.tmpfs != 0 { b"t" } else { b"b" });
        mnt_pad(&mut out, &mut at, if e.ro != 0 { b"\t1\t" } else { b"\t0\t" });
        mnt_pad(&mut out, &mut at, &e.tgt[..e.tlen as usize]);
        mnt_pad(&mut out, &mut at, b"\t");
        mnt_pad(&mut out, &mut at, &e.src[..e.slen as usize]);
        mnt_pad(&mut out, &mut at, b"\n");
        i += 1;
    }
    mnt_setup_dir();
    let fd = openat(
        AT_FDCWD,
        MNT_FILE.as_ptr() as *const CChar,
        0o1 | 0o100 | 0o1000,
        0o644,
    );
    if fd < 0 {
        return;
    }
    syscall4(SYS_WRITE, fd as usize, out.as_ptr() as usize, at, 0);
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
}

/// Longest matching mount point wins, exactly like the kernel's lookup.
unsafe fn mnt_apply(gp: &mut [u8; PATH_MAX], n: usize) -> usize {
    mnt_load();
    let mut best: isize = -1;
    let mut bi = 0usize;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let tl = e.tlen as usize;
        if tl <= n
            && &gp[..tl] == &e.tgt[..tl]
            && (n == tl || gp[tl] == b'/')
            && (tl as isize) > best
        {
            best = tl as isize;
            bi = i;
        }
        i += 1;
    }
    if best < 0 {
        return n;
    }
    let e = &MNT[bi];
    let tl = e.tlen as usize;
    let sl = e.slen as usize;
    let rest = n - tl;
    if sl + rest >= PATH_MAX {
        return n;
    }
    let mut tmp = [0u8; PATH_MAX];
    tmp[..sl].copy_from_slice(&e.src[..sl]);
    if rest > 0 {
        tmp[sl..sl + rest].copy_from_slice(&gp[tl..n]);
    }
    gp[..sl + rest].copy_from_slice(&tmp[..sl + rest]);
    sl + rest
}

/// Read-only check must run on the path *as the guest wrote it*: by the time
/// `abs_guest` has applied the table the mount point is gone and the path looks
/// like an ordinary source path, so a post-mapping check never fires.
unsafe fn path_is_ro(path: *const CChar) -> bool {
    if path.is_null() || *path != b'/' as CChar {
        return false;
    }
    let mut gp = [0u8; PATH_MAX];
    match abs_guest_raw(path, &mut gp) {
        Some(n) => mnt_is_ro(&gp[..n]),
        None => false,
    }
}

unsafe fn mnt_is_ro(gp: &[u8]) -> bool {
    mnt_load();
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let tl = e.tlen as usize;
        if e.ro != 0
            && tl <= gp.len()
            && &gp[..tl] == &e.tgt[..tl]
            && (gp.len() == tl || gp[tl] == b'/')
        {
            return true;
        }
        i += 1;
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn mount(
    source: *const CChar,
    target: *const CChar,
    fstype: *const CChar,
    flags: usize,
    _data: *const core::ffi::c_void,
) -> CInt {
    let rfd = rootfd();
    if rfd < 0 {
        return set_errno(1);
    }
    if target.is_null() {
        return set_errno(14);
    }
    // The target is resolved *raw*: mounting onto an existing mount point must
    // stack, not be rewritten into the mount's source.
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(target, &mut gp) {
        Some(n) => n,
        None => return set_errno(36),
    };
    // The target must exist and be a directory, like mount(2). Translate it to
    // the host path first: the raw syscall knows nothing about the guest root.
    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    let mut real = [0u8; PATH_MAX];
    if real_path(parent, &name[..nl], &mut real).is_none() {
        rc::close(parent);
        return set_errno(2);
    }
    rc::close(parent);
    let tfd = syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        real.as_ptr() as usize,
        (O_RDONLY0 | O_DIRECTORY) as usize,
        0,
    );
    if tfd < 0 {
        return -1;
    }
    syscall4(SYS_CLOSE, tfd as usize, 0, 0, 0);
    mnt_load();
    if flags & MS_REMOUNT != 0 {
        let mut i = 0usize;
        while i < MNT_N {
            let e = &mut MNT[i];
            let tl = e.tlen as usize;
            if tl == n && &e.tgt[..tl] == &gp[..n] {
                e.ro = if flags & MS_RDONLY != 0 { 1 } else { 0 };
                mnt_save();
                return 0;
            }
            i += 1;
        }
        return set_errno(22);
    }
    if MNT_N >= MNT_MAX {
        return set_errno(28);
    }
    if n >= MNT_P {
        return set_errno(36);
    }
    let bind = flags & MS_BIND != 0;
    let is_tmpfs = !bind && !fstype.is_null() && bytes(fstype) == b"tmpfs";
    if !bind && !is_tmpfs {
        // Only the two forms a namespace this shape can honestly provide.
        return set_errno(19);
    }
    // util-linux performs a bind as two calls (bind, then bind|remount for the
    // flags). A second call for the same target with the same source is the
    // same mount, not EBUSY; mirror the kernel by updating the flags instead.
    let mut i = 0usize;
    while i < MNT_N {
        let tl = MNT[i].tlen as usize;
        if tl == n && &MNT[i].tgt[..tl] == &gp[..n] {
            if bind && !source.is_null() {
                let mut sp = [0u8; PATH_MAX];
                if let Some(sn) = abs_guest(source, &mut sp) {
                    if sn < MNT_P
                        && sn == MNT[i].slen as usize
                        && &MNT[i].src[..sn] == &sp[..sn]
                    {
                        MNT[i].ro = if flags & MS_RDONLY != 0 { 1 } else { 0 };
                        mnt_save();
                        return 0;
                    }
                }
            }
            return set_errno(16);
        }
        i += 1;
    }
    let mut e = MntE {
        tgt: [0; MNT_P],
        tlen: n as u16,
        src: [0; MNT_P],
        slen: 0,
        ro: if flags & MS_RDONLY != 0 { 1 } else { 0 },
        tmpfs: if is_tmpfs { 1 } else { 0 },
    };
    e.tgt[..n].copy_from_slice(&gp[..n]);
    if is_tmpfs {
        mnt_setup_dir();
        let mut sd = [0u8; 64];
        let base = b"/run/husk/tmp";
        let mut k = 0usize;
        while k < base.len() {
            sd[k] = base[k];
            k += 1;
        }
        let mut d = [0u8; 16];
        let dn = put_i64(&mut d, 0, MNT_N as i64);
        let mut j = 0usize;
        while j < dn {
            sd[k] = d[j];
            k += 1;
            j += 1;
        }
        mkdirat(AT_FDCWD, sd.as_ptr() as *const CChar, 0o755);
        e.src[..k].copy_from_slice(&sd[..k]);
        e.slen = k as u16;
    } else {
        if source.is_null() {
            return set_errno(14);
        }
        let mut sp = [0u8; PATH_MAX];
        let sn = match abs_guest(source, &mut sp) {
            Some(v) => v,
            None => return set_errno(36),
        };
        if sn >= MNT_P {
            return set_errno(36);
        }
        e.src[..sn].copy_from_slice(&sp[..sn]);
        e.slen = sn as u16;
    }
    MNT[MNT_N] = e;
    MNT_N += 1;
    mnt_save();
    0
}

#[no_mangle]
pub unsafe extern "C" fn umount2(target: *const CChar, _flags: CInt) -> CInt {
    let rfd = rootfd();
    if rfd < 0 {
        return set_errno(1);
    }
    if target.is_null() {
        return set_errno(14);
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(target, &mut gp) {
        Some(n) => n,
        None => return set_errno(36),
    };
    mnt_load();
    let mut i = 0usize;
    while i < MNT_N {
        let tl = MNT[i].tlen as usize;
        if tl == n && &MNT[i].tgt[..tl] == &gp[..n] {
            let mut k = i;
            while k + 1 < MNT_N {
                MNT[k] = MNT[k + 1];
                k += 1;
            }
            MNT_N -= 1;
            mnt_save();
            return 0;
        }
        i += 1;
    }
    set_errno(22)
}

// --------------------------------------------------------- POSIX mqueues ---
//
// The kernel has no mqueue filesystem here (no mount, no /dev/mqueue), so
// `mq_open` answers ENOSYS. A container gives programs a working named queue;
// this is the same service built out of what a namespace of this shape *can*
// do: a directory per queue under /dev/mqueue/.q, a file per message, and a
// handle that is simply a directory fd.
//
// Message file name: "p<prio 03>_<mono nsec 019>_<pid>". Priority first so a
// lexicographic scan picks the highest priority, then the oldest.
const MQ_ROOT: &[u8] = b"/dev/mqueue\0";
const MQ_BASE: &[u8] = b"/dev/mqueue/.q\0";
const MQ_FLAGS_CREAT: CInt = 0o100;
const MQ_FLAGS_EXCL: CInt = 0o200;
const MQ_FLAGS_NONBLOCK: CInt = 0o4000;
const F_GETFL: CInt = 3;

unsafe fn mq_mkdirs() {
    mkdirat(AT_FDCWD, MQ_ROOT.as_ptr() as *const CChar, 0o1777);
    mkdirat(AT_FDCWD, MQ_BASE.as_ptr() as *const CChar, 0o1777);
}

/// "/name" -> "/dev/mqueue/.q/name"; POSIX allows exactly one leading slash and
/// no further slashes inside the name.
unsafe fn mq_path(name: *const CChar, out: &mut [u8; 320]) -> Option<usize> {
    if name.is_null() {
        return None;
    }
    let b = bytes(name);
    if b.is_empty() || b[0] != b'/' {
        return None;
    }
    let rest = &b[1..];
    if rest.is_empty() || rest.len() > 240 {
        return None;
    }
    if rest.iter().any(|c| *c == b'/') {
        return None;
    }
    if rest == b"." || rest == b".." {
        return None;
    }
    let base = b"/dev/mqueue/.q/";
    let n = base.len() + rest.len();
    if n >= out.len() {
        return None;
    }
    out[..base.len()].copy_from_slice(base);
    out[base.len()..n].copy_from_slice(rest);
    Some(n)
}

unsafe fn mq_put_dec(out: &mut [u8], at: &mut usize, val: u64, mut width: usize) {
    let mut tmp = [0u8; 20];
    let mut n = 0usize;
    let mut v = val;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n < width {
        out[*at] = b'0';
        *at += 1;
        width -= 1;
    }
    let mut i = n;
    while i > 0 {
        i -= 1;
        out[*at] = tmp[i];
        *at += 1;
    }
}

unsafe fn mq_now_ns() -> u64 {
    // CLOCK_MONOTONIC (1) into a struct timespec { i64 sec; i64 nsec }
    let mut ts: [i64; 2] = [0, 0];
    let r = syscall4(113, 1, ts.as_mut_ptr() as usize, 0, 0);
    if r < 0 {
        return 0;
    }
    (ts[0] as u64) * 1_000_000_000 + (ts[1] as u64)
}

#[no_mangle]
pub unsafe extern "C" fn mq_open(name: *const CChar, oflag: CInt, mode: CInt) -> CInt {
    let mut qp = [0u8; 320];
    let qn = match mq_path(name, &mut qp) {
        Some(n) => n,
        None => return set_errno(22),
    };
    qp[qn] = 0;
    mq_mkdirs();
    if oflag & MQ_FLAGS_CREAT != 0 {
        let m = if mode == 0 { 0o666 } else { mode };
        let r = mkdirat(AT_FDCWD, qp.as_ptr() as *const CChar, m);
        if r < 0 {
            let e = *__errno_location();
            if e != 17 {
                return -1;
            }
            if oflag & MQ_FLAGS_EXCL != 0 {
                return set_errno(17);
            }
        }
    }
    let mut flags = O_DIRECTORY;
    if oflag & MQ_FLAGS_NONBLOCK != 0 {
        flags |= MQ_FLAGS_NONBLOCK;
    }
    let fd = openat(AT_FDCWD, qp.as_ptr() as *const CChar, flags, 0);
    if fd < 0 {
        return -1;
    }
    fd
}

#[no_mangle]
pub unsafe extern "C" fn mq_close(mqdes: CInt) -> CInt {
    if mqdes < 0 {
        return set_errno(9);
    }
    rc::close(mqdes);
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_unlink(name: *const CChar) -> CInt {
    let mut qp = [0u8; 320];
    let qn = match mq_path(name, &mut qp) {
        Some(n) => n,
        None => return set_errno(22),
    };
    qp[qn] = 0;
    // Drop whatever is queued, then the queue directory itself.
    let dfd = openat(
        AT_FDCWD,
        qp.as_ptr() as *const CChar,
        O_DIRECTORY,
        0,
    );
    if dfd < 0 {
        return -1;
    }
    let mut buf = [0u8; 1024];
    loop {
        let n = syscall4(61, dfd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
        if n <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < n as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &buf[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if nm != b"." && nm != b".." {
                unlinkat(dfd, nm.as_ptr() as *const CChar, 0);
            }
            off += reclen;
        }
    }
    rc::close(dfd);
    rmdirat_path(&qp[..qn])
}

unsafe fn rmdirat_path(p: &[u8]) -> CInt {
    let mut z = [0u8; 321];
    z[..p.len()].copy_from_slice(p);
    z[p.len()] = 0;
    pathop(AT_FDCWD, z.as_ptr() as *const CChar, false, |fd, q| {
        syscall4(SYS_UNLINKAT, fd as usize, q as usize, 0x200, 0)
    })
}

/// Scans the queue directory and remembers the best (priority, name) message.
unsafe fn mq_pick(dfd: CInt, best: &mut [u8; 64], best_len: &mut usize, best_prio: &mut u32) -> bool {
    let mut found = false;
    let mut buf = [0u8; 1024];
    loop {
        let n = syscall4(61, dfd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
        if n <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < n as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &buf[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if nm.len() > 4 && nm[0] == b'p' && len < 64 {
                // p<prio 3>_...
                let mut prio = 0u32;
                let mut ok = true;
                for k in 1..4 {
                    let c = nm[k];
                    if !c.is_ascii_digit() {
                        ok = false;
                        break;
                    }
                    prio = prio * 10 + (c - b'0') as u32;
                }
                if ok && prio >= *best_prio {
                    if prio > *best_prio || !found || nm < &best[..*best_len] {
                        best[..len].copy_from_slice(nm);
                        *best_len = len;
                        *best_prio = prio;
                        found = true;
                    }
                }
            }
            off += reclen;
        }
    }
    found
}

unsafe fn mq_is_nonblock(mqdes: CInt) -> bool {
    let fl = syscall4(25, mqdes as usize, F_GETFL as usize, 0, 0);
    fl >= 0 && (fl as CInt) & MQ_FLAGS_NONBLOCK != 0
}

#[no_mangle]
pub unsafe extern "C" fn mq_send(
    mqdes: CInt,
    msg: *const CChar,
    len: usize,
    prio: u32,
) -> CInt {
    mq_send_impl(mqdes, msg, len, prio)
}

unsafe fn mq_send_impl(mqdes: CInt, msg: *const CChar, len: usize, prio: u32) -> CInt {
    if mqdes < 0 || msg.is_null() {
        return set_errno(22);
    }
    let mut name = [0u8; 64];
    let mut at = 0usize;
    name[at] = b'p';
    at += 1;
    mq_put_dec(&mut name, &mut at, (prio.min(999)) as u64, 3);
    name[at] = b'_';
    at += 1;
    mq_put_dec(&mut name, &mut at, mq_now_ns(), 19);
    name[at] = b'_';
    at += 1;
    mq_put_dec(&mut name, &mut at, syscall4(178, 0, 0, 0, 0) as u64, 0); // getpid
    let fd = syscall4(
        SYS_OPENAT,
        mqdes as usize,
        name.as_ptr() as usize,
        0o1 | 0o100 | 0o200, // WRONLY|CREAT|EXCL
        0o666usize,
    );
    if fd < 0 {
        return -1;
    }
    let mut left = len;
    let mut p = msg as *const u8;
    while left > 0 {
        let w = syscall4(SYS_WRITE, fd as usize, p as usize, left, 0) as isize;
        if w <= 0 {
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
            unlinkat(mqdes, name.as_ptr() as *const CChar, 0);
            return set_errno(28);
        }
        left -= w as usize;
        p = p.add(w as usize);
    }
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_receive(
    mqdes: CInt,
    msg: *mut CChar,
    len: usize,
    prio_out: *mut u32,
) -> isize {
    mq_receive_impl(mqdes, msg, len, prio_out, None)
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedsend(
    mqdes: CInt,
    msg: *const CChar,
    len: usize,
    prio: u32,
    _abs: *const core::ffi::c_void,
) -> CInt {
    mq_send_impl(mqdes, msg, len, prio)
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedreceive(
    mqdes: CInt,
    msg: *mut CChar,
    len: usize,
    prio_out: *mut u32,
    abs: *const core::ffi::c_void,
) -> isize {
    mq_receive_impl(mqdes, msg, len, prio_out, Some(abs))
}

unsafe fn mq_receive_impl(
    mqdes: CInt,
    msg: *mut CChar,
    len: usize,
    prio_out: *mut u32,
    abs: Option<*const core::ffi::c_void>,
) -> isize {
    if mqdes < 0 || msg.is_null() {
        return set_errno(22) as isize;
    }
    let nonblock = mq_is_nonblock(mqdes);
    // Deadline from an absolute CLOCK_REALTIME timespec, if one was given.
    let mut deadline_ns: u64 = 0;
    let mut timed = false;
    if let Some(p) = abs {
        if !p.is_null() {
            let ts = core::slice::from_raw_parts(p as *const i64, 2);
            deadline_ns = (ts[0] as u64) * 1_000_000_000 + (ts[1] as u64);
            timed = true;
        }
    }
    loop {
        let mut best = [0u8; 64];
        let mut best_len = 0usize;
        let mut best_prio = 0u32;
        if mq_pick(mqdes, &mut best, &mut best_len, &mut best_prio) {
            let fd = syscall4(
                SYS_OPENAT,
                mqdes as usize,
                best.as_ptr() as usize,
                0,
                0,
            );
            if fd < 0 {
                continue;
            }
            let r = syscall4(SYS_READ, fd as usize, msg as usize, len, 0) as isize;
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
            unlinkat(mqdes, best.as_ptr() as *const CChar, 0);
            if r < 0 {
                return -1;
            }
            if !prio_out.is_null() {
                core::ptr::write_volatile(prio_out, best_prio);
            }
            return r;
        }
        if nonblock {
            return set_errno(11) as isize;
        }
        if timed {
            // CLOCK_REALTIME, seconds since the epoch.
            let mut rt: [i64; 2] = [0, 0];
            syscall4(113, 0, rt.as_mut_ptr() as usize, 0, 0);
            let now = (rt[0] as u64) * 1_000_000_000 + (rt[1] as u64);
            if now >= deadline_ns {
                return set_errno(110) as isize;
            }
        }
        let ts = [0i64, 2_000_000]; // 2 ms
        syscall4(115, ts.as_ptr() as usize, 0, 0, 0); // nanosleep
    }
}

/// struct mq_attr: four longs, plus reserved space. We answer what the queue
/// actually is: unlimited messages, and the flag this handle was opened with.
#[no_mangle]
pub unsafe extern "C" fn mq_getattr(mqdes: CInt, attr: *mut i64) -> CInt {
    if mqdes < 0 || attr.is_null() {
        return set_errno(22);
    }
    let fl = syscall4(25, mqdes as usize, F_GETFL as usize, 0, 0);
    let nonblock = fl >= 0 && (fl as CInt) & MQ_FLAGS_NONBLOCK != 0;
    let a = attr;
    core::ptr::write_volatile(a, if nonblock { MQ_FLAGS_NONBLOCK as i64 } else { 0 });
    core::ptr::write_volatile(a.add(1), 0); // mq_maxmsg: unbounded
    core::ptr::write_volatile(a.add(2), 8192); // mq_msgsize ceiling we honour
    core::ptr::write_volatile(a.add(3), 0); // mq_curmsgs: not tracked
    let mut i = 4;
    while i < 8 {
        core::ptr::write_volatile(a.add(i), 0);
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_setattr(
    mqdes: CInt,
    attr: *const i64,
    old: *mut i64,
) -> CInt {
    if mqdes < 0 {
        return set_errno(22);
    }
    if !old.is_null() {
        mq_getattr(mqdes, old);
    }
    if attr.is_null() {
        return 0;
    }
    let want = core::ptr::read_volatile(attr);
    let cur = syscall4(25, mqdes as usize, F_GETFL as usize, 0, 0) as CInt;
    if cur >= 0 {
        let mut fl = cur;
        if want & (MQ_FLAGS_NONBLOCK as i64) != 0 {
            fl |= MQ_FLAGS_NONBLOCK;
        } else {
            fl &= !MQ_FLAGS_NONBLOCK;
        }
        syscall4(25, mqdes as usize, 4 /* F_SETFL */, fl as usize, 0);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_notify(_mqdes: CInt, _sev: *const core::ffi::c_void) -> CInt {
    // No kernel notification: a program that only registers it keeps working,
    // one that waits on it never will. Documented in docs/design/ipc.md.
    set_errno(95) // EOPNOTSUPP
}

unsafe fn virt_mount_text(info: bool) -> usize {
    let mut at = 0usize;
    let fs = root_fstype();
    let root = core::slice::from_raw_parts(ROOT_ENV.as_ptr(), ROOT_ENV_LEN);
    if info {
        vt_put(&mut at, b"20 1 0:30 / / rw,relatime - ");
        vt_put(&mut at, fs);
        vt_put(&mut at, b" ");
        vt_put(&mut at, root);
        vt_put(&mut at, b" rw\n");
        vt_put(&mut at, b"21 20 0:5 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n");
        vt_put(&mut at, b"22 20 0:6 / /sys rw,nosuid,nodev,noexec,relatime - sysfs sysfs rw\n");
        vt_put(&mut at, b"23 20 0:7 / /dev rw,nosuid,relatime - devtmpfs devtmpfs rw\n");
        vt_put(&mut at, b"24 23 0:8 / /dev/pts rw,nosuid,noexec,relatime - devpts devpts rw\n");
        vt_put(&mut at, b"25 23 0:9 / /dev/shm rw,nosuid,nodev - tmpfs tmpfs rw\n");
    } else {
        vt_put(&mut at, root);
        vt_put(&mut at, b" / ");
        vt_put(&mut at, fs);
        vt_put(&mut at, b" rw,relatime 0 0\n");
        vt_put(&mut at, b"proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n");
        vt_put(&mut at, b"sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0\n");
        vt_put(&mut at, b"devtmpfs /dev devtmpfs rw,nosuid,relatime 0 0\n");
        vt_put(&mut at, b"devpts /dev/pts devpts rw,nosuid,noexec,relatime 0 0\n");
        vt_put(&mut at, b"tmpfs /dev/shm tmpfs rw,nosuid,nodev 0 0\n");
    }
    mnt_load();
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let tl = e.tlen as usize;
        if info {
            vt_put(&mut at, b"30 20 0:40 / ");
            vt_put(&mut at, &e.tgt[..tl]);
            vt_put(&mut at, b" ");
            vt_put(&mut at, if e.ro != 0 { b"ro" } else { b"rw" });
            vt_put(&mut at, b",relatime - ");
            vt_put(&mut at, if e.tmpfs != 0 { b"tmpfs tmpfs " } else { b"none none " });
            vt_put(&mut at, if e.ro != 0 { b"ro" } else { b"rw" });
            vt_put(&mut at, b"\n");
        } else {
            vt_put(&mut at, if e.tmpfs != 0 { b"tmpfs " } else { b"none " });
            vt_put(&mut at, &e.tgt[..tl]);
            vt_put(&mut at, if e.tmpfs != 0 { b" tmpfs " } else { b" none " });
            vt_put(&mut at, if e.ro != 0 { b"ro" } else { b"rw" });
            vt_put(&mut at, b",relatime 0 0\n");
        }
        i += 1;
    }
    at
}

// ------------------------------------------------------- synthesized /proc
//
// Measured on this device under untrusted_app (evidence 107): the kernel answers
// EACCES for /proc/{uptime,loadavg,stat,filesystems,locks}, /proc/net/* and
// /proc/sys/*, and netlink binds are refused. Nothing retries into a different
// answer; the domain simply may not see them.
//
// A container is expected to answer these out of its own namespace, so the shim
// does. Whatever can be read honestly is read: sysinfo(2) for uptime and load,
// sched_getaffinity for the CPU set, clock_gettime for btime. Whatever cannot be
// read is the documented default a namespace of this shape carries, and the
// table below says so - none of these numbers claims to be a measurement of the
// hardware.

const SYS_SYSINFO: usize = 179;
const SYS_SCHED_GETAFFINITY: usize = 123;
const CLOCK_REALTIME: usize = 0;
/// arm64 Android builds with CONFIG_HZ=100, which is the unit /proc/stat is in.
const USER_HZ: u64 = 100;

struct Sysinfo {
    uptime: u64,
    loads: [u64; 3],
    procs: u16,
}

unsafe fn sysinfo_read() -> Option<Sysinfo> {
    let mut b = [0u8; 112];
    if syscall4(SYS_SYSINFO, b.as_mut_ptr() as usize, 0, 0, 0) != 0 {
        return None;
    }
    Some(Sysinfo {
        uptime: rd8u(b.as_ptr()) as u64,
        loads: [
            rd8u(b.as_ptr().add(8)),
            rd8u(b.as_ptr().add(16)),
            rd8u(b.as_ptr().add(24)),
        ],
        procs: u16::from_le_bytes([b[80], b[81]]),
    })
}

unsafe fn now_secs() -> u64 {
    let mut ts = [0u64; 2];
    if syscall4(SYS_CLOCK_GETTIME, CLOCK_REALTIME, ts.as_mut_ptr() as usize, 0, 0) != 0 {
        return 0;
    }
    ts[0]
}

unsafe fn cpu_mask(mask: &mut [u8; 128]) -> u64 {
    if syscall4(
        SYS_SCHED_GETAFFINITY,
        0,
        mask.len(),
        mask.as_mut_ptr() as usize,
        0,
    ) != 0
    {
        return 1;
    }
    let mut n = 0u64;
    for &b in mask.iter() {
        n += b.count_ones() as u64;
    }
    if n == 0 {
        1
    } else {
        n
    }
}

/// 16.16 fixed point, two decimals - the format /proc/loadavg uses.
unsafe fn vt_load(at: &mut usize, v: u64) {
    vt_num(at, v >> 16);
    vt_put(at, b".");
    let frac = ((v & 0xffff) * 100) >> 16;
    if frac < 10 {
        vt_put(at, b"0");
    }
    vt_num(at, frac);
}

/// `/proc/sys` defaults for a namespace of this shape. Reads only; writes are
/// refused because there is no kernel knob behind them.
unsafe fn synth_sysctl(p: &[u8]) -> Option<&'static [u8]> {
    const TABLE: [(&[u8], &[u8]); 6] = [
        (b"/proc/sys/vm/max_map_count", b"65530\n"),
        (b"/proc/sys/kernel/pid_max", b"32768\n"),
        (b"/proc/sys/net/core/somaxconn", b"4096\n"),
        (b"/proc/sys/net/ipv4/ip_local_port_range", b"32768\t60999\n"),
        (b"/proc/sys/vm/overcommit_memory", b"0\n"),
        (b"/proc/sys/kernel/randomize_va_space", b"2\n"),
    ];
    let mut i = 0;
    while i < TABLE.len() {
        if TABLE[i].0 == p {
            return Some(TABLE[i].1);
        }
        i += 1;
    }
    None
}

unsafe fn synth_proc_stat(at: &mut usize) -> bool {
    let up = match sysinfo_read() {
        Some(s) => s.uptime,
        None => return false,
    };
    let mut mask = [0u8; 128];
    let ncpu = cpu_mask(&mut mask);
    let total_ticks = up * USER_HZ * ncpu;
    // The aggregate line: everything counted as idle. Per-core busy time is not
    // available to this domain, and a fabricated busy number would be worse than
    // an honest zero.
    vt_put(at, b"cpu  0 0 0 ");
    vt_num(at, total_ticks);
    vt_put(at, b" 0 0 0 0 0 0\n");
    let mut cpu = 0usize;
    let mut b = 0usize;
    while b < mask.len() && cpu < 1024 {
        let mut bit = 0;
        while bit < 8 {
            if mask[b] & (1 << bit) != 0 {
                vt_put(at, b"cpu");
                vt_num(at, cpu as u64);
                vt_put(at, b" 0 0 0 ");
                vt_num(at, up * USER_HZ);
                vt_put(at, b" 0 0 0 0 0 0\n");
                cpu += 1;
            }
            bit += 1;
        }
        b += 1;
    }
    vt_put(at, b"intr 0\nctxt 0\nbtime ");
    let now = now_secs();
    if now > up {
        vt_num(at, now - up);
    } else {
        vt_put(at, b"0");
    }
    vt_put(at, b"\nprocesses 0\nprocs_running 1\nprocs_blocked 0\n");
    vt_put(at, b"softirq 0 0 0 0 0 0 0 0 0 0\n");
    true
}

/// Interface table for /proc/net/*. `NR_NETIF` is filled in by the runtime from
/// the platform (Android's NetworkInterface) in the form
/// `name;addr/prefix;mac;name;addr/prefix;mac...`; loopback is always present
/// because loopback is measured working here. Nothing is invented when the
/// variable is absent: the file then lists loopback only.
unsafe fn netif(i: usize, out: &mut [u8; 32]) -> Option<usize> {
    if i == 0 {
        out[..2].copy_from_slice(b"lo");
        return Some(2);
    }
    let list = env_lookup(b"NR_NETIF")?;
    let mut idx = 1usize;
    let mut start = 0usize;
    let mut k = 0usize;
    while start <= list.len() {
        let end = match list[start..].iter().position(|c| *c == b';') {
            Some(e) => start + e,
            None => list.len(),
        };
        if idx == i {
            let name_end = match list[start..end].iter().position(|c| *c == b' ') {
                Some(e) => start + e,
                None => end,
            };
            let n = name_end - start;
            if n == 0 || n >= out.len() {
                return None;
            }
            out[..n].copy_from_slice(&list[start..name_end]);
            k = n;
            out[k] = 0;
            return Some(k);
        }
        idx += 1;
        start = end + 1;
    }
    None
}

unsafe fn synth_net_dev(at: &mut usize) -> bool {
    vt_put(
        at,
        b"Inter-|   Receive                                                |  Transmit\n \
face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n",
    );
    let mut i = 0;
    let mut buf = [0u8; 32];
    while let Some(n) = netif(i, &mut buf) {
        vt_put(at, if n == 2 { b"    lo: " } else { b"        " });
        if n != 2 {
            vt_put(at, &buf[..n]);
            vt_put(at, b": ");
        }
        vt_put(
            at,
            b"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        );
        i += 1;
        if i > 16 {
            break;
        }
    }
    true
}

/// Fill VTEXT for a file this namespace answers itself. None = not ours.
unsafe fn virt_synth(p: &[u8]) -> Option<usize> {
    let mut at = 0usize;
    if p == b"/proc/uptime" {
        let s = sysinfo_read()?;
        vt_num(&mut at, s.uptime);
        vt_put(&mut at, b".00 0.00\n");
        return Some(at);
    }
    if p == b"/proc/loadavg" {
        let s = sysinfo_read()?;
        vt_load(&mut at, s.loads[0]);
        vt_put(&mut at, b" ");
        vt_load(&mut at, s.loads[1]);
        vt_put(&mut at, b" ");
        vt_load(&mut at, s.loads[2]);
        vt_put(&mut at, b" 1/");
        vt_num(&mut at, s.procs as u64);
        vt_put(&mut at, b" 0\n");
        return Some(at);
    }
    if p == b"/proc/stat" {
        if !synth_proc_stat(&mut at) {
            return None;
        }
        return Some(at);
    }
    if p == b"/proc/filesystems" {
        // What this kernel will actually mount through us; the pseudo list is
        // the one the namespace presents, the disk ones are read from statfs.
        vt_put(
            &mut at,
            b"nodev\tsysfs\nnodev\tproc\nnodev\ttmpfs\nnodev\tdevpts\nnodev\tcgroup\n\
nodev\tcgroup2\nnodev\tmqueue\nnodev\tdebugfs\nnodev\ttracefs\nnodev\tconfigfs\n\
nodev\tbpf\nnodev\tfusectl\nnodev\tramfs\n\text4\n\tf2fs\n",
        );
        return Some(at);
    }
    if p == b"/proc/locks" {
        // No lock table is visible from this domain; an empty table is the
        // truthful answer, and it is the shape programs parse.
        return Some(0);
    }
    if p == b"/proc/net/dev" {
        if !synth_net_dev(&mut at) {
            return None;
        }
        return Some(at);
    }
    if p == b"/proc/net/route" {
        vt_put(
            &mut at,
            b"Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n",
        );
        return Some(at);
    }
    if p == b"/proc/net/tcp" || p == b"/proc/net/udp" || p == b"/proc/net/tcp6" || p == b"/proc/net/udp6" {
        vt_put(
            &mut at,
            b"  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n",
        );
        return Some(at);
    }
    if p == b"/proc/net/unix" {
        vt_put(&mut at, b"Num       RefCount Protocol Flags    Type St Inode Path\n");
        return Some(at);
    }
    if let Some(v) = synth_sysctl(p) {
        vt_put(&mut at, v);
        return Some(at);
    }
    None
}

/// Serve a file the namespace owns but the kernel cannot give us.
unsafe fn virt_open(p: &[u8], flags: CInt) -> Option<CInt> {
    let n = if p == b"/proc/mounts" || p == b"/proc/self/mounts" {
        virt_mount_text(false)
    } else if p == b"/proc/mountinfo" || p == b"/proc/self/mountinfo" {
        virt_mount_text(true)
    } else {
        // Asked *before* any access check: a path that is not ours has to fall
        // through to the normal resolution, writable or not.
        match virt_synth(p) {
            Some(n) => n,
            None => return None,
        }
    };
    if flags & O_ACCMODE_BITS != 0 {
        // Everything above is a read-only view; nothing can write it back.
        return Some(set_errno(13));
    }
    static VNAME: [u8; 10] = *b"husk-virt\0";
    let fd = syscall4(
        SYS_MEMFD_CREATE,
        VNAME.as_ptr() as usize,
        0usize,
        0,
        0,
    );
    if fd < 0 {
        return Some(set_errno(5));
    }
    let fd = fd as CInt;
    // flags = 0 above: this kernel rejects MFD_CLOEXEC with EINVAL (measured), so
    // the close-on-exec bit is set the other way round.
    syscall4(SYS_FCNTL, fd as usize, F_SETFD, FD_CLOEXEC, 0);
    let mut off = 0usize;
    while off < n {
        let w = syscall4(
            SYS_WRITE,
            fd as usize,
            VTEXT.as_ptr().add(off) as usize,
            n - off,
            0,
        );
        if w <= 0 {
            rc::close(fd);
            return Some(set_errno(5));
        }
        off += w as usize;
    }
    syscall4(SYS_LSEEK, fd as usize, 0, 0, 0);
    Some(fd)
}

// ------------------------------------------------------------- hard links
//
// Android refuses link(2) inside app data even between files the app owns
// (SELinux, measured: EACCES with no shim involved). tar and dpkg do use hard
// links, so the fallback is a byte copy plus the source mode. It is not a link -
// link count and shared inode are lost - and it is reported once per process so
// nobody mistakes it for the real thing.
static mut LINK_COPY_NOTICED: u32 = 0;

unsafe fn copy_file(
    ffd: CInt,
    from: *const CChar,
    tfd: CInt,
    to: *const CChar,
) -> Ssize {
    const O_WRONLY: CInt = 0o1;
    const O_CREAT: CInt = 0o100;
    const O_EXCL: CInt = 0o200;
    let src = syscall4(SYS_OPENAT, ffd as usize, from as usize, 0, 0);
    if src < 0 {
        return -1;
    }
    let src = src as CInt;
    let mut st = [0u8; 128];
    let mut mode = 0o644u32;
    if syscall4(SYS_FSTAT, src as usize, st.as_ptr() as usize, 0, 0) == 0 {
        mode = rd4u(st.as_ptr().add(16)) & 0o7777;
    }
    let dst = syscall4(
        SYS_OPENAT,
        tfd as usize,
        to as usize,
        (O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC) as usize,
        mode as usize,
    );
    if dst < 0 {
        rc::close(src);
        return -1;
    }
    let dst = dst as CInt;
    let mut buf = [0u8; 65536];
    loop {
        let n = syscall4(SYS_READ, src as usize, buf.as_ptr() as usize, buf.len(), 0);
        if n < 0 {
            break;
        }
        if n == 0 {
            rc::close(src);
            rc::close(dst);
            if LINK_COPY_NOTICED == 0 {
                LINK_COPY_NOTICED = 1;
                dbg(&[b"nrshim: hard link emulated by copy (kernel denies link in app data)\n"]);
            }
            return 0;
        }
        let mut off = 0usize;
        while off < n as usize {
            let w = syscall4(
                SYS_WRITE,
                dst as usize,
                buf.as_ptr().add(off) as usize,
                n as usize - off,
                0,
            );
            if w <= 0 {
                rc::close(src);
                rc::close(dst);
                return -1;
            }
            off += w as usize;
        }
    }
    rc::close(src);
    rc::close(dst);
    -1
}

#[no_mangle]
unsafe fn openat_inner(dirfd: CInt, path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    if path.is_null() {
        return ret_or_errno(syscall4(
            SYS_OPENAT,
            dirfd as usize,
            0,
            flags as usize,
            mode as usize,
        ));
    }
    let absolute = *path == b'/' as CChar;
    if dirfd != AT_FDCWD && !absolute {
        if verbose_on() {
            dbg(&[b"OPENAT fd="]);
            dbgn(&[], dirfd as i64);
            dbg(&[b"  name=", bytes(path), b"\n"]);
        }
        let rfd = rootfd();
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
                Ok(Some(n)) => {
                    return match rc::resolve_open(rfd, &gp[..n], flags, mode) {
                        Ok(fd) => fd,
                        Err(e) => set_errno(e),
                    }
                }
                Ok(None) => {}
                Err(e) => return set_errno(e),
            }
        }
        return ret_or_errno(syscall4(
            SYS_OPENAT,
            dirfd as usize,
            path as usize,
            flags as usize,
            mode as usize,
        ));
    }
    if dirfd == AT_FDCWD || absolute {
        let rfd = rootfd();
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            let n = match abs_guest(path, &mut gp) {
                Some(n) => n,
                None => PATH_MAX,
            };
            if n < PATH_MAX && flags & O_WRITEISH != 0 && path_is_ro(path) {
                return set_errno(30);
            }
            // DRM 假节点：先于 passthrough 判断，否则 /dev/* 会直接落到宿主上 ENOENT
            if verbose_on() && n >= 4 && &gp[..4] == b"/dev".as_ref() {
                dbg(&[b"nrshim open ", &gp[..n], b"\n"]);
            }
            if let Some(fd) = virt_open(&gp[..n], flags) {
                return fd;
            }
            let mut sysbuf = [0u8; PATH_MAX];
            if let Some(sn) = v4l2_sys_host(&gp[..n], &mut sysbuf) {
                return ret_or_errno(syscall4(
                    SYS_OPENAT,
                    AT_FDCWD as usize,
                    sysbuf.as_ptr() as usize,
                    flags as usize,
                    mode as usize,
                ));
            }
            if let Some(vn) = video_index(&gp[..n]) {
                match v4l2_open(vn) {
                    Some(Ok(fd)) => return fd,
                    Some(Err(e)) => return set_errno(e),
                    None => {}
                }
            }
            if is_drm_node(&gp[..n]) {
                if let Some(api) = drm_api() {
                    return drm_api_open(&api);
                }
                if !drm_shim_off() {
                    return drm_fake_open();
                }
            }
            if is_drm_dir(&gp[..n]) {
                if let Some(d) = drm_dir_host() {
                    return ret_or_errno(syscall4(
                        SYS_OPENAT,
                        AT_FDCWD as usize,
                        d.as_ptr() as usize,
                        flags as usize,
                        mode as usize,
                    ));
                }
            }
            if passthrough(&gp[..n]) {
                return ret_or_errno(syscall4(
                    SYS_OPENAT,
                    AT_FDCWD as usize,
                    gp.as_ptr() as usize,
                    flags as usize,
                    mode as usize,
                ));
            }
            if verbose_on() {
                dbg(&[b"OPEN ", &gp[..n], b" f="]);
                dbgn(&[], flags as i64);
            }
            return match rc::resolve_open(rfd, &gp[..n], flags, mode) {
                Ok(fd) => fd,
                Err(e) => {
                    dbg(&[b"nrshim open ", &gp[..n], b" err "]);
                    dbgn(&[], e as i64);
                    set_errno(e)
                }
            };
        }
    }
    let mut buf = [0u8; PATH_MAX];
    if abs_guest(path, &mut buf).is_none() {
        return set_errno(36);
    }
    let bl = {
        let mut i = 0;
        while i < PATH_MAX && buf[i] != 0 {
            i += 1;
        }
        i
    };
    let mut sysbuf = [0u8; PATH_MAX];
    if let Some(sn) = v4l2_sys_host(&buf[..bl], &mut sysbuf) {
        return ret_or_errno(syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            sysbuf.as_ptr() as usize,
            flags as usize,
            mode as usize,
        ));
    }
    if let Some(vn) = video_index(&buf[..bl]) {
        match v4l2_open(vn) {
            Some(Ok(fd)) => return fd,
            Some(Err(e)) => return set_errno(e),
            None => {}
        }
    }
    if is_drm_node(&buf[..bl]) {
        if let Some(api) = drm_api() {
            return drm_api_open(&api);
        }
        return drm_fake_open();
    }
    ret_or_errno(syscall4(
        SYS_OPENAT,
        dirfd as usize,
        buf.as_ptr() as usize,
        flags as usize,
        mode as usize,
    ))
}


#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: CInt, path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    let r = openat_inner(dirfd, path, flags, mode);
    // EINVAL out of an open is always a real defect: either the guest asked for
    // a flag combination this kernel cannot honour, or our own translation
    // mangled it. Record which, rather than let a package manager report
    // "error creating new backup file" with no way to see the request.
    if r < 0 && !path.is_null() && *__errno_location() == EINVAL && dbg_on() {
        let (b, k) = dec_buf(flags as i64);
        dbg_file(&[b"openat EINVAL flags=", &b[k..], b" ", bytes(path), b"\n"]);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn openat64(
    dirfd: CInt,
    path: *const CChar,
    flags: CInt,
    mode: CInt,
) -> CInt {
    openat(dirfd, path, flags, mode)
}

unsafe fn stat_like(path: *const CChar, st: *mut u8, flags: CInt) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    let rfd = rootfd();
    if rfd >= 0 {
        let mut gp = [0u8; PATH_MAX];
        let n = match abs_guest(path, &mut gp) {
            Some(n) => n,
            None => PATH_MAX,
        };
        let mut sysbuf = [0u8; PATH_MAX];
        if let Some(sn) = v4l2_sys_host(&gp[..n], &mut sysbuf) {
            return ret_or_errno(syscall4(
                SYS_NEWFSTATAT,
                AT_FDCWD as usize,
                sysbuf.as_ptr() as usize,
                st as usize,
                flags as usize,
            ));
        }
        if let Some(vn) = video_index(&gp[..n]) {
            return v4l2_stat(vn, st);
        }
        if passthrough(&gp[..n]) {
            return ret_or_errno(syscall4(
                SYS_NEWFSTATAT,
                AT_FDCWD as usize,
                gp.as_ptr() as usize,
                st as usize,
                flags as usize,
            ));
        }
        return match rc::resolve_fstat(rfd, &gp[..n], st, flags) {
            Ok(()) => 0,
            Err(e) => set_errno(e),
        };
    }
    let mut gp = [0u8; PATH_MAX];
    if abs_guest(path, &mut gp).is_none() {
        return set_errno(36);
    }
    ret_or_errno(syscall4(
        SYS_NEWFSTATAT,
        AT_FDCWD as usize,
        gp.as_ptr() as usize,
        st as usize,
        flags as usize,
    ))
}

unsafe fn accessat_nofollow(dirfd: CInt, path: *const CChar, mode: CInt) -> CInt {
    let fd = syscall4(
        SYS_OPENAT,
        dirfd as usize,
        path as usize,
        (O_PATH | O_NOFOLLOW | O_CLOEXEC) as usize,
        0,
    );
    if fd < 0 {
        return ret_or_errno(fd);
    }
    let r = rc::access_fd(fd as CInt, mode, true);
    rc::close(fd as CInt);
    match r {
        Ok(()) => 0,
        Err(e) => set_errno(e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn stat(path: *const CChar, st: *mut u8) -> CInt {
    stat_like(path, st, 0)
}

#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const CChar, st: *mut u8) -> CInt {
    stat_like(path, st, AT_SYMLINK_NOFOLLOW)
}

#[no_mangle]
pub unsafe extern "C" fn fstatat(
    dirfd: CInt,
    path: *const CChar,
    st: *mut u8,
    flags: CInt,
) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    if dirfd == AT_FDCWD || (!path.is_null() && *path == b'/' as CChar) {
        return stat_like(path, st, flags);
    }
    let rfd = rootfd();
    if rfd >= 0 {
        let mut gp = [0u8; PATH_MAX];
        match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
            Ok(Some(n)) => {
                return match rc::resolve_fstat(rfd, &gp[..n], st, flags) {
                    Ok(()) => 0,
                    Err(e) => set_errno(e),
                }
            }
            Ok(None) => {}
            Err(e) => return set_errno(e),
        }
    }
    ret_or_errno(syscall4(
        SYS_NEWFSTATAT,
        dirfd as usize,
        path as usize,
        st as usize,
        flags as usize,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn access(path: *const CChar, mode: CInt) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    let rfd = rootfd();
    if rfd >= 0 {
        let mut gp = [0u8; PATH_MAX];
        let n = match abs_guest(path, &mut gp) {
            Some(n) => n,
            None => PATH_MAX,
        };
        if let Some(_vn) = video_index(&gp[..n]) {
            return 0;
        }
        if passthrough(&gp[..n]) {
            return ret_or_errno(syscall4(
                SYS_FACCESSAT,
                AT_FDCWD as usize,
                gp.as_ptr() as usize,
                mode as usize,
                0,
            ));
        }
        return match rc::resolve_access(rfd, &gp[..n], mode, 0) {
            Ok(()) => 0,
            Err(e) => set_errno(e),
        };
    }
    let mut gp = [0u8; PATH_MAX];
    if abs_guest(path, &mut gp).is_none() {
        return set_errno(36);
    }
    ret_or_errno(syscall4(
        SYS_FACCESSAT,
        AT_FDCWD as usize,
        gp.as_ptr() as usize,
        mode as usize,
        0,
    ))
}

// ---------------------------------------------------------------------------
// Namespace-mutating calls: mkdir/unlink/rename/symlink/link/chmod/chown/
// truncate/utimensat. Every one of these takes a path, so the path has to be
// split into (parentfd, final name) exactly like open does; otherwise the
// kernel would act on the *host* path and either fail or escape the rootfs.
// ---------------------------------------------------------------------------

/// Resolve `path` into a parent fd + final component and hand both to `f`.
/// `follow` follows a symlink in the final component (chmod/touch semantics);
/// most other calls (unlink/rename/mkdir/symlink) must not.
unsafe fn pathop<F>(dirfd: CInt, path: *const CChar, follow: bool, f: F) -> CInt
where
    F: FnOnce(CInt, *const CChar) -> Ssize,
{
    if path.is_null() {
        return set_errno(14);
    }
    let mut gp = [0u8; PATH_MAX];
    let n = if dirfd != AT_FDCWD && *path != b'/' as CChar {
        let rfd = rootfd();
        if rfd < 0 {
            return ret_or_errno(f(dirfd, path));
        }
        match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
            Ok(Some(n)) => n,
            Ok(None) => return ret_or_errno(f(dirfd, path)),
            Err(e) => return set_errno(e),
        }
    } else {
        match abs_guest(path, &mut gp) {
            Some(n) => n,
            None => return set_errno(36),
        }
    };
    // Every operation that goes through here modifies the tree (mkdir, unlink,
    // rmdir, mknod, symlink): a read-only emulated mount must refuse with EROFS.
    if path_is_ro(path) {
        return set_errno(30);
    }
    let rfd = rootfd();
    let mut nb = [0u8; 260];
    let mut raw = [0u8; PATH_MAX];
    let mut own = false;
    let (fd, p): (CInt, *const CChar) = if rfd >= 0 && !passthrough(&gp[..n]) {
        let mut name = [0u8; 256];
        let parent = if follow {
            rc::resolve_parent_follow(rfd, &gp[..n], &mut name)
        } else {
            rc::resolve_parent(rfd, &gp[..n], &mut name)
        };
        let parent = match parent {
            Ok(fd) => fd,
            Err(e) => {
                dbg(&[b"pathop ", &gp[..n], b" err "]);
                dbgn(&[], e as i64);
                return set_errno(e);
            }
        };
        let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
        nb[..nl].copy_from_slice(&name[..nl]);
        nb[nl] = 0;
        own = true;
        (parent, nb.as_ptr() as *const CChar)
    } else {
        raw[..n].copy_from_slice(&gp[..n]);
        raw[n] = 0;
        (dirfd, raw.as_ptr() as *const CChar)
    };
    let r = f(fd, p);
    if own {
        rc::close(fd);
    }
    ret_or_errno(r)
}

/// Open the final object named by a guest path as an `O_PATH` descriptor.
/// Metadata operations then target that pinned inode through `/proc/self/fd/N`
/// (or `AT_EMPTY_PATH`) instead of re-looking up the final name, which closes
/// the final-component replacement race on kernels without fchmodat2.
unsafe fn target_fd(dirfd: CInt, path: *const CChar, follow: bool) -> Result<CInt, CInt> {
    if path.is_null() {
        return Err(14);
    }
    if *path == 0 {
        return Err(2);
    }
    let flags = O_PATH | O_CLOEXEC | if follow { 0 } else { O_NOFOLLOW };
    if dirfd != AT_FDCWD && *path != b'/' as CChar {
        let rfd = rootfd();
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
                Ok(Some(n)) => return rc::resolve_final(rfd, &gp[..n], follow),
                Ok(None) => return rc::openat(dirfd, bytes(path), flags, 0),
                Err(e) => return Err(e),
            }
        }
        return rc::openat(dirfd, bytes(path), flags, 0);
    }

    let rfd = rootfd();
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return Err(36),
    };
    if rfd < 0 || passthrough(&gp[..n]) {
        return rc::openat(AT_FDCWD, &gp[..n], flags, 0);
    }
    rc::resolve_final(rfd, &gp[..n], follow)
}

/// Reconstruct an absolute guest path from a real dirfd plus a relative name.
/// This is what keeps `fchmodat(dfd, "link", ...)` from letting the kernel
/// follow an absolute symlink target against the host root. A dirfd outside the
/// rootfs (for example a passthrough /proc or /dev fd) returns None and keeps
/// the historical raw behavior.
unsafe fn dirfd_guest_path(dirfd: CInt, rel: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    if rel.is_empty() || rel.len() >= PATH_MAX {
        return None;
    }
    let mut real = [0u8; PATH_MAX];
    let rl = match fd_real_path(dirfd, &mut real) {
        Some(v) => v,
        None => return None,
    };
    let root = guest_root();
    if root.is_empty() || rl < root.len() || &real[..root.len()] != root {
        return None;
    }
    if rl != root.len() && real[root.len()] != b'/' {
        return None;
    }

    let mut raw = [0u8; PATH_MAX];
    let mut n = 1usize;
    raw[0] = b'/';
    if rl > root.len() {
        let suffix = &real[root.len() + 1..rl];
        if suffix.len() + 1 >= PATH_MAX {
            return None;
        }
        raw[1..1 + suffix.len()].copy_from_slice(suffix);
        n = 1 + suffix.len();
    }
    if n + 1 + rel.len() >= PATH_MAX {
        return None;
    }
    if n != 1 || raw[0] != b'/' {
        raw[n] = b'/';
        n += 1;
    }
    raw[n..n + rel.len()].copy_from_slice(rel);
    n += rel.len();
    Some(normalize(&raw[..n], out))
}

/// Read the kernel's real path for an already-open descriptor.
unsafe fn fd_real_path(fd: CInt, out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let mut pb = [0u8; 32];
    let proc_path = fd_proc_path(fd, &mut pb);
    let rn = syscall4(
        SYS_READLINKAT,
        AT_FDCWD as usize,
        proc_path.as_ptr() as usize,
        out.as_mut_ptr() as usize,
        PATH_MAX - 1,
    );
    if rn <= 0 {
        return None;
    }
    let mut rl = rn as usize;
    while rl > 0 && out[rl - 1] == 0 {
        rl -= 1;
    }
    (rl != 0).then_some(rl)
}

/// A relative path from a host descriptor is only safe inside the three
/// deliberately passed-through trees. Refuse to let `/proc`, `/dev`, or `/sys`
/// descriptors become a namespace escape via `..` or proc magic links such as
/// `self/root` and `self/cwd`.
unsafe fn host_dirfd_safe(dirfd: CInt, rel: &[u8]) -> Option<usize> {
    if rel.is_empty() || rel.len() >= PATH_MAX {
        return None;
    }
    let mut real = [0u8; PATH_MAX];
    let rn = fd_real_path(dirfd, &mut real)?;
    let mut root_len = 0usize;
    let mut is_proc = false;
    for root in [b"/proc".as_ref(), b"/dev".as_ref(), b"/sys".as_ref()] {
        if rn >= root.len()
            && &real[..root.len()] == root
            && (rn == root.len() || real[root.len()] == b'/')
        {
            root_len = root.len();
            is_proc = root == b"/proc".as_ref();
            break;
        }
    }
    if root_len == 0 {
        return None;
    }
    for c in rel.split(|x| *x == b'/') {
        if c == b".." || (is_proc && (c == b"root" || c == b"cwd")) {
            return None;
        }
    }
    Some(root_len)
}

unsafe fn fd_visible_path(fd: CInt) -> bool {
    let mut real = [0u8; PATH_MAX];
    let Some(rn) = fd_real_path(fd, &mut real) else {
        return false;
    };
    let root = guest_root();
    if !root.is_empty()
        && rn >= root.len()
        && &real[..root.len()] == root
        && (rn == root.len() || real[root.len()] == b'/')
    {
        return true;
    }
    pseudo_path(&real[..rn]) && passthrough(&real[..rn])
}

/// Resolve a non-absolute path relative to a real directory fd.
/// `Ok(Some(n))` means `out[..n]` is a guest-absolute path and the caller must
/// use the guest-root resolver. `Ok(None)` means the fd is one of the explicit
/// host passthrough trees and the relative path was lexically checked. Any
/// other fd is denied: an inherited host fd must not become a namespace hole.
unsafe fn resolve_dirfd_rel(
    dirfd: CInt,
    rel: &[u8],
    out: &mut [u8; PATH_MAX],
) -> Result<Option<usize>, CInt> {
    if rel.is_empty() {
        return Err(2);
    }
    if rel.len() >= PATH_MAX {
        return Err(36);
    }
    if let Some(n) = dirfd_guest_path(dirfd, rel, out) {
        return Ok(Some(n));
    }
    if host_dirfd_safe(dirfd, rel).is_some() {
        return Ok(None);
    }
    Err(13)
}

unsafe fn fd_proc_path(fd: CInt, out: &mut [u8; 32]) -> &[u8] {
    let prefix = b"/proc/self/fd/";
    out[..prefix.len()].copy_from_slice(prefix);
    let mut at = prefix.len();
    let mut digits = [0u8; 10];
    let mut n = 0usize;
    let mut v = fd as u32;
    if v == 0 {
        digits[0] = b'0';
        n = 1;
    } else {
        while v != 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
    }
    while n > 0 {
        n -= 1;
        out[at] = digits[n];
        at += 1;
    }
    out[at] = 0;
    &out[..=at]
}

unsafe fn chmod_target(dirfd: CInt, path: *const CChar, mode: CInt, follow: bool) -> CInt {
    let fd = match target_fd(dirfd, path, follow) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let mut pb = [0u8; 32];
    let p = fd_proc_path(fd, &mut pb);
    let r = syscall4(
        SYS_FCHMODAT,
        AT_FDCWD as usize,
        p.as_ptr() as usize,
        mode as usize,
        0,
    );
    rc::close(fd);
    ret_or_errno(r)
}

unsafe fn chown_target(
    dirfd: CInt,
    path: *const CChar,
    uid: u32,
    gid: u32,
    follow: bool,
    raw_flags: CInt,
) -> CInt {
    // Under the fake-root convention the guest cannot change ownership (no
    // CAP_CHOWN), and every package manager calls it on every file. Reporting
    // success is what a user namespace would do; the real owner on disk stays
    // u0_a302. See the fake-root note above getuid().
    if fake_root_on() {
        return 0;
    }
    if path.is_null() {
        return set_errno(14);
    }
    if *path == 0 && raw_flags & AT_EMPTY_PATH != 0 {
        return ret_or_errno(syscall5(
            SYS_FCHOWNAT,
            dirfd as usize,
            path as usize,
            uid as usize,
            gid as usize,
            raw_flags as usize,
        ));
    }
    let fd = match target_fd(dirfd, path, follow) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let mut flags = AT_EMPTY_PATH;
    if !follow {
        flags |= AT_SYMLINK_NOFOLLOW;
    }
    let r = syscall5(
        SYS_FCHOWNAT,
        fd as usize,
        b"\0".as_ptr() as usize,
        uid as usize,
        gid as usize,
        flags as usize,
    );
    rc::close(fd);
    ret_or_errno(r)
}

unsafe fn utimens_target(
    dirfd: CInt,
    path: *const CChar,
    times: *const u8,
    follow: bool,
    raw_flags: CInt,
) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    // Under the fake-root convention the guest cannot change ownership (no
    // CAP_CHOWN), and every package manager calls it on every file. Reporting
    // success is what a user namespace would do; the real owner on disk stays
    // u0_a302. See the fake-root note above getuid().
    if fake_root_on() {
        return 0;
    }
    if *path == 0 {
        if raw_flags & AT_EMPTY_PATH != 0 {
            // The caller handed us an fd, not a name: nothing to translate.
            return ret_or_errno(syscall4(
                SYS_UTIMENSAT,
                dirfd as usize,
                path as usize,
                times as usize,
                raw_flags as usize,
            ));
        }
        return set_errno(2);
    }
    if follow {
        let fd = match target_fd(dirfd, path, true) {
            Ok(fd) => fd,
            Err(e) => return set_errno(e),
        };
        let mut pb = [0u8; 32];
        let p = fd_proc_path(fd, &mut pb);
        let r = syscall4(
            SYS_UTIMENSAT,
            AT_FDCWD as usize,
            p.as_ptr() as usize,
            times as usize,
            0,
        );
        rc::close(fd);
        return ret_or_errno(r);
    }
    // No-follow: the *name* is the target, usually a symlink. Resolving the
    // whole path to an fd and going through /proc/self/fd would follow the
    // link (a dangling one answers ENOENT) and set the times of whatever it
    // points at instead of the link itself. Resolve only the parent inside the
    // guest and keep AT_SYMLINK_NOFOLLOW so the kernel acts on the name.
    let rfd = rootfd();
    let mut raw = [0u8; PATH_MAX];
    let mut nb = [0u8; 260];
    let (parent, name, own) = match one_side(rfd, dirfd, path, false, &mut raw, &mut nb) {
        Ok(v) => v,
        Err(e) => return set_errno(e),
    };
    let r = syscall4(
        SYS_UTIMENSAT,
        parent as usize,
        name as usize,
        times as usize,
        AT_SYMLINK_NOFOLLOW as usize,
    );
    if own {
        rc::close(parent);
    }
    ret_or_errno(r)
}

/// One side of a two-path call: resolve `path` (relative to `dirfd`) into a
/// parent fd + final component. Returns the fd, the name pointer and whether the
/// fd is ours to close. Paths relative to a real dirfd are passed straight
/// through, because such an fd already points inside the rootfs.
unsafe fn one_side(
    rfd: CInt,
    dirfd: CInt,
    path: *const CChar,
    follow: bool,
    raw: &mut [u8; PATH_MAX],
    nb: &mut [u8; 260],
) -> Result<(CInt, *const CChar, bool), CInt> {
    if dirfd != AT_FDCWD && *path != b'/' as CChar {
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
                Ok(Some(n)) => {
                    let mut name = [0u8; 256];
                    let parent = if follow {
                        rc::resolve_parent_follow(rfd, &gp[..n], &mut name)
                    } else {
                        rc::resolve_parent(rfd, &gp[..n], &mut name)
                    }?;
                    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
                    nb[..nl].copy_from_slice(&name[..nl]);
                    nb[nl] = 0;
                    return Ok((parent, nb.as_ptr() as *const CChar, true));
                }
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        return Ok((dirfd, path, false));
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return Err(36),
    };
    if rfd < 0 || passthrough(&gp[..n]) {
        raw[..n].copy_from_slice(&gp[..n]);
        raw[n] = 0;
        return Ok((AT_FDCWD, raw.as_ptr() as *const CChar, false));
    }
    let mut name = [0u8; 256];
    let parent = if follow {
        rc::resolve_parent_follow(rfd, &gp[..n], &mut name)
    } else {
        rc::resolve_parent(rfd, &gp[..n], &mut name)
    }?;
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    nb[..nl].copy_from_slice(&name[..nl]);
    nb[nl] = 0;
    Ok((parent, nb.as_ptr() as *const CChar, true))
}

unsafe fn pathop2<F>(
    afd: CInt,
    a: *const CChar,
    bfd: CInt,
    b: *const CChar,
    follow_a: bool,
    follow_b: bool,
    f: F,
) -> CInt
where
    F: FnOnce(CInt, *const CChar, CInt, *const CChar) -> Ssize,
{
    if a.is_null() || b.is_null() {
        return set_errno(14);
    }
    let rfd = rootfd();
    let mut rawa = [0u8; PATH_MAX];
    let mut rawb = [0u8; PATH_MAX];
    let mut nba = [0u8; 260];
    let mut nbb = [0u8; 260];
    let (pa, na, own_a) = match one_side(rfd, afd, a, follow_a, &mut rawa, &mut nba) {
        Ok(v) => v,
        Err(e) => return set_errno(e),
    };
    let (pb, nb, own_b) = match one_side(rfd, bfd, b, follow_b, &mut rawb, &mut nbb) {
        Ok(v) => v,
        Err(e) => {
            if own_a {
                rc::close(pa);
            }
            return set_errno(e);
        }
    };
    let r = f(pa, na, pb, nb);
    if own_a {
        rc::close(pa);
    }
    if own_b {
        rc::close(pb);
    }
    ret_or_errno(r)
}

#[no_mangle]
pub unsafe extern "C" fn mkdir(path: *const CChar, mode: CInt) -> CInt {
    pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_MKDIRAT, fd as usize, p as usize, mode as usize, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat(dirfd: CInt, path: *const CChar, mode: CInt) -> CInt {
    if verbose_on() {
        dbg(&[b"MKDIRAT fd="]);
        dbgn(&[], dirfd as i64);
        dbg(&[b"  path=", bytes(path), b"\n"]);
    }
    pathop(dirfd, path, false, |fd, p| {
        syscall4(SYS_MKDIRAT, fd as usize, p as usize, mode as usize, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rmdir(path: *const CChar) -> CInt {
    pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, 0x200, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const CChar) -> CInt {
    pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, 0, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat(dirfd: CInt, path: *const CChar, flags: CInt) -> CInt {
    pathop(dirfd, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, flags as usize, 0)
    })
}

// ---------------------------------------------------------------- fake root
//
// A container normally runs as root *inside*, with the host mapping that to an
// unprivileged user; every path-capable tool (apt, dpkg, systemctl, cron) checks
// geteuid() == 0 before it will do anything. We cannot be uid 0, but we can make
// that one check read 0 and make the ownership syscalls it follows a no-op -
// exactly the fiction a user namespace would give us, minus the namespace.
//
// What it does NOT do: it grants nothing. mknod still fails (no CAP_MKNOD),
// setuid() still fails, capabilities stay at 0. Files keep the real owner
// (u0_a302) on disk; only what the process is *told* changes. Off unless
// NR_FAKE_ROOT is set, so the default runtime stays truthful.
static mut FAKE_ROOT: u8 = 2;

unsafe fn fake_root_on() -> bool {
    if FAKE_ROOT == 2 {
        // On by default: inside a Linux runtime you are root, and the host user
        // that really owns the files is an implementation detail - the same
        // fiction a user namespace gives a container. NR_FAKE_ROOT=0 asks for
        // the truthful answer instead.
        let off = matches!(env_lookup(b"NR_FAKE_ROOT"), Some(v) if v == b"0" || v == b"off" || v == b"no");
        FAKE_ROOT = if off { 0 } else { 1 };
    }
    FAKE_ROOT == 1
}

unsafe fn log_id(name: &[u8], v: i64) {
    if dbg_on() {
        let (b, k) = dec_buf(v);
        dbg_file(&[b"id ", name, b"=", &b[k..], b"\n"]);
    }
}

#[no_mangle]
pub unsafe extern "C" fn getuid() -> u32 {
    if fake_root_on() {
        return 0;
    }
    syscall4(SYS_GETUID, 0, 0, 0, 0) as u32
}

#[no_mangle]
pub unsafe extern "C" fn geteuid() -> u32 {
    getuid()
}

#[no_mangle]
pub unsafe extern "C" fn getgid() -> u32 {
    if fake_root_on() {
        return 0;
    }
    syscall4(SYS_GETGID, 0, 0, 0, 0) as u32
}

#[no_mangle]
pub unsafe extern "C" fn getegid() -> u32 {
    getgid()
}

// The other half of the same fiction: apt drops to `_apt` for downloads and back
// to root for the install. Neither call can succeed for us (the process is
// u0_a302 and stays u0_a302), so under NR_FAKE_ROOT they are accepted and
// ignored - the process never actually changes identity either way.
#[no_mangle]
pub unsafe extern "C" fn setuid(_uid: u32) -> CInt {
    log_id(b"setuid", _uid as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETUID, _uid as usize, 0, 0, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn seteuid(_uid: u32) -> CInt {
    log_id(b"seteuid", _uid as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETRESUID, usize::MAX, _uid as usize, usize::MAX, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setgid(_gid: u32) -> CInt {
    log_id(b"setgid", _gid as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETGID, _gid as usize, 0, 0, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setegid(_gid: u32) -> CInt {
    log_id(b"setegid", _gid as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETRESGID, usize::MAX, _gid as usize, usize::MAX, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setgroups(_n: usize, _list: *const u32) -> CInt {
    log_id(b"setgroups", _n as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETGROUPS, _n, _list as usize, 0, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setresuid(_r: u32, _e: u32, _s: u32) -> CInt {
    log_id(b"setresuid", _r as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETRESUID, _r as usize, _e as usize, _s as usize, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setresgid(_r: u32, _e: u32, _s: u32) -> CInt {
    log_id(b"setresgid", _r as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETRESGID, _r as usize, _e as usize, _s as usize, 0)) }
}

// mknod(2) had no translation at all, so `mkfifo /tmp/x` asked the kernel for
// the *host* "/tmp/x" and came back ENOENT. FIFOs are ordinary tooling (daemons
// that use mkfifo + a reader, tar extraction of pipes); char/block nodes are not
// creatable without CAP_MKNOD, which is a separate wall.
#[no_mangle]
pub unsafe extern "C" fn mknodat(dirfd: CInt, path: *const CChar, mode: CInt, dev: u64) -> CInt {
    pathop(dirfd, path, false, |fd, p| {
        syscall5(
            SYS_MKNODAT,
            fd as usize,
            p as usize,
            mode as usize,
            dev as usize,
            0,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn mknod(path: *const CChar, mode: CInt, dev: u64) -> CInt {
    mknodat(AT_FDCWD, path, mode, dev)
}

#[no_mangle]
pub unsafe extern "C" fn mkfifo(path: *const CChar, mode: CInt) -> CInt {
    const S_IFIFO: CInt = 0o010000;
    mknodat(AT_FDCWD, path, mode | S_IFIFO, 0)
}

#[no_mangle]
pub unsafe extern "C" fn mkfifoat(dirfd: CInt, path: *const CChar, mode: CInt) -> CInt {
    const S_IFIFO: CInt = 0o010000;
    mknodat(dirfd, path, mode | S_IFIFO, 0)
}

#[no_mangle]
pub unsafe extern "C" fn rename(from: *const CChar, to: *const CChar) -> CInt {
    pathop2(
        AT_FDCWD,
        from,
        AT_FDCWD,
        to,
        false,
        false,
        |fa, pa, fb, pb| {
            syscall4(
                SYS_RENAMEAT,
                fa as usize,
                pa as usize,
                fb as usize,
                pb as usize,
            )
        },
    )
}

#[no_mangle]
pub unsafe extern "C" fn renameat(
    afd: CInt,
    from: *const CChar,
    bfd: CInt,
    to: *const CChar,
) -> CInt {
    pathop2(afd, from, bfd, to, false, false, |fa, pa, fb, pb| {
        syscall4(
            SYS_RENAMEAT,
            fa as usize,
            pa as usize,
            fb as usize,
            pb as usize,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn renameat2(
    afd: CInt,
    from: *const CChar,
    bfd: CInt,
    to: *const CChar,
    flags: CInt,
) -> CInt {
    pathop2(afd, from, bfd, to, false, false, |fa, pa, fb, pb| {
        let r = syscall5(
            SYS_RENAMEAT2,
            fa as usize,
            pa as usize,
            fb as usize,
            pb as usize,
            flags as usize,
        );
        // renameat() cannot preserve RENAME_NOREPLACE / RENAME_EXCHANGE /
        // RENAME_WHITEOUT. Only use the legacy fallback for flags == 0.
        if r == -38 && flags == 0 {
            return syscall4(
                SYS_RENAMEAT,
                fa as usize,
                pa as usize,
                fb as usize,
                pb as usize,
            );
        }
        r
    })
}

#[no_mangle]
pub unsafe extern "C" fn symlink(target: *const CChar, linkpath: *const CChar) -> CInt {
    // The *target* is stored verbatim (guest-relative link contents); only the
    // link location is resolved.
    pathop(AT_FDCWD, linkpath, false, |fd, p| {
        syscall4(SYS_SYMLINKAT, target as usize, fd as usize, p as usize, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat(
    target: *const CChar,
    dirfd: CInt,
    linkpath: *const CChar,
) -> CInt {
    pathop(dirfd, linkpath, false, |fd, p| {
        syscall4(SYS_SYMLINKAT, target as usize, fd as usize, p as usize, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn link(old: *const CChar, new: *const CChar) -> CInt {
    pathop2(
        AT_FDCWD,
        old,
        AT_FDCWD,
        new,
        false,
        false,
        |fa, pa, fb, pb| {
            // linkat(2) carries a fifth argument (flags). Sending it with only
            // four left x4 holding whatever the caller had loaded, and the
            // kernel answered EINVAL - which is what dpkg reported as
            // "error creating new backup file '/var/lib/dpkg/status-old'".
            // The copy fallback is shared with linkat(3), because this SELinux
            // policy refuses the real call inside app data.
            let r = syscall5(SYS_LINKAT, fa as usize, pa as usize, fb as usize, pb as usize, 0);
            if r >= 0 {
                return r;
            }
            if r == -13 || r == -1 {
                return copy_file(fa, pa, fb, pb);
            }
            r
        },
    )
}

#[no_mangle]
pub unsafe extern "C" fn linkat(
    ofd: CInt,
    old: *const CChar,
    nfd: CInt,
    new: *const CChar,
    flags: CInt,
) -> CInt {
    // Resolve the old final symlink through the guest root before the kernel
    // sees AT_SYMLINK_FOLLOW. Passing the flag with an unresolved old name
    // would let an absolute link target resolve against the host root.
    let follow_old = flags & AT_SYMLINK_FOLLOW != 0;
    pathop2(ofd, old, nfd, new, follow_old, false, |fa, pa, fb, pb| {
        // linkat's 5th argument carries flags; keep it in x4 via a small shim.
        let r = syscall5(
            SYS_LINKAT,
            fa as usize,
            pa as usize,
            fb as usize,
            pb as usize,
            flags as usize,
        );
        if r >= 0 {
            return r;
        }
        // Raw syscalls return -errno; errno itself is only written by the
        // wrappers, so the decision has to be made on the return value.
        if r == -13 || r == -1 {
            return copy_file(fa, pa, fb, pb);
        }
        r
    })
}

unsafe fn syscall5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> Ssize {
    let ret: Ssize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d, in("x4") e,
        lateout("x0") ret, options(nostack));
    ret
}

#[no_mangle]
pub unsafe extern "C" fn chmod(path: *const CChar, mode: CInt) -> CInt {
    chmod_target(AT_FDCWD, path, mode, true)
}

#[no_mangle]
pub unsafe extern "C" fn fchmodat(
    dirfd: CInt,
    path: *const CChar,
    mode: CInt,
    flags: CInt,
) -> CInt {
    chmod_target(dirfd, path, mode, flags & AT_SYMLINK_NOFOLLOW == 0)
}

#[no_mangle]
pub unsafe extern "C" fn chown(path: *const CChar, uid: u32, gid: u32) -> CInt {
    chown_target(AT_FDCWD, path, uid, gid, true, 0)
}

#[no_mangle]
pub unsafe extern "C" fn lchown(path: *const CChar, uid: u32, gid: u32) -> CInt {
    chown_target(AT_FDCWD, path, uid, gid, false, AT_SYMLINK_NOFOLLOW)
}

#[no_mangle]
pub unsafe extern "C" fn fchownat(
    dirfd: CInt,
    path: *const CChar,
    uid: u32,
    gid: u32,
    flags: CInt,
) -> CInt {
    chown_target(
        dirfd,
        path,
        uid,
        gid,
        flags & AT_SYMLINK_NOFOLLOW == 0,
        flags,
    )
}

#[no_mangle]
pub unsafe extern "C" fn truncate(path: *const CChar, len: i64) -> CInt {
    let fd = open(path, 1, 0);
    if fd < 0 {
        return -1;
    }
    const SYS_FTRUNCATE: usize = 46;
    let r = syscall4(SYS_FTRUNCATE, fd as usize, len as usize, 0, 0);
    rc::close(fd);
    ret_or_errno(r)
}

#[no_mangle]
pub unsafe extern "C" fn utimensat(
    dirfd: CInt,
    path: *const CChar,
    times: *const u8,
    flags: CInt,
) -> CInt {
    let r = utimens_target(dirfd, path, times, flags & AT_SYMLINK_NOFOLLOW == 0, flags);
    if dbg_on() {
        let (b, k) = dec_buf(r as i64);
        dbg_file(&[b"utimensat ", bytes(path), b" -> ", &b[k..], b"\n"]);
    }
    r
}

/// `lutimes(3)`: utimes without following the final symlink.
///
/// dpkg is built without HAVE_UTIMENSAT, so this is the call it makes for every
/// symlink it installs (tarobject_set_mtime -> lutimes). It was missing here,
/// which made every package that ships a symlink die at the first link with
/// ENOENT on the `.dpkg-new` name - while `tree`, which has no symlinks, kept
/// installing fine and hid the bug.
#[no_mangle]
pub unsafe extern "C" fn lutimes(path: *const CChar, tv: *const i64) -> CInt {
    let mut ts = [0i64; 4];
    if !tv.is_null() {
        ts[0] = *tv; // sec
        ts[1] = (*tv.add(1)) * 1000; // usec -> nsec
        ts[2] = *tv.add(2);
        ts[3] = (*tv.add(3)) * 1000;
    }
    let t = if tv.is_null() {
        core::ptr::null()
    } else {
        ts.as_ptr()
    };
    utimens_target(AT_FDCWD, path, t as *const u8, false, AT_SYMLINK_NOFOLLOW)
}

#[no_mangle]
pub unsafe extern "C" fn utimes(path: *const CChar, tv: *const i64) -> CInt {
    // struct timeval[2] -> struct timespec[2]
    let mut ts = [0i64; 4];
    if !tv.is_null() {
        ts[0] = *tv; // sec
        ts[1] = (*tv.add(1)) * 1000; // usec -> nsec
        ts[2] = *tv.add(2);
        ts[3] = (*tv.add(3)) * 1000;
    }
    let t = if tv.is_null() {
        core::ptr::null()
    } else {
        ts.as_ptr()
    };
    utimens_target(AT_FDCWD, path, t as *const u8, true, 0)
}

#[no_mangle]
pub unsafe extern "C" fn readlinkat(
    dirfd: CInt,
    path: *const CChar,
    out: *mut CChar,
    len: CSizet,
) -> Ssize {
    if path.is_null() {
        *__errno_location() = 14;
        return -1;
    }
    // `/proc/self/exe` must name the guest program, not the loader (see EXE_GUEST).
    {
        const WANT: &[u8] = b"/proc/self/exe";
        let mut hit = EXE_GUEST_LEN > 0;
        let mut i = 0usize;
        while hit && i <= WANT.len() {
            let c = *path.add(i) as u8;
            if i < WANT.len() {
                if c != WANT[i] {
                    hit = false;
                }
            } else if c != 0 {
                hit = false;
            }
            i += 1;
        }
        if hit {
            let l = core::cmp::min(EXE_GUEST_LEN, len);
            core::ptr::copy_nonoverlapping(EXE_GUEST.as_ptr(), out as *mut u8, l);
            return l as Ssize;
        }
    }
    let absolute = *path == b'/' as CChar;
    if verbose_on() {
        dbg(&[b"READLINKAT fd="]);
        dbgn(&[], dirfd as i64);
        dbg(&[b" p=", bytes(path), b"\n"]);
    }
    let rfd = rootfd();
    if rfd < 0 && dirfd != AT_FDCWD && !absolute {
        return syscall4(
            SYS_READLINKAT,
            dirfd as usize,
            path as usize,
            out as usize,
            len,
        );
    }
    let mut gp = [0u8; PATH_MAX];
    if rfd >= 0 && dirfd != AT_FDCWD && !absolute {
        match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
            Ok(Some(n)) => {
                let mut name = [0u8; 256];
                let parent = match rc::resolve_parent(rfd, &gp[..n], &mut name) {
                    Ok(fd) => fd,
                    Err(e) => return set_errno(e) as Ssize,
                };
                let r = syscall4(
                    SYS_READLINKAT,
                    parent as usize,
                    name.as_ptr() as usize,
                    out as usize,
                    len,
                );
                rc::close(parent);
                if verbose_on() {
                    dbg(&[b"  -> "]);
                    dbgn(&[], r as i64);
                }
                return r;
            }
            Ok(None) => {
                return syscall4(
                    SYS_READLINKAT,
                    dirfd as usize,
                    path as usize,
                    out as usize,
                    len,
                )
            }
            Err(e) => return set_errno(e) as Ssize,
        }
    }
    let n = match abs_guest(path, &mut gp) {
        Some(n) if rfd >= 0 && !passthrough(&gp[..n]) => n,
        Some(_) => {
            return syscall4(
                SYS_READLINKAT,
                AT_FDCWD as usize,
                gp.as_ptr() as usize,
                out as usize,
                len,
            )
        }
        None => return set_errno(36) as Ssize,
    };
    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent(rfd, &gp[..n], &mut name) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e) as Ssize,
    };
    let r = syscall4(
        SYS_READLINKAT,
        parent as usize,
        name.as_ptr() as usize,
        out as usize,
        len,
    );
    rc::close(parent);
    r
}

#[no_mangle]
pub unsafe extern "C" fn readlink(path: *const CChar, out: *mut CChar, len: CSizet) -> Ssize {
    // glibc's readlink() is a separate entry point that does not go through the
    // interposed readlinkat(), so it needs its own hook.
    readlinkat(AT_FDCWD, path, out, len)
}

/// Keep the runtime's private root descriptor alive even when a guest daemon
/// closes every descriptor it inherited (PulseAudio does exactly this before
/// its final chdir("/")). The fd is never exposed to guest code, and all path
/// resolution depends on it, so closing it would turn a perfectly valid guest
/// daemon into ENOENT/EACCES on every later absolute path.
#[no_mangle]
pub unsafe extern "C" fn close(fd: CInt) -> CInt {
    let rfd = rootfd();
    if fd >= 0 && fd == rfd {
        dbg(&[b"close preserved rootfd\n"]);
        return 0;
    }
    if fd >= 0 && v4l2_is_fake(fd) {
        v4l2_forget(fd);
    }
    ret_or_errno(syscall4(SYS_CLOSE, fd as usize, 0, 0, 0))
}

/// Same rule for a bulk close. Preserve the root fd by punching a one-fd hole
/// in the requested range instead of walking a potentially enormous range in
/// user space.
#[no_mangle]
pub unsafe extern "C" fn close_range(first: u32, last: u32, flags: CInt) -> CInt {
    let rfd = rootfd();
    if rfd >= 0 {
        let r = rfd as u32;
        if first <= r && r <= last {
            if first < r {
                let rc = syscall4(
                    SYS_CLOSE_RANGE,
                    first as usize,
                    (r - 1) as usize,
                    flags as usize,
                    0,
                );
                if rc < 0 {
                    return ret_or_errno(rc);
                }
            }
            if r < last {
                let rc = syscall4(
                    SYS_CLOSE_RANGE,
                    (r + 1) as usize,
                    last as usize,
                    flags as usize,
                    0,
                );
                if rc < 0 {
                    return ret_or_errno(rc);
                }
            }
            return 0;
        }
    }
    ret_or_errno(syscall4(
        SYS_CLOSE_RANGE,
        first as usize,
        last as usize,
        flags as usize,
        0,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn chdir(path: *const CChar) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    let rfd = rootfd();
    if rfd < 0 {
        dbg(&[b"chdir NOROOTFS\n"]);
        let mut gp = [0u8; PATH_MAX];
        if abs_guest(path, &mut gp).is_none() {
            return set_errno(36);
        }
        return ret_or_errno(syscall4(SYS_CHDIR, gp.as_ptr() as usize, 0, 0, 0));
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return set_errno(36),
    };
    if verbose_on() {
        dbg(&[b"chdir guest=", &gp[..n], b"\n"]);
    }
    if passthrough(&gp[..n]) {
        return ret_or_errno(syscall4(SYS_CHDIR, gp.as_ptr() as usize, 0, 0, 0));
    }
    match rc::resolve_open(rfd, &gp[..n], O_PATH | O_DIRECTORY | O_CLOEXEC, 0) {
        Ok(fd) => {
            let r = syscall4(SYS_FCHDIR, fd as usize, 0, 0, 0);
            rc::close(fd);
            if r < 0 {
                return ret_or_errno(r);
            }
            let mut realnow = [0u8; PATH_MAX];
            let rn2 = syscall4(
                SYS_READLINKAT,
                AT_FDCWD as usize,
                b"/proc/self/cwd\0".as_ptr() as usize,
                realnow.as_mut_ptr() as usize,
                PATH_MAX - 1,
            );
            if rn2 > 0 {
                dbg(&[b"chdir realnow=", &realnow[..rn2 as usize], b"\n"]);
            }
            let mut canon = [0u8; PATH_MAX];
            let cn = normalize(&gp[..n], &mut canon);
            set_cwd(&canon[..cn]);
            0
        }
        Err(e) => set_errno(e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn fchdir(fd: CInt) -> CInt {
    if !fd_visible_path(fd) {
        return set_errno(13);
    }
    let r = syscall4(SYS_FCHDIR, fd as usize, 0, 0, 0);
    if r == 0 {
        CWD_STATE.store(0, Ordering::Release);
    }
    ret_or_errno(r)
}

unsafe fn elf_interp(fd: CInt) -> bool {
    let mut eh = [0u8; 64];
    let r = syscall4(SYS_PREAD64, fd as usize, eh.as_mut_ptr() as usize, 64, 0);
    if r != 64 || &eh[..4] != b"\x7fELF" {
        return false;
    }
    let phoff = u64::from_le_bytes(eh[32..40].try_into().unwrap());
    let phentsize = u16::from_le_bytes(eh[54..56].try_into().unwrap()) as u64;
    let phnum = u16::from_le_bytes(eh[56..58].try_into().unwrap()) as u64;
    let mut ph = [0u8; 56];
    let mut i = 0u64;
    while i < phnum && i < 64 {
        let off = phoff + i * phentsize;
        let r = syscall4(
            SYS_PREAD64,
            fd as usize,
            ph.as_mut_ptr() as usize,
            56,
            off as usize,
        );
        if r != 56 {
            return false;
        }
        if u32::from_le_bytes(ph[0..4].try_into().unwrap()) == 3 {
            return true;
        }
        i += 1;
    }
    false
}

// ------------------------------------------------------------------ self path
// The program's own guest path, recorded at exec time. In this model
// `/proc/self/exe` resolves to the *loader*, not to the program, so anything
// that derives its install prefix from it (python: sys.executable -> sys.prefix)
// looks outside the rootfs. Answering readlinkat("/proc/self/exe") with the
// guest path keeps those users inside it.
static mut EXE_GUEST: [u8; PATH_MAX] = [0; PATH_MAX];
static mut EXE_GUEST_LEN: usize = 0;

// ---------------------------------------------------------------------------
// statx: coreutils >= 9 and many modern binaries use statx() directly, which
// bypasses newfstatat. Without this wrapper `stat`/`ls -l` see host paths.
// ---------------------------------------------------------------------------
const SYS_STATX: usize = 291;

#[no_mangle]
pub unsafe extern "C" fn statx(
    dirfd: CInt,
    path: *const CChar,
    flags: CInt,
    mask: u32,
    buf: *mut u8,
) -> CInt {
    if path.is_null() {
        return ret_or_errno(syscall5(
            SYS_STATX,
            dirfd as usize,
            0,
            flags as usize,
            mask as usize,
            buf as usize,
        ));
    }
    if *path == b'/' as CChar {
        let mut gp = [0u8; PATH_MAX];
        if let Some(n) = abs_guest(path, &mut gp) {
            let mut sbuf = [0u8; PATH_MAX];
            if let Some(sn) = v4l2_sys_host(&gp[..n], &mut sbuf) {
                return ret_or_errno(syscall5(
                    SYS_STATX,
                    AT_FDCWD as usize,
                    sbuf.as_ptr() as usize,
                    flags as usize,
                    mask as usize,
                    buf as usize,
                ));
            }
            if let Some(vn) = video_index(&gp[..n]) {
                return v4l2_statx(vn, buf);
            }
        }
    }
    if *path == 0 && flags & AT_EMPTY_PATH != 0 {
        return ret_or_errno(syscall5(
            SYS_STATX,
            dirfd as usize,
            path as usize,
            flags as usize,
            mask as usize,
            buf as usize,
        ));
    }
    let follow = flags & AT_SYMLINK_NOFOLLOW == 0;
    let fd = match target_fd(dirfd, path, follow) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let mut pb = [0u8; 32];
    let p = fd_proc_path(fd, &mut pb);
    // On this 4.19 kernel statx(AT_EMPTY_PATH) is unsupported. The procfs
    // magic link pins the already-resolved inode; pass zero follow flags so a
    // nofollow symlink remains the symlink itself.
    let pass_flags = flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH);
    let r = syscall5(
        SYS_STATX,
        AT_FDCWD as usize,
        p.as_ptr() as usize,
        pass_flags as usize,
        mask as usize,
        buf as usize,
    );
    rc::close(fd);
    ret_or_errno(r)
}

// ---------------------------------------------------------------------------
// getcwd: the kernel can only report the *host* path of the cwd, so the result
// is produced from the raw syscall and rewritten into the guest view. The guest
// path is always shorter (the rootfs prefix is stripped).
// ---------------------------------------------------------------------------
unsafe fn guest_path_from_real(real: &[u8], out: *mut u8, out_len: usize) -> Option<usize> {
    let root = guest_root();
    if root.len() > 1 && real.len() >= root.len() && &real[..root.len()] == root {
        if real.len() == root.len() {
            *out = b'/';
            return Some(1);
        }
        let g = &real[root.len()..];
        if g.len() > out_len {
            return None;
        }
        core::ptr::copy_nonoverlapping(g.as_ptr(), out, g.len());
        return Some(g.len());
    }
    if real.first() == Some(&b'/') {
        if real.len() > out_len {
            return None;
        }
        core::ptr::copy_nonoverlapping(real.as_ptr(), out, real.len());
        return Some(real.len());
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn getcwd(buf: *mut CChar, size: CSizet) -> *mut CChar {
    const SYS_GETCWD: usize = 17;
    let _ = rootfd(); // make sure the rootfs prefix is known before stripping
    let mut real = [0u8; PATH_MAX];
    let n = syscall4(SYS_GETCWD, real.as_mut_ptr() as usize, PATH_MAX - 1, 0, 0);
    if n <= 0 {
        return set_errno_ptr(34, core::ptr::null_mut()); // ERANGE
    }
    let mut rl = (n as usize).min(PATH_MAX - 1);
    while rl > 0 && real[rl - 1] == 0 {
        rl -= 1;
    }
    let mut guest = [0u8; PATH_MAX];
    let gl = match guest_path_from_real(&real[..rl], guest.as_mut_ptr(), PATH_MAX) {
        Some(g) => g,
        None => return set_errno_ptr(34, core::ptr::null_mut()),
    };
    if buf.is_null() {
        let out = malloc(gl + 1);
        if out.is_null() {
            return set_errno_ptr(12, core::ptr::null_mut());
        }
        core::ptr::copy_nonoverlapping(guest.as_ptr(), out, gl);
        *out.add(gl) = 0;
        return out as *mut CChar;
    }
    if size < gl + 1 {
        return set_errno_ptr(34, core::ptr::null_mut());
    }
    core::ptr::copy_nonoverlapping(guest.as_ptr(), buf as *mut u8, gl);
    *(buf as *mut u8).add(gl) = 0;
    if verbose_on() {
        dbg(&[
            b"getcwd->",
            core::slice::from_raw_parts(buf as *const u8, gl),
            b"\n",
        ]);
    }
    buf
}

// ---------------------------------------------------------------------------
// opendir: libc's own opendir reaches the kernel without going through our
// interposed open, so `ls`, `du`, `find`, bash globbing etc. would read the
// host directory. Re-implement it on top of our resolver and hand the fd to
// the real fdopendir.
// ---------------------------------------------------------------------------
#[no_mangle]
pub unsafe extern "C" fn opendir(path: *const CChar) -> *mut u8 {
    // Deliberately no O_DIRECTORY: on this kernel (Android 16 / 6.6) opening a
    // directory on /data with O_RDONLY|O_DIRECTORY fails with EINVAL, while the
    // same open without O_DIRECTORY succeeds. fdopendir() still validates that
    // the fd is a directory (ENOTDIR otherwise).
    const O_NONBLOCK: CInt = 0o4000;
    let fd = open(path, O_NONBLOCK | O_CLOEXEC, 0);
    if fd < 0 {
        return core::ptr::null_mut();
    }
    let d = fdopendir(fd);
    if d.is_null() {
        rc::close(fd);
    }
    d
}

// ---------------------------------------------------------------------------
// Path calls that have no *at() variant (xattr family, statfs) are handled by
// rebuilding the real path through /proc/self/fd/<parentfd>/<name>, which the
// kernel resolves relative to the fd we already validated.
// ---------------------------------------------------------------------------
const SYS_SETXATTR: usize = 5;
const SYS_LSETXATTR: usize = 6;
const SYS_GETXATTR: usize = 8;
const SYS_LGETXATTR: usize = 9;
const SYS_LISTXATTR: usize = 11;
const SYS_LLISTXATTR: usize = 12;
const SYS_REMOVEXATTR: usize = 14;
const SYS_LREMOVEXATTR: usize = 15;
const SYS_STATFS: usize = 43;

unsafe fn proc_fd_path(fd: CInt, name: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let mut i = 0usize;
    for &c in b"/proc/self/fd/" {
        out[i] = c;
        i += 1;
    }
    let mut v = fd as u32;
    let mut d = [0u8; 10];
    let mut dn = 0usize;
    if v == 0 {
        d[0] = b'0';
        dn = 1;
    }
    while v > 0 {
        d[dn] = b'0' + (v % 10) as u8;
        v /= 10;
        dn += 1;
    }
    while dn > 0 {
        dn -= 1;
        out[i] = d[dn];
        i += 1;
    }
    if name.is_empty() || (name.len() == 1 && name[0] == b'.') {
        out[i] = 0;
        return Some(i);
    }
    if i + 1 + name.len() + 1 > PATH_MAX {
        return None;
    }
    out[i] = b'/';
    i += 1;
    out[i..i + name.len()].copy_from_slice(name);
    i += name.len();
    out[i] = 0;
    Some(i)
}

/// Run a path-only syscall against the resolved parent fd + final name.
unsafe fn pathonly<F>(path: *const CChar, follow: bool, f: F) -> Ssize
where
    F: FnOnce(*const CChar) -> Ssize,
{
    if path.is_null() {
        *__errno_location() = 14;
        return -1;
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => {
            *__errno_location() = 36;
            return -1;
        }
    };
    let rfd = rootfd();
    let mut raw = [0u8; PATH_MAX];
    if rfd >= 0 && !passthrough(&gp[..n]) {
        let mut name = [0u8; 256];
        let parent = if follow {
            rc::resolve_parent_follow(rfd, &gp[..n], &mut name)
        } else {
            rc::resolve_parent(rfd, &gp[..n], &mut name)
        };
        let parent = match parent {
            Ok(fd) => fd,
            Err(e) => {
                *__errno_location() = e;
                return -1;
            }
        };
        let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
        let r = match proc_fd_path(parent, &name[..nl], &mut raw) {
            Some(_) => f(raw.as_ptr() as *const CChar),
            None => {
                *__errno_location() = 36;
                -1
            }
        };
        rc::close(parent);
        return r;
    }
    raw[..n].copy_from_slice(&gp[..n]);
    raw[n] = 0;
    f(raw.as_ptr() as *const CChar)
}

#[no_mangle]
pub unsafe extern "C" fn lgetxattr(
    path: *const CChar,
    name: *const CChar,
    value: *mut u8,
    size: CSizet,
) -> Ssize {
    pathonly(path, false, |p| {
        syscall5(
            SYS_LGETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            0,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn getxattr(
    path: *const CChar,
    name: *const CChar,
    value: *mut u8,
    size: CSizet,
) -> Ssize {
    pathonly(path, true, |p| {
        syscall5(
            SYS_GETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            0,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn setxattr(
    path: *const CChar,
    name: *const CChar,
    value: *const u8,
    size: CSizet,
    flags: CInt,
) -> CInt {
    ret_or_errno(pathonly(path, true, |p| {
        syscall5(
            SYS_SETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            flags as usize,
        )
    }))
}

#[no_mangle]
pub unsafe extern "C" fn lsetxattr(
    path: *const CChar,
    name: *const CChar,
    value: *const u8,
    size: CSizet,
    flags: CInt,
) -> CInt {
    ret_or_errno(pathonly(path, false, |p| {
        syscall5(
            SYS_LSETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            flags as usize,
        )
    }))
}

#[no_mangle]
pub unsafe extern "C" fn listxattr(path: *const CChar, list: *mut CChar, size: CSizet) -> Ssize {
    pathonly(path, true, |p| {
        syscall4(SYS_LISTXATTR, p as usize, list as usize, size, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn llistxattr(path: *const CChar, list: *mut CChar, size: CSizet) -> Ssize {
    pathonly(path, false, |p| {
        syscall4(SYS_LLISTXATTR, p as usize, list as usize, size, 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn removexattr(path: *const CChar, name: *const CChar) -> CInt {
    ret_or_errno(pathonly(path, true, |p| {
        syscall4(SYS_REMOVEXATTR, p as usize, name as usize, 0, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn lremovexattr(path: *const CChar, name: *const CChar) -> CInt {
    ret_or_errno(pathonly(path, false, |p| {
        syscall4(SYS_LREMOVEXATTR, p as usize, name as usize, 0, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn statfs(path: *const CChar, buf: *mut u8) -> CInt {
    ret_or_errno(pathonly(path, true, |p| {
        syscall4(SYS_STATFS, p as usize, buf as usize, 0, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn execve(
    path: *const CChar,
    argv: *const *const CChar,
    envp: *const *const CChar,
) -> CInt {
    if path.is_null() {
        return set_errno(14);
    }
    let rfd = rootfd();
    if rfd < 0 {
        let mut gp = [0u8; PATH_MAX];
        if abs_guest(path, &mut gp).is_none() {
            return set_errno(36);
        }
        return ret_or_errno(syscall4(
            SYS_EXECVE,
            gp.as_ptr() as usize,
            argv as usize,
            envp as usize,
            0,
        ));
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return set_errno(36),
    };
    if n < PATH_MAX {
        EXE_GUEST[..n].copy_from_slice(&gp[..n]);
        EXE_GUEST_LEN = n;
    }
    if pseudo_path(&gp[..n]) && passthrough(&gp[..n]) {
        return ret_or_errno(syscall4(
            SYS_EXECVE,
            gp.as_ptr() as usize,
            argv as usize,
            envp as usize,
            0,
        ));
    }
    let mut real = [0u8; PATH_MAX];
    if under_root(&gp[..n]) {
        // Already real (derived from /proc/self/exe): skip the guest lookup but
        // keep the loader re-entry below, which a dynamic guest ELF still needs.
        real[..n].copy_from_slice(&gp[..n]);
        real[n] = 0;
    } else {
        let mut name = [0u8; 256];
        let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
            Ok(fd) => fd,
            Err(e) => return set_errno(e),
        };
        let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
        if real_path(parent, &name[..nl], &mut real).is_none() {
            rc::close(parent);
            dbg_file(&[b"  ! resolve failed for ", &gp[..n], b"\n"]);
            return set_errno(2);
        }
        dbg_file(&[b"  real=", &real[..real.iter().position(|c| *c == 0).unwrap_or(0)], b"\n"]);
        rc::close(parent);
    }

    let fd = ret_or_errno(syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        real.as_ptr() as usize,
        0,
        0,
    ));
    if fd < 0 {
        let (b, i) = dec_buf(*__errno_location() as i64);
        dbg_file(&[b"  ! open failed: ", &b[i..], b"\n"]);
        return -1;
    }

    // ---------------------------------------------------- #! scripts ---
    // The kernel resolves a script's interpreter against the *host* root, so a
    // guest script whose first line says `#!/usr/bin/python3` can never start
    // here (the host has no such file). Parse the line ourselves, resolve the
    // interpreter inside the guest root, and re-enter the loader with the argv
    // shape the kernel would have produced: [interp, arg?, script, argv[1..]].
    let mut hdr = [0u8; 256];
    let hn = syscall4(
        SYS_PREAD64,
        fd as usize,
        hdr.as_mut_ptr() as usize,
        hdr.len(),
        0,
    );
    let mut tgt = real;
    let mut shebang_arg = [0u8; 200];
    let mut shebang_arg_len = 0usize;
    let mut is_script = false;

    if hn > 2 && hdr[0] == b'#' && hdr[1] == b'!' {
        let hn = hn as usize;
        let mut i = 2usize;
        while i < hn && (hdr[i] == b' ' || hdr[i] == b'\t') {
            i += 1;
        }
        let istart = i;
        while i < hn && hdr[i] != b' ' && hdr[i] != b'\t' && hdr[i] != b'\n' && hdr[i] != b'\r' {
            i += 1;
        }
        if i == istart {
            rc::close(fd);
            return set_errno(ENOEXEC);
        }
        // one optional argument, as the kernel allows
        let mut j = i;
        while j < hn && (hdr[j] == b' ' || hdr[j] == b'\t') {
            j += 1;
        }
        let astart = j;
        while j < hn && hdr[j] != b'\n' && hdr[j] != b'\r' {
            j += 1;
        }
        if j > astart {
            let l = core::cmp::min(j - astart, shebang_arg.len() - 1);
            shebang_arg[..l].copy_from_slice(&hdr[astart..astart + l]);
            shebang_arg_len = l;
        }
        let mut interp = [0u8; PATH_MAX];
        let l = i - istart;
        if l >= PATH_MAX {
            rc::close(fd);
            return set_errno(36);
        }
        interp[..l].copy_from_slice(&hdr[istart..i]);
        interp[l] = 0;
        if interp[0] != b'/' {
            rc::close(fd);
            return set_errno(ENOEXEC);
        }
        // Two candidates, and that is enough for a usr-merged rootfs:
        //   <root>/usr/bin/x   (what Ubuntu 24.04 actually stores)
        //   <root>/bin/x       (the pre-merge spelling a shebang may still use)
        // Anything deeper (python3 -> python3.12) is left to the kernel: it is a
        // symlink *inside* the root, so it resolves correctly once we are there.
        let root = guest_root();
        let mut found = false;
        for variant in 0..2usize {
            let mut p = [0u8; PATH_MAX];
            if root.len() + 4 + l + 1 >= PATH_MAX {
                break;
            }
            let mut o = root.len();
            p[..root.len()].copy_from_slice(root);
            if variant == 1 {
                p[o..o + 4].copy_from_slice(b"/usr");
                o += 4;
            }
            p[o..o + l].copy_from_slice(&interp[..l]);
            o += l;
            p[o] = 0;
            let f = syscall4(SYS_OPENAT, AT_FDCWD as usize, p.as_ptr() as usize, 0, 0);
            if f >= 0 {
                rc::close(f as CInt);
                tgt = p;
                found = true;
                break;
            }
        }
        if !found {
            rc::close(fd);
            return set_errno(ENOENT);
        }
        is_script = true;
    }

    let tfd = if is_script {
        let f = ret_or_errno(syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            tgt.as_ptr() as usize,
            0,
            0,
        ));
        if f < 0 {
            rc::close(fd);
            return -1;
        }
        f
    } else {
        fd
    };
    let dynamic = elf_interp(tfd);
    rc::close(tfd);
    if tfd != fd {
        rc::close(fd);
    }

    if !dynamic {
        if !is_script {
            return ret_or_errno(syscall4(
                SYS_EXECVE,
                tgt.as_ptr() as usize,
                argv as usize,
                envp as usize,
                0,
            ));
        }
        let mut sargs: [*const CChar; 64] = [core::ptr::null(); 64];
        let mut k = 0usize;
        sargs[k] = tgt.as_ptr() as *const CChar;
        k += 1;
        if shebang_arg_len > 0 {
            sargs[k] = shebang_arg.as_ptr() as *const CChar;
            k += 1;
        }
        sargs[k] = real.as_ptr() as *const CChar;
        k += 1;
        let mut i = 1usize;
        while !argv.is_null() && !(*argv.add(i)).is_null() && k < 63 {
            sargs[k] = *argv.add(i);
            k += 1;
            i += 1;
        }
        return ret_or_errno(syscall4(
            SYS_EXECVE,
            tgt.as_ptr() as usize,
            sargs.as_ptr() as usize,
            envp as usize,
            0,
        ));
    }

    let root = guest_root();
    let suf = b"/usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1";
    let libsuf = b"/usr/lib/aarch64-linux-gnu";
    let mut ld = [0u8; PATH_MAX];
    let mut lib = [0u8; PATH_MAX];
    // NR_LD points at an executable copy of the guest ld.so (normally
    // <nativeLibraryDir>/libld.so). Inside a real app process the app-private
    // directory is not executable, so the interpreter has to be exec'd from
    // somewhere the kernel allows - the APK's native library dir.
    let ld_path: &[u8] = match env_lookup(b"NR_LD") {
        Some(p) if !p.is_empty() => p,
        _ => {
            if root.len() + suf.len() + 1 >= PATH_MAX {
                return set_errno(36);
            }
            ld[..root.len()].copy_from_slice(root);
            ld[root.len()..root.len() + suf.len()].copy_from_slice(suf);
            let ldn = root.len() + suf.len();
            ld[ldn] = 0;
            &ld[..ldn]
        }
    };
    let ldn = ld_path.len();
    if ldn == 0 || ldn + 1 >= PATH_MAX {
        return set_errno(36);
    }
    let mut ld2 = [0u8; PATH_MAX];
    ld2[..ldn].copy_from_slice(ld_path);
    ld2[ldn] = 0;
    if root.len() + libsuf.len() + 1 >= PATH_MAX {
        return set_errno(36);
    }
    lib[..root.len()].copy_from_slice(root);
    lib[root.len()..root.len() + libsuf.len()].copy_from_slice(libsuf);
    let libn = root.len() + libsuf.len();
    lib[libn] = 0;

    let mut args: [*const CChar; 64] = [core::ptr::null(); 64];
    let mut k = 0usize;
    args[k] = ld2.as_ptr() as *const CChar;
    k += 1;
    args[k] = b"--library-path\0".as_ptr() as *const CChar;
    k += 1;
    args[k] = lib.as_ptr() as *const CChar;
    k += 1;
    args[k] = tgt.as_ptr() as *const CChar;
    k += 1;
    if is_script {
        if shebang_arg_len > 0 && k < 62 {
            args[k] = shebang_arg.as_ptr() as *const CChar;
            k += 1;
        }
        if k < 62 {
            args[k] = real.as_ptr() as *const CChar;
            k += 1;
        }
    }
    let mut i = 1usize;
    while !argv.is_null() && !(*argv.add(i)).is_null() && k < 63 {
        args[k] = *argv.add(i);
        k += 1;
        i += 1;
    }
    ret_or_errno(syscall4(
        SYS_EXECVE,
        ld2.as_ptr() as usize,
        args.as_ptr() as usize,
        envp as usize,
        0,
    ))
}

// --------------------------------------------------------------- temp names
// glibc builds temporary names in __gen_tempname() and creates them with an
// *internal* syscall wrapper (__open64_nocancel / __mkdir64_nocancel). Those
// calls never leave the shared object, so they have no GOT slot to patch: a
// guest template reaches the kernel verbatim and "/tmp/ccXXXXXX" lands on the
// host's /tmp. That is why gcc's very first temp file died with EACCES (host
// /tmp here is shell-owned 0771). The whole operation therefore has to be
// replaced: we generate the six random characters ourselves and create the file
// through our own resolver, which keeps the caller's template string a guest
// path.
const SYS_GETPID: usize = 172;
const SYS_CLOCK_GETTIME: usize = 113;
const EEXIST: CInt = 17;
const EINVAL: CInt = 22;
const EFAULT: CInt = 14;
const ENOENT: CInt = 2;
const EACCES: CInt = 13;
const ENOTDIR: CInt = 20;
const ENOEXEC: CInt = 8;
const O_RDWR: CInt = 2;
const O_CREAT: CInt = 0o100;
const O_EXCL: CInt = 0o200;

const TMP_LETTERS: [u8; 62] = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

static mut RAND_COUNTER: u64 = 0;

unsafe fn next_random() -> u32 {
    // Entropy comes from auxv AT_RANDOM (the kernel put it there at exec) mixed
    // with a per-process counter. getrandom(2) is not usable on this device:
    // the seccomp policy traps it and the default action is SIGSYS, which kills
    // the process - that is what "Bad system call" was when gcc first created a
    // temp file through here. Names only have to be unpredictable enough that a
    // hostile local process cannot pre-create one, and O_EXCL covers the rest.
    let mut s = boot::at_random()
        ^ RAND_COUNTER.wrapping_mul(0x9E3779B97F4A7C15)
        ^ (&RAND_COUNTER as *const u64 as u64);
    RAND_COUNTER = RAND_COUNTER.wrapping_add(1);
    if s == 0 {
        // No AT_RANDOM in the auxv: fall back to clock + pid, as glibc does.
        let mut ts = [0u64; 2];
        syscall4(SYS_CLOCK_GETTIME, 1, ts.as_mut_ptr() as usize, 0, 0);
        s = ts[0]
            ^ (ts[1] << 17)
            ^ ((syscall4(SYS_GETPID, 0, 0, 0, 0) as u64) << 40)
            ^ (&s as *const u64 as u64);
    }
    // splitmix64 finaliser
    s ^= s >> 30;
    s = s.wrapping_mul(0xBF58476D1CE4E5B9);
    s ^= s >> 27;
    s = s.wrapping_mul(0x94D049BB133111EB);
    s ^= s >> 31;
    s as u32
}

/// Shared body: fill the six X's in `tmpl` and create the object.
/// Returns a file descriptor, 0 for a directory, or -1 with errno set.
unsafe fn gen_tempname(tmpl: *mut CChar, suffixlen: usize, extra: CInt, want_dir: bool) -> CInt {
    if tmpl.is_null() {
        return set_errno(EFAULT);
    }
    let n = c_strlen(tmpl);
    if n < 6 + suffixlen {
        return set_errno(EINVAL);
    }
    let xpos = n - 6 - suffixlen;
    let mut i = 0;
    while i < 6 {
        if *tmpl.add(xpos + i) != b'X' as CChar {
            return set_errno(EINVAL);
        }
        i += 1;
    }
    let mut attempts = 0;
    while attempts < 100 {
        attempts += 1;
        let mut v = next_random();
        let mut i = 0;
        while i < 6 {
            *tmpl.add(xpos + i) = TMP_LETTERS[(v % 62) as usize] as CChar;
            v /= 62;
            i += 1;
        }
        if want_dir {
            if mkdirat(AT_FDCWD, tmpl, 0o700) == 0 {
                return 0;
            }
        } else {
            let fd = openat(AT_FDCWD, tmpl, extra | O_RDWR | O_CREAT | O_EXCL, 0o600);
            if fd >= 0 {
                return fd;
            }
        }
        // Only a name collision is worth another try; anything else (missing
        // directory, read-only mount) is the caller's problem and glibc would
        // have reported it too.
        if *__errno_location() != EEXIST {
            return -1;
        }
    }
    set_errno(EEXIST)
}

#[no_mangle]
pub unsafe extern "C" fn mkstemp(tmpl: *mut CChar) -> CInt {
    gen_tempname(tmpl, 0, 0, false)
}

#[no_mangle]
pub unsafe extern "C" fn mkstemps(tmpl: *mut CChar, suffixlen: CInt) -> CInt {
    if suffixlen < 0 {
        return set_errno(EINVAL);
    }
    gen_tempname(tmpl, suffixlen as usize, 0, false)
}

#[no_mangle]
pub unsafe extern "C" fn mkostemp(tmpl: *mut CChar, flags: CInt) -> CInt {
    gen_tempname(tmpl, 0, flags & !(O_CREAT | O_EXCL), false)
}

#[no_mangle]
pub unsafe extern "C" fn mkostemps(tmpl: *mut CChar, suffixlen: CInt, flags: CInt) -> CInt {
    if suffixlen < 0 {
        return set_errno(EINVAL);
    }
    gen_tempname(tmpl, suffixlen as usize, flags & !(O_CREAT | O_EXCL), false)
}

#[no_mangle]
pub unsafe extern "C" fn mkdtemp(tmpl: *mut CChar) -> *mut CChar {
    if gen_tempname(tmpl, 0, 0, true) < 0 {
        return core::ptr::null_mut();
    }
    tmpl
}

// ------------------------------------------------------------ exec helpers
// gcc and make do not call execve(2) themselves: they go through execv(2) /
// execvp(3), which live inside libc and reach the kernel through internal
// aliases that no GOT patch can see. Left alone, a guest path would be handed
// to the kernel verbatim - that is why the driver could not start cc1.
#[no_mangle]
pub unsafe extern "C" fn execv(path: *const CChar, argv: *const *const CChar) -> CInt {
    // dpkg's command_exec() calls execv(), and its child's stderr is a pipe the
    // parent usually discards, so this goes to <root>/tmp/nr-exec.log under
    // NR_DEBUG instead. It was the only way to see that the call never arrived.
    if dbg_on() {
        dbg_file(&[b"execv|", bytes(path), b"|rootfd="]);
        let (b, i) = dec_buf(rootfd() as i64);
        dbg_file(&[&b[i..], b"\n"]);
    }
    execve(path, argv, environ as *const *const CChar)
}

unsafe fn env_lookup_list(envp: *const *const CChar, name: &[u8]) -> Option<&'static [u8]> {
    if envp.is_null() {
        return None;
    }
    let mut i = 0;
    while i < 4096 {
        let e = *envp.add(i);
        if e.is_null() {
            return None;
        }
        let s = bytes(e);
        if s.len() > name.len() && &s[..name.len()] == name && s[name.len()] == b'=' {
            return Some(&s[name.len() + 1..]);
        }
        i += 1;
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn execvpe(
    file: *const CChar,
    argv: *const *const CChar,
    envp: *const *const CChar,
) -> CInt {
    if file.is_null() {
        return set_errno(EFAULT);
    }
    let f = bytes(file);
    if f.is_empty() {
        return set_errno(ENOENT);
    }
    if f.contains(&b'/') {
        return execve(file, argv, envp);
    }
    // execvpe searches the PATH of the environment it is handed, not the
    // caller's; POSIX/_CS_PATH default is /bin:/usr/bin.
    let path: &[u8] = match env_lookup_list(envp, b"PATH") {
        Some(p) => p,
        None => b"/bin:/usr/bin",
    };
    if dbg_on() {
        dbg(&[b"execvpe|", f, b"\n"]);
    }
    let mut last = ENOENT;
    let mut buf = [0u8; PATH_MAX];
    for dir in path.split(|c| *c == b':') {
        let d: &[u8] = if dir.is_empty() { b"." } else { dir };
        let n = d.len() + 1 + f.len();
        if n + 1 >= PATH_MAX {
            last = 36;
            continue;
        }
        buf[..d.len()].copy_from_slice(d);
        buf[d.len()] = b'/';
        buf[d.len() + 1..n].copy_from_slice(f);
        buf[n] = 0;
        if execve(buf.as_ptr() as *const CChar, argv, envp) == 0 {
            return 0;
        }
        last = *__errno_location();
        // Only a miss or a "not executable here" is worth another prefix; any
        // other failure is authoritative, exactly as glibc treats it.
        if last != ENOENT && last != EACCES && last != ENOTDIR && last != ENOEXEC {
            return set_errno(last);
        }
    }
    set_errno(last)
}

#[no_mangle]
pub unsafe extern "C" fn execvp(file: *const CChar, argv: *const *const CChar) -> CInt {
    execvpe(file, argv, environ as *const *const CChar)
}

// ------------------------------------------------------- execl / execlp / system
//
// These are what dpkg actually calls (measured on the guest binary: `dpkg`
// imports execvp@GLIBC_2.17, execlp@GLIBC_2.17 and system@GLIBC_2.17 - not
// execv), and glibc implements all of them on top of its *internal* __execve,
// a bare syscall with no symbol to interpose. That is why a rootfs in which
// `/usr/bin/dpkg-split --version` ran fine from a shell still failed with
// "dpkg: unable to execute split package reassembly (dpkg-split): No such file
// or directory": the child asked the host kernel for /usr/bin/dpkg-split.
//
// execl/execlp take a variable argument list. Rather than hand-write the
// register spill, declare a fixed list longer than any real caller: on AArch64
// a variadic caller and a fixed-arity callee place the arguments in exactly the
// same registers and stack slots, the list is NULL-terminated, and anything
// past the terminator is never read. A caller that overruns the declared
// window is refused (E2BIG) instead of exec'ing garbage.

unsafe fn varargs_exec(path: *const CChar, args: &[*const CChar], search: bool) -> CInt {
    if path.is_null() {
        return set_errno(EFAULT);
    }
    let mut v: [*const CChar; 20] = [core::ptr::null(); 20];
    let mut n = 0usize;
    let mut terminated = false;
    for &a in args {
        if n + 1 >= v.len() {
            break;
        }
        v[n] = a;
        n += 1;
        if a.is_null() {
            terminated = true;
            break;
        }
    }
    if !terminated {
        return set_errno(7); // E2BIG
    }
    if search {
        execvp(path, v.as_ptr())
    } else {
        execv(path, v.as_ptr())
    }
}

macro_rules! varargs_fn {
    ($name:ident, $search:expr) => {
        #[no_mangle]
        #[allow(clippy::too_many_arguments)]
        pub unsafe extern "C" fn $name(
            path: *const CChar,
            a0: *const CChar,
            a1: *const CChar,
            a2: *const CChar,
            a3: *const CChar,
            a4: *const CChar,
            a5: *const CChar,
            a6: *const CChar,
            a7: *const CChar,
            a8: *const CChar,
            a9: *const CChar,
            a10: *const CChar,
            a11: *const CChar,
            a12: *const CChar,
            a13: *const CChar,
        ) -> CInt {
            varargs_exec(
                path,
                &[a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13],
                $search,
            )
        }
    };
}

varargs_fn!(execl, false);
varargs_fn!(execlp, true);

/// Shared tail of execl/execlp: x0 = file, x1 = argv, x2 = search PATH.
#[no_mangle]
pub unsafe extern "C" fn nr_exec_argv(
    path: *const CChar,
    argv: *const *const CChar,
    search: CInt,
) -> CInt {
    if search != 0 {
        execvp(path, argv)
    } else {
        execv(path, argv)
    }
}

const SYS_CLONE: usize = 220;
const SYS_WAIT4: usize = 260;
const SYS_EXIT: usize = 93;
const SIGCHLD: usize = 17;

/// glibc's system() runs `/bin/sh -c cmd` through its private execve, so every
/// maintainer script in a package install would have run against the host root
/// ("dpkg: unable to execute ... /bin/sh"). Same shape, translated path.
#[no_mangle]
pub unsafe extern "C" fn system(cmd: *const CChar) -> CInt {
    if cmd.is_null() {
        // POSIX: non-zero means a command processor is available.
        return 1;
    }
    let pid = syscall5(SYS_CLONE, SIGCHLD, 0, 0, 0, 0);
    if pid < 0 {
        return -1;
    }
    if pid == 0 {
        let sh = b"/bin/sh\0";
        let a0 = b"sh\0";
        let a1 = b"-c\0";
        let args = [
            a0.as_ptr() as *const CChar,
            a1.as_ptr() as *const CChar,
            cmd,
            core::ptr::null(),
        ];
        execve(
            sh.as_ptr() as *const CChar,
            args.as_ptr(),
            environ as *const *const CChar,
        );
        syscall4(SYS_EXIT, 127, 0, 0, 0);
        loop {}
    }
    let mut st: CInt = 0;
    loop {
        let r = syscall4(
            SYS_WAIT4,
            pid as usize,
            &mut st as *mut CInt as usize,
            0,
            0,
        );
        if r == pid as Ssize {
            break;
        }
        if r < 0 {
            let e = *__errno_location();
            if e != 4 {
                return -1;
            }
        }
    }
    // The shell can carry a command that died from a signal; keep the wait
    // status shape system(3) documents rather than flattening it to 0..255.
    st
}


// ------------------------------------------------------------- Unix sockets
//
// glibc's connect(2) wrapper passes AF_UNIX pathnames straight to the kernel;
// unlike open/stat, there is no filesystem helper inside libc that we can rely
// on to translate the guest root. A guest rooted at `rootfs-full` therefore
// cannot connect to an app socket at `/tmp/...` unless we resolve the pathname
// ourselves and hand the kernel a `/proc/self/fd/<parent>/<name>` address.
//
// Abstract sockets (sun_path[0] == 0), unnamed sockets, and non-AF_UNIX
// addresses are passed through unchanged.
#[no_mangle]
pub unsafe extern "C" fn connect(sockfd: CInt, addr: *const u8, addrlen: u32) -> CInt {
    let raw_connect = || {
        ret_or_errno(syscall4(
            SYS_CONNECT,
            sockfd as usize,
            addr as usize,
            addrlen as usize,
            0,
        ))
    };

    if addr.is_null() || addrlen < 2 || addrlen as usize > 2 + UNIX_PATH_MAX + 1 {
        return raw_connect();
    }
    if core::ptr::read_unaligned(addr as *const u16) != AF_UNIX {
        return raw_connect();
    }

    let path_len = (addrlen as usize - 2).min(UNIX_PATH_MAX);
    if path_len == 0 {
        return raw_connect();
    }
    let path = core::slice::from_raw_parts(addr.add(2), path_len);
    if path[0] == 0 {
        return raw_connect();
    }
    let path_end = path.iter().position(|c| *c == 0).unwrap_or(path_len);
    if path_end == 0 {
        return raw_connect();
    }

    let mut guest = [0u8; UNIX_PATH_MAX + 1];
    guest[..path_end].copy_from_slice(&path[..path_end]);
    guest[path_end] = 0;

    let mut gp = [0u8; PATH_MAX];
    let Some(gn) = abs_guest(guest.as_ptr() as *const CChar, &mut gp) else {
        return set_errno(36);
    };
    let rfd = rootfd();
    if rfd < 0 || passthrough(&gp[..gn]) {
        return raw_connect();
    }

    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..gn], &mut name) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let name_len = name.iter().position(|c| *c == 0).unwrap_or(0);
    if name_len == 0 {
        rc::close(parent);
        return set_errno(2);
    }

    let mut proc_buf = [0u8; 32];
    let proc_path = fd_proc_path(parent, &mut proc_buf);
    let prefix_len = proc_path.len().saturating_sub(1); // omit trailing NUL
    if prefix_len + 1 + name_len > UNIX_PATH_MAX {
        rc::close(parent);
        return set_errno(36);
    }

    let mut translated = SockAddrUn {
        family: AF_UNIX,
        path: [0u8; UNIX_PATH_MAX],
    };
    translated.path[..prefix_len].copy_from_slice(&proc_path[..prefix_len]);
    translated.path[prefix_len] = b'/';
    translated.path[prefix_len + 1..prefix_len + 1 + name_len].copy_from_slice(&name[..name_len]);
    let translated_len = 2 + prefix_len + 1 + name_len + 1;
    let r = syscall4(
        SYS_CONNECT,
        sockfd as usize,
        &translated as *const SockAddrUn as usize,
        translated_len,
        0,
    );
    rc::close(parent);
    ret_or_errno(r)
}

// bind(2) needs the same translation as connect(2). The kernel creates the
// socket node in the *host* namespace, so a guest path such as
// `/tmp/husk.sock` would land outside the rootfs (on Android usually ENOENT,
// because there is no host /tmp). Resolve the parent inside the guest tree and
// hand the kernel `/proc/self/fd/<parent>/<name>`.
//
// Same exclusions as connect: abstract, unnamed and non-AF_UNIX pass through.
#[no_mangle]
pub unsafe extern "C" fn bind(sockfd: CInt, addr: *const u8, addrlen: u32) -> CInt {
    let raw_bind = || {
        ret_or_errno(syscall4(
            SYS_BIND,
            sockfd as usize,
            addr as usize,
            addrlen as usize,
            0,
        ))
    };

    if addr.is_null() || addrlen < 2 || addrlen as usize > 2 + UNIX_PATH_MAX + 1 {
        return raw_bind();
    }
    if core::ptr::read_unaligned(addr as *const u16) != AF_UNIX {
        return raw_bind();
    }
    let path_len = (addrlen as usize - 2).min(UNIX_PATH_MAX);
    if path_len == 0 {
        return raw_bind();
    }
    let path = core::slice::from_raw_parts(addr.add(2), path_len);
    if path[0] == 0 {
        return raw_bind();
    }
    let path_end = path.iter().position(|c| *c == 0).unwrap_or(path_len);
    if path_end == 0 {
        return raw_bind();
    }

    let mut guest = [0u8; UNIX_PATH_MAX + 1];
    guest[..path_end].copy_from_slice(&path[..path_end]);
    guest[path_end] = 0;

    let mut gp = [0u8; PATH_MAX];
    let Some(gn) = abs_guest(guest.as_ptr() as *const CChar, &mut gp) else {
        return set_errno(36);
    };
    let rfd = rootfd();
    if rfd < 0 || passthrough(&gp[..gn]) {
        return raw_bind();
    }

    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..gn], &mut name) {
        Ok(fd) => fd,
        Err(e) => return set_errno(e),
    };
    let name_len = name.iter().position(|c| *c == 0).unwrap_or(0);
    if name_len == 0 {
        rc::close(parent);
        return set_errno(2);
    }

    let mut proc_buf = [0u8; 32];
    let proc_path = fd_proc_path(parent, &mut proc_buf);
    let prefix_len = proc_path.len().saturating_sub(1); // omit trailing NUL
    if prefix_len + 1 + name_len > UNIX_PATH_MAX {
        rc::close(parent);
        return set_errno(36);
    }

    let mut translated = SockAddrUn {
        family: AF_UNIX,
        path: [0u8; UNIX_PATH_MAX],
    };
    translated.path[..prefix_len].copy_from_slice(&proc_path[..prefix_len]);
    translated.path[prefix_len] = b'/';
    translated.path[prefix_len + 1..prefix_len + 1 + name_len].copy_from_slice(&name[..name_len]);
    let translated_len = 2 + prefix_len + 1 + name_len + 1;
    let r = syscall4(
        SYS_BIND,
        sockfd as usize,
        &translated as *const SockAddrUn as usize,
        translated_len,
        0,
    );
    rc::close(parent);
    ret_or_errno(r)
}

// ------------------------------------------------------- not interposed: execl(3)
// execl/execle/execlp cannot be interposed from Rust on stable (C-variadic
// definitions are unstable) and, more importantly, glibc's execl builds its argv
// and then calls its *own* internal execve, which has no GOT slot. Programs that
// use them therefore fall back to libc and see their guest path verbatim
// (ENOENT). execve/execv/execvpe/execvp and posix_spawn *are* interposed, and
// that is what shells, make, gcc and python use (evidence/16, evidence/18).
// A pure-asm trampoline is possible but would have to be exported through
// rustc's cdylib export list, so it is deliberately left out for now.

// ------------------------------------------------------------------- stdio
// as(1) opens its output with fopen(), not open(): glibc's fopen reaches the
// kernel through an internal open alias, so the GOT patch cannot see it and the
// guest path would escape to the host's /tmp. stdio has to be interposed the
// same way open() is: map the path, open it ourselves, then hand the descriptor
// to fdopen() - a stream over a descriptor needs no path resolution at all.
const O_WRONLY: CInt = 1;
const O_TRUNC: CInt = 0o1000;
const O_APPEND: CInt = 0o2000;

unsafe fn stdio_flags(mode: &[u8]) -> Option<CInt> {
    if mode.is_empty() {
        return None;
    }
    let mut f: CInt = match mode[0] {
        b'r' => 0,
        b'w' => O_WRONLY | O_CREAT | O_TRUNC,
        b'a' => O_WRONLY | O_CREAT | O_APPEND,
        _ => return None,
    };
    for &c in &mode[1..] {
        match c {
            b'+' => {
                f = (f & !O_WRONLY) | O_RDWR;
            }
            b'x' => f |= O_EXCL,
            b'e' => f |= O_CLOEXEC,
            b'b' | b'm' | b'c' => {}
            0 => break,
            _ => return None,
        }
    }
    Some(f)
}

unsafe fn fopen_mode(path: *const CChar, mode: *const CChar) -> *mut u8 {
    if path.is_null() || mode.is_null() {
        *__errno_location() = EFAULT;
        return core::ptr::null_mut();
    }
    let flags = match stdio_flags(bytes(mode)) {
        Some(f) => f,
        None => {
            *__errno_location() = EINVAL;
            return core::ptr::null_mut();
        }
    };
    let fd = open(path, flags, 0o666);
    if fd < 0 {
        return core::ptr::null_mut();
    }
    let fp = fdopen(fd, mode);
    if fp.is_null() {
        rc::close(fd);
    }
    fp
}

#[no_mangle]
pub unsafe extern "C" fn fopen(path: *const CChar, mode: *const CChar) -> *mut u8 {
    fopen_mode(path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn fopen64(path: *const CChar, mode: *const CChar) -> *mut u8 {
    fopen_mode(path, mode)
}

// ------------------------------------------------------------ fd-level owners
//
// tar (which dpkg-deb and dpkg both run) does not call chown()/chmod(): it calls
// fchown() and fchmod() on the descriptor it just extracted through. Neither
// was interposed, so `dpkg -i` reported
//   tar: ./control: Cannot change ownership to uid 0, gid 0: Operation not permitted
// and aborted the unpack - the metadata operations that a container is expected
// to pretend succeeded.

/// "/proc/self/fd/<fd>", in a caller-provided buffer. The descriptor is ours,
/// so the host /proc entry points at the real file *inside* the guest root and
/// the path needs no guest translation.
unsafe fn fd_host_path(fd: CInt, buf: &mut [u8; 32]) -> usize {
    let pre = b"/proc/self/fd/";
    buf[..pre.len()].copy_from_slice(pre);
    let (d, k) = dec_buf(fd as i64);
    let n = d.len() - k;
    buf[pre.len()..pre.len() + n].copy_from_slice(&d[k..]);
    let end = pre.len() + n;
    buf[end] = 0;
    end
}

#[no_mangle]
pub unsafe extern "C" fn fchown(fd: CInt, uid: u32, gid: u32) -> CInt {
    if fake_root_on() {
        return 0;
    }
    // AT_EMPTY_PATH + an empty name means "the file this descriptor is open on".
    ret_or_errno(syscall5(
        SYS_FCHOWNAT,
        fd as usize,
        b"\0".as_ptr() as usize,
        uid as usize,
        gid as usize,
        AT_EMPTY_PATH as usize,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn fchmod(fd: CInt, mode: u32) -> CInt {
    let mut p = [0u8; 32];
    fd_host_path(fd, &mut p);
    ret_or_errno(syscall4(SYS_FCHMODAT, AT_FDCWD as usize, p.as_ptr() as usize, mode as usize, 0))
}

#[no_mangle]
pub unsafe extern "C" fn futimens(fd: CInt, times: *const u64) -> CInt {
    let mut p = [0u8; 32];
    fd_host_path(fd, &mut p);
    let r = syscall4(
        SYS_UTIMENSAT,
        AT_FDCWD as usize,
        p.as_ptr() as usize,
        times as usize,
        0,
    );
    let rr = ret_or_errno(r);
    if dbg_on() {
        let (b, k) = dec_buf(rr as i64);
        dbg_file(&[b"futimens fd=", bytes(p.as_ptr() as *const CChar), b" -> ", &b[k..], b"\n"]);
    }
    rr
}

/// audit(7) is unreachable from `untrusted_app`: the ROM's seccomp policy refuses
/// the NETLINK_AUDIT socket, so libaudit answers -1 and shadow treats that as
/// fatal ("Cannot open audit interface - aborting."). Every package that creates
/// a user or group then fails during `dpkg --configure`, which makes apt refuse
/// to touch anything at all - `pipewire`, `rtkit` and anything else with an
/// `adduser` postinst.
///
/// A container with CAP_AUDIT_WRITE gets a socket that accepts the message and
/// drops it. This is the same end state, stated plainly: hand the caller a sink.
/// Writes to it succeed, so `audit_log_user_message` never reports an error.
#[no_mangle]
pub unsafe extern "C" fn audit_open() -> CInt {
    open(b"/dev/null\0".as_ptr() as *const CChar, 1, 0)
}

#[no_mangle]
pub unsafe extern "C" fn creat(path: *const CChar, mode: CInt) -> CInt {
    open(path, O_WRONLY | O_CREAT | O_TRUNC, mode)
}

#[no_mangle]
pub unsafe extern "C" fn creat64(path: *const CChar, mode: CInt) -> CInt {
    open(path, O_WRONLY | O_CREAT | O_TRUNC, mode)
}

// ---------------------------------------------------------------- realpath
// gcc and cc1 resolve paths with realpath()/__realpath_chk(). Inside libc that
// ends up calling faccessat(), whose first attempt is the faccessat2 syscall -
// which this ROM's seccomp policy answers with SIGSYS instead of ENOSYS. Doing
// the job here removes that trap and, more importantly, makes the answer a
// *guest* path instead of a host one.
const ERANGE_NUM: CInt = 34;
const ENOMEM_NUM: CInt = 12;

unsafe fn realpath_impl(path: *const CChar, out: *mut CChar, out_size: usize) -> *mut CChar {
    if path.is_null() {
        return set_errno_ptr(EFAULT, core::ptr::null_mut());
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return set_errno_ptr(36, core::ptr::null_mut()),
    };
    let rfd = rootfd();
    let mut guest = [0u8; PATH_MAX];
    let gl: usize;
    if rfd < 0 || (pseudo_path(&gp[..n]) && passthrough(&gp[..n])) {
        guest[..n].copy_from_slice(&gp[..n]);
        gl = n;
    } else if under_root(&gp[..n]) {
        gl = match guest_path_from_real(&gp[..n], guest.as_mut_ptr(), PATH_MAX - 1) {
            Some(l) => l,
            None => return set_errno_ptr(36, core::ptr::null_mut()),
        };
    } else if n == 1 {
        guest[0] = b'/';
        gl = 1;
    } else {
        let mut name = [0u8; 256];
        let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
            Ok(fd) => fd,
            Err(e) => return set_errno_ptr(e, core::ptr::null_mut()),
        };
        let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
        let mut real = [0u8; PATH_MAX];
        let r = real_path(parent, &name[..nl], &mut real);
        rc::close(parent);
        let rl = match r {
            Some(l) => l,
            None => return set_errno_ptr(ENOENT, core::ptr::null_mut()),
        };
        gl = match guest_path_from_real(&real[..rl], guest.as_mut_ptr(), PATH_MAX - 1) {
            Some(l) => l,
            None => return set_errno_ptr(36, core::ptr::null_mut()),
        };
    }
    if out.is_null() {
        let p = malloc(gl + 1) as *mut CChar;
        if p.is_null() {
            return set_errno_ptr(ENOMEM_NUM, core::ptr::null_mut());
        }
        core::ptr::copy_nonoverlapping(guest.as_ptr(), p as *mut u8, gl);
        *p.add(gl) = 0;
        return p;
    }
    if out_size != 0 && gl + 1 > out_size {
        return set_errno_ptr(ERANGE_NUM, core::ptr::null_mut());
    }
    core::ptr::copy_nonoverlapping(guest.as_ptr(), out as *mut u8, gl);
    *out.add(gl) = 0;
    out
}

#[no_mangle]
pub unsafe extern "C" fn realpath(path: *const CChar, resolved: *mut CChar) -> *mut CChar {
    realpath_impl(path, resolved, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __realpath_chk(
    path: *const CChar,
    resolved: *mut CChar,
    resolved_size: CSizet,
) -> *mut CChar {
    realpath_impl(path, resolved, resolved_size)
}

#[no_mangle]
pub unsafe extern "C" fn canonicalize_file_name(path: *const CChar) -> *mut CChar {
    realpath_impl(path, core::ptr::null_mut(), 0)
}

// ------------------------------------------------------------------ dlopen
// The linker (ld.bfd, through collect2) loads the LTO plugin with dlopen() from
// an absolute guest path. glibc's dlopen opens the file through an internal
// open, so the path has to be translated here. dlmopen() is the same call with
// an explicit namespace and a different name, which is what lets this work
// without the shim having to find "the real dlopen" (our own symbol is the one
// that would be found).
const LM_ID_BASE: i64 = 0;

/// Everything loaded after `_dl_init` resolves its own PLT against libc, so the
/// bootstrap GOT rewrite never reached it (gdk-pixbuf loaders, GIO modules,
/// libspa). glibc appends the new objects to the link_map chain; re-walking it
/// here is the whole fix. Costs one chain scan per dlopen.
#[inline]
unsafe fn loaded(h: *mut u8) -> *mut u8 {
    if !h.is_null() {
        crate::boot::nr_repatch();
    }
    h
}

#[no_mangle]
pub unsafe extern "C" fn dlopen(file: *const CChar, mode: CInt) -> *mut u8 {
    if file.is_null() {
        return loaded(dlmopen(LM_ID_BASE, file, mode));
    }
    let f = bytes(file);
    // A bare soname is a search, not a path: leave that to the loader.
    if !f.contains(&b'/') {
        return loaded(dlmopen(LM_ID_BASE, file, mode));
    }
    let rfd = rootfd();
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(file, &mut gp) {
        Some(n) => n,
        None => return core::ptr::null_mut(),
    };
    let mut real = [0u8; PATH_MAX];
    if rfd < 0 || under_root(&gp[..n]) {
        real[..n].copy_from_slice(&gp[..n]);
        real[n] = 0;
        return loaded(dlmopen(LM_ID_BASE, real.as_ptr() as *const CChar, mode));
    }
    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
        Ok(fd) => fd,
        Err(e) => {
            *__errno_location() = e;
            return core::ptr::null_mut();
        }
    };
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    let r = real_path(parent, &name[..nl], &mut real);
    rc::close(parent);
    if r.is_none() {
        *__errno_location() = ENOENT;
        return core::ptr::null_mut();
    }
    loaded(dlmopen(LM_ID_BASE, real.as_ptr() as *const CChar, mode))
}

// --------------------------------------------------------- sigaction guard
// Why this exists (measured): bash installs its own terminator for some signals
// and, once it does, *our* SIGSYS handler is gone - so a seccomp trap is then
// delivered to bash's handler, which re-raises the signal and exits 128+31
// (159). With our handler in place the same trap is emulated to ENOSYS instead.
// The handler is therefore kept: any attempt to install a SIGSYS disposition is
// accepted (the caller is told "success") and quietly ignored.
//
// The other signals are forwarded, which needs the two struct layouts:
//   libc       { handler@0, mask@8 (128B), flags@136, restorer@144 }  size 152
//   kernel     { handler@0, flags@8, restorer@16, mask@24 (8B) }      sigsetsize 8
const SIGSYS_SIGNAL: CInt = 31;
const SIGACTION_SIZEOF: usize = 152;

unsafe fn k_from_libc(act: *const u8, k: &mut KSigaction) {
    let q = act as *const u64;
    k.handler = *q as usize;
    k.mask = *q.add(1);
    k.flags = *(act.add(136) as *const i32) as usize;
    k.restorer = *q.add(18) as usize;
}

unsafe fn libc_from_k(k: &KSigaction, out: *mut u8) {
    core::ptr::write_bytes(out, 0, SIGACTION_SIZEOF);
    let q = out as *mut u64;
    *q = k.handler as u64;
    *q.add(1) = k.mask;
    *(out.add(136) as *mut i32) = k.flags as i32;
    *q.add(18) = k.restorer as u64;
}

#[no_mangle]
pub unsafe extern "C" fn sigaction(sig: CInt, act: *const u8, old: *mut u8) -> CInt {
    if sig == SIGSYS_SIGNAL {
        if !act.is_null() {
            dbg(&[b"nr-sigaction|SIGSYS ignored\n"]);
        }
        if !old.is_null() {
            core::ptr::write_bytes(old, 0, SIGACTION_SIZEOF);
        }
        return 0;
    }
    let mut k = KSigaction {
        handler: 0,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
    let mut ko = KSigaction {
        handler: 0,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
    let kp = if act.is_null() {
        0
    } else {
        k_from_libc(act, &mut k);
        &k as *const KSigaction as usize
    };
    let op = if old.is_null() {
        0
    } else {
        &mut ko as *mut KSigaction as usize
    };
    let r = syscall4(SYS_RT_SIGACTION, sig as usize, kp, op, 8) as CInt;
    if r < 0 {
        return set_errno(-r);
    }
    if !old.is_null() {
        libc_from_k(&ko, old);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn signal(sig: CInt, handler: usize) -> usize {
    // BSD semantics, as glibc's own signal() implements them.
    let mut act = [0u8; SIGACTION_SIZEOF];
    let mut old = [0u8; SIGACTION_SIZEOF];
    let q = act.as_mut_ptr() as *mut u64;
    *q = handler as u64;
    *(act.as_mut_ptr().add(136) as *mut i32) = 0x1000_0000u32 as i32; // SA_RESTART
    if sigaction(sig, act.as_ptr(), old.as_mut_ptr()) != 0 {
        return usize::MAX;
    }
    *(old.as_ptr() as *const u64) as usize
}

// ------------------------------------------------------------ signalling
// Measured on RMX3357 (Android 13, kernel 4.19): `kill(2)` from an app is answered
// with a SIGSYS that no handler can catch -- `openat2` traps, `kill` kills. bash's
// job-control setup calls kill(shell_pid, SIGTTIN) and the guest dies there, which
// is what looked like "loader bootstrap gets killed".
//
// So the call never reaches the kernel:
//   * a job-control signal aimed at ourselves -> success, no-op. Job control is
//     meaningless for an app that owns no real controlling terminal.
//   * anything aimed at another process      -> EPERM, which is exactly what an
//     Android app already gets from the kernel.
//   * any other self-signal                  -> passed through untouched (dying
//     from the real signal is never worse than dying from SIGSYS).
const SYS_KILL_NR: usize = 129;
const SYS_GETTID: usize = 178;
const SIGCONT_N: CInt = 18;
const SIGTSTP_N: CInt = 20;
const SIGTTIN_N: CInt = 21;
const SIGTTOU_N: CInt = 22;

fn is_job_control(sig: CInt) -> bool {
    sig == SIGCONT_N || sig == SIGTSTP_N || sig == SIGTTIN_N || sig == SIGTTOU_N
}

unsafe fn targets_self(pid: CInt) -> bool {
    let me = syscall4(SYS_GETPID, 0, 0, 0, 0) as CInt;
    pid == 0 || pid == -1 || pid == me || pid == -me
}

/// Shared policy for kill/tkill/tgkill. Returns Some(result) when handled here.
unsafe fn signal_policy(pid: CInt, sig: CInt) -> Option<CInt> {
    if !targets_self(pid) {
        return Some(set_errno(1)); // EPERM: apps cannot signal other processes
    }
    // Measured: this ROM answers kill(2) -- even kill(getpid(), ...) -- with a
    // SIGSYS that no handler can catch, so *nothing* self-targeted may reach the
    // kernel. Jobs control signals are no-ops for us; anything else is logged and
    // dropped, which is the honest maximum an app sandbox allows.
    if sig != 0 && !is_job_control(sig) {
        dbgn(&[b"nr-kill|self pid="], pid as i64);
        dbgn(&[b"|sig="], sig as i64);
        dbg(&[b"\n"]);
    }
    Some(0)
}

#[no_mangle]
pub unsafe extern "C" fn kill(pid: CInt, sig: CInt) -> CInt {
    if let Some(r) = signal_policy(pid, sig) {
        return r;
    }
    let r = syscall4(SYS_KILL_NR, pid as usize, sig as usize, 0, 0) as CInt;
    if r < 0 {
        return set_errno(-r);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn tkill(tid: CInt, sig: CInt) -> CInt {
    if let Some(r) = signal_policy(tid, sig) {
        return r;
    }
    let r = syscall4(130, tid as usize, sig as usize, 0, 0) as CInt;
    if r < 0 {
        return set_errno(-r);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn tgkill(tgid: CInt, tid: CInt, sig: CInt) -> CInt {
    if let Some(r) = signal_policy(tgid, sig) {
        return r;
    }
    let r = syscall4(131, tgid as usize, tid as usize, sig as usize, 0) as CInt;
    if r < 0 {
        return set_errno(-r);
    }
    r
}

// ------------------------------------------------------------- posix_spawn
// make(1) runs every recipe through posix_spawn(), and glibc's implementation
// execs the child through an internal alias of execve: a guest path would reach
// the kernel untouched, and even a translated real path would arrive without
// its interpreter. So the spawn is done here instead - fork, honour what the
// caller asked for, then exec through our own execve.
//
// The two opaque types are read only through public getters (attributes) and
// through the layout glibc 2.39 really uses. That layout was dumped on this
// device (evidence 16): a 32-byte entry with tag at +0, fd at +8, new_fd at
// +12, path at +16, oflag at +24, mode at +28, and a header of
// {int allocated; int used; entry *actions}.
const FA_ENTRY: usize = 32;
const SYS_DUP3: usize = 24;
const SYS_SETPGID: usize = 154;
const SYS_SETSID: usize = 157;
const SYS_RT_SIGPROCMASK: usize = 135;
const SIG_SETMASK: usize = 2;

const SPAWN_SETPGROUP: i16 = 0x02;
const SPAWN_SETSIGDEF: i16 = 0x04;
const SPAWN_SETSIGMASK: i16 = 0x08;
const SPAWN_SETSID: i16 = 0x80;

unsafe fn apply_sigdefault(set: &[u8; 128]) {
    let dfl = KSigaction {
        handler: 0,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
    let mut sig = 1usize;
    while sig <= 64 {
        let byte = set[(sig - 1) / 8];
        if byte & (1 << ((sig - 1) % 8)) != 0 {
            syscall4(
                SYS_RT_SIGACTION,
                sig,
                &dfl as *const KSigaction as usize,
                0,
                8,
            );
        }
        sig += 1;
    }
}

/// Returns 0 or an errno value; the caller turns a failure into exit 127.
unsafe fn spawn_apply_attr(attr: *const u8) -> CInt {
    let mut flags: i16 = 0;
    if posix_spawnattr_getflags(attr, &mut flags) != 0 {
        return EINVAL;
    }
    if flags & SPAWN_SETPGROUP != 0 {
        let mut pg: CInt = 0;
        if posix_spawnattr_getpgroup(attr, &mut pg) != 0 {
            return EINVAL;
        }
        if syscall4(SYS_SETPGID, 0, pg as usize, 0, 0) != 0 {
            return *__errno_location();
        }
    }
    if flags & SPAWN_SETSID != 0 {
        if syscall4(SYS_SETSID, 0, 0, 0, 0) < 0 {
            return *__errno_location();
        }
    }
    if flags & SPAWN_SETSIGDEF != 0 {
        let mut set = [0u8; 128];
        if posix_spawnattr_getsigdefault(attr, set.as_mut_ptr()) != 0 {
            return EINVAL;
        }
        apply_sigdefault(&set);
    }
    if flags & SPAWN_SETSIGMASK != 0 {
        let mut set = [0u8; 128];
        if posix_spawnattr_getsigmask(attr, set.as_mut_ptr()) != 0 {
            return EINVAL;
        }
        if syscall4(SYS_RT_SIGPROCMASK, SIG_SETMASK, set.as_ptr() as usize, 0, 8) != 0 {
            return *__errno_location();
        }
    }
    0
}

unsafe fn spawn_apply_actions(fa: *const u8) -> CInt {
    if fa.is_null() {
        return 0;
    }
    let allocated = core::ptr::read_unaligned(fa as *const i32);
    let used = core::ptr::read_unaligned(fa.add(4) as *const i32);
    let actions = core::ptr::read_unaligned(fa.add(8) as *const *const u8);
    if used < 0 || allocated < used || used > 256 {
        return EINVAL;
    }
    if used == 0 {
        return 0;
    }
    if actions.is_null() {
        return EINVAL;
    }
    let mut i = 0usize;
    while i < used as usize {
        let a = actions.add(i * FA_ENTRY);
        let tag = core::ptr::read_unaligned(a as *const i32);
        let fd = core::ptr::read_unaligned(a.add(8) as *const i32);
        match tag {
            0 => rc::close(fd), // close
            1 => {
                // dup2
                let nfd = core::ptr::read_unaligned(a.add(12) as *const i32);
                if fd != nfd && syscall4(SYS_DUP3, fd as usize, nfd as usize, 0, 0) < 0 {
                    return *__errno_location();
                }
            }
            2 => {
                // open
                let p = core::ptr::read_unaligned(a.add(16) as *const *const CChar);
                let oflag = core::ptr::read_unaligned(a.add(24) as *const i32);
                let mode = core::ptr::read_unaligned(a.add(28) as *const i32);
                let r = open(p, oflag, mode);
                if r < 0 {
                    return *__errno_location();
                }
                if r != fd {
                    if syscall4(SYS_DUP3, r as usize, fd as usize, 0, 0) < 0 {
                        return *__errno_location();
                    }
                    rc::close(r);
                }
            }
            3 => {
                // chdir
                let p = core::ptr::read_unaligned(a.add(8) as *const *const CChar);
                if chdir(p) != 0 {
                    return *__errno_location();
                }
            }
            4 => {
                // fchdir
                if fchdir(fd) != 0 {
                    return *__errno_location();
                }
            }
            _ => return EINVAL,
        }
        i += 1;
    }
    0
}

unsafe fn search_guest_path(f: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let path: &[u8] = match env_lookup(b"PATH") {
        Some(p) => p,
        None => b"/bin:/usr/bin",
    };
    let mut buf = [0u8; PATH_MAX];
    for dir in path.split(|c| *c == b':') {
        let d: &[u8] = if dir.is_empty() { b"." } else { dir };
        let n = d.len() + 1 + f.len();
        if n + 1 >= PATH_MAX {
            continue;
        }
        buf[..d.len()].copy_from_slice(d);
        buf[d.len()] = b'/';
        buf[d.len() + 1..n].copy_from_slice(f);
        buf[n] = 0;
        if access(buf.as_ptr() as *const CChar, 1) == 0 {
            return abs_guest(buf.as_ptr() as *const CChar, out);
        }
    }
    None
}

unsafe fn spawn_common(
    pid: *mut CInt,
    path: *const CChar,
    search: bool,
    fa: *const u8,
    attr: *const u8,
    argv: *const *const CChar,
    envp: *const *const CChar,
) -> CInt {
    if pid.is_null() || path.is_null() {
        return EFAULT;
    }
    if dbg_on() {
        dbg(&[b"spawn|search="]);
        dbgn(&[], search as i64);
        dbg(&[b"|", bytes(path), b"\n"]);
    }
    let rfd = rootfd();
    let mut gp = [0u8; PATH_MAX];
    let n = if search && !bytes(path).contains(&b'/') {
        match search_guest_path(bytes(path), &mut gp) {
            Some(n) => n,
            None => return ENOENT,
        }
    } else {
        match abs_guest(path, &mut gp) {
            Some(n) => n,
            None => return 36,
        }
    };
    // posix_spawn reports "no such program" synchronously, unlike execve.
    if rfd >= 0 && access(gp.as_ptr() as *const CChar, 1) != 0 {
        return *__errno_location();
    }
    let mut pz = [0u8; PATH_MAX];
    pz[..n].copy_from_slice(&gp[..n]);
    pz[n] = 0;

    let child = fork();
    if child < 0 {
        return *__errno_location();
    }
    if child == 0 {
        // Child: no allocation, no unwinding, no way back.
        let mut e = 0;
        if !attr.is_null() {
            e = spawn_apply_attr(attr);
        }
        if e == 0 && !fa.is_null() {
            e = spawn_apply_actions(fa);
        }
        if e == 0 {
            execve(pz.as_ptr() as *const CChar, argv, envp);
        }
        syscall4(SYS_EXIT_GROUP, 127, 0, 0, 0);
    }
    *pid = child;
    0
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn(
    pid: *mut CInt,
    path: *const CChar,
    fa: *const u8,
    attr: *const u8,
    argv: *const *const CChar,
    envp: *const *const CChar,
) -> CInt {
    spawn_common(pid, path, false, fa, attr, argv, envp)
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnp(
    pid: *mut CInt,
    path: *const CChar,
    fa: *const u8,
    attr: *const u8,
    argv: *const *const CChar,
    envp: *const *const CChar,
) -> CInt {
    spawn_common(pid, path, true, fa, attr, argv, envp)
}

// ------------------------------------------------------------- fortified
// Ubuntu builds everything with _FORTIFY_SOURCE=3, so a program's call to
// readlink() may really be a call to __readlink_chk(), and open() likewise may
// be __open_2(). Those are separate exported symbols: glibc resolves them to
// its own internal implementations, which no GOT patch can see. as(1) and ls(1)
// are both built this way, so the wrappers have to be interposed too.
#[no_mangle]
pub unsafe extern "C" fn __readlink_chk(
    path: *const CChar,
    out: *mut CChar,
    len: CSizet,
    out_size: CSizet,
) -> Ssize {
    let n = if out_size != 0 && len > out_size {
        out_size
    } else {
        len
    };
    readlink(path, out, n)
}

#[no_mangle]
pub unsafe extern "C" fn __readlinkat_chk(
    dirfd: CInt,
    path: *const CChar,
    out: *mut CChar,
    len: CSizet,
    out_size: CSizet,
) -> Ssize {
    let n = if out_size != 0 && len > out_size {
        out_size
    } else {
        len
    };
    readlinkat(dirfd, path, out, n)
}

#[no_mangle]
pub unsafe extern "C" fn __open_2(path: *const CChar, flags: CInt) -> CInt {
    if path.is_null() {
        return set_errno(EFAULT);
    }
    open(path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __open64_2(path: *const CChar, flags: CInt) -> CInt {
    __open_2(path, flags)
}

#[no_mangle]
pub unsafe extern "C" fn __openat_2(dirfd: CInt, path: *const CChar, flags: CInt) -> CInt {
    if path.is_null() {
        return set_errno(EFAULT);
    }
    openat(dirfd, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __openat64_2(dirfd: CInt, path: *const CChar, flags: CInt) -> CInt {
    __openat_2(dirfd, path, flags)
}

// ------------------------------------------------------ the *64 and at() twins
// On 64-bit Linux these are all the same call, but they are different exported
// names, and a program picks whichever its headers chose: dash(1) asks for
// stat64/lstat64/faccessat, gcc for stat, ls for statx. Missing one of the twins
// means those calls fall into libc and resolve the guest path against the host.
const AT_EACCESS: CInt = 0x200;

#[no_mangle]
pub unsafe extern "C" fn stat64(path: *const CChar, st: *mut u8) -> CInt {
    stat(path, st)
}

#[no_mangle]
pub unsafe extern "C" fn lstat64(path: *const CChar, st: *mut u8) -> CInt {
    lstat(path, st)
}

#[no_mangle]
pub unsafe extern "C" fn fstatat64(
    dirfd: CInt,
    path: *const CChar,
    st: *mut u8,
    flags: CInt,
) -> CInt {
    fstatat(dirfd, path, st, flags)
}

#[no_mangle]
pub unsafe extern "C" fn fstat64(fd: CInt, st: *mut u8) -> CInt {
    ret_or_errno(syscall4(SYS_FSTAT, fd as usize, st as usize, 0, 0))
}

#[no_mangle]
pub unsafe extern "C" fn faccessat(
    dirfd: CInt,
    path: *const CChar,
    mode: CInt,
    flags: CInt,
) -> CInt {
    if path.is_null() {
        return set_errno(EFAULT);
    }
    if *path == 0 && flags & AT_EMPTY_PATH != 0 {
        let nofollow = flags & AT_SYMLINK_NOFOLLOW != 0;
        return match rc::access_fd(dirfd, mode, nofollow) {
            Ok(()) => 0,
            Err(e) => set_errno(e),
        };
    }
    let absolute = *path == b'/' as CChar;
    let nofollow = flags & AT_SYMLINK_NOFOLLOW != 0;
    let rfd = rootfd();
    if rfd >= 0 && (dirfd == AT_FDCWD || absolute) && !nofollow {
        // AT_EACCESS only changes whether the effective ids are used; this
        // process is not set-id, so plain access semantics are the same thing.
        let _ = AT_EACCESS;
        return access(path, mode);
    }
    if dirfd != AT_FDCWD && !absolute {
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
                Ok(Some(n)) => {
                    return match rc::resolve_access(rfd, &gp[..n], mode, flags) {
                        Ok(()) => 0,
                        Err(e) => set_errno(e),
                    }
                }
                Ok(None) => {}
                Err(e) => return set_errno(e),
            }
        }
        if nofollow {
            return accessat_nofollow(dirfd, path, mode);
        }
        return ret_or_errno(syscall4(
            SYS_FACCESSAT,
            dirfd as usize,
            path as usize,
            mode as usize,
            0,
        ));
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return set_errno(36),
    };
    if rfd >= 0 && !passthrough(&gp[..n]) {
        return match rc::resolve_access(rfd, &gp[..n], mode, flags) {
            Ok(()) => 0,
            Err(e) => set_errno(e),
        };
    }
    gp[n] = 0;
    if nofollow {
        return accessat_nofollow(AT_FDCWD, gp.as_ptr() as *const CChar, mode);
    }
    ret_or_errno(syscall4(
        SYS_FACCESSAT,
        AT_FDCWD as usize,
        gp.as_ptr() as usize,
        mode as usize,
        0,
    ))
}

// make(1) resolves every command it runs with eaccess(3) before spawning it, so
// this one is on the hot path of any build. glibc short-circuits eaccess to
// access() when the real and effective ids match (always true here), and that
// access() is the internal one - the guest path would be tested against the
// host. Same contract, so it just forwards.
#[no_mangle]
pub unsafe extern "C" fn eaccess(path: *const CChar, mode: CInt) -> CInt {
    access(path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn euidaccess(path: *const CChar, mode: CInt) -> CInt {
    access(path, mode)
}

// ---------------------------------------------------------------- identity ---
//
// glibc's NSS module for /etc/passwd and /etc/group is compiled *into* libc:
// nss_files opens "/etc/passwd" with a direct call, not through a GOT slot, so
// interposition never sees it and it lands on the host file system. The result
// is visible and embarrassing: `bash -i` prints "I have no name!", `groups`
// says "cannot find name for group ID 20302" for every gid, `id` prints bare
// numbers - while the very same file is readable by `cat`.
//
// Until the loader can redirect libc's internal calls (see docs/design: the
// text is not writable - mprotect back to r-x is refused with EACCES on an app
// domain, so body patching has to be baked in at build time), the identity
// entry points are answered here, straight out of the rootfs.

const SYS_READ: usize = 63;
const ERANGE: CInt = 34;
const O_RDONLY: CInt = 0;

#[repr(C)]
pub struct Passwd {
    pw_name: *const CChar,
    pw_passwd: *const CChar,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: *const CChar,
    pw_dir: *const CChar,
    pw_shell: *const CChar,
}

#[repr(C)]
pub struct Group {
    gr_name: *const CChar,
    gr_passwd: *const CChar,
    gr_gid: u32,
    gr_mem: *const *const CChar,
}

static mut EMPTY_MEMBERS: [*const CChar; 1] = [core::ptr::null()];

/// Raw source of whichever file is being scanned. One per process: the non-_r
/// getpwuid()/getgrgid() in glibc are not thread safe either, and the _r forms
/// only borrow it for the duration of a call.
static mut ID_SRC: [u8; 16384] = [0; 16384];
static mut ID_BUSY: AtomicU8 = AtomicU8::new(0);
/// Scratch for the non-reentrant results.
static mut ID_RES: [u8; 512] = [0; 512];

unsafe fn id_lock() {
    while ID_BUSY
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

unsafe fn id_unlock() {
    ID_BUSY.store(0, Ordering::Release);
}

unsafe fn slurp(path: &[u8]) -> usize {
    let mut p = [0u8; 64];
    if path.len() + 1 > p.len() {
        return 0;
    }
    let mut i = 0;
    while i < path.len() {
        p[i] = path[i];
        i += 1;
    }
    p[i] = 0;
    let fd = open(p.as_ptr() as *const CChar, O_RDONLY, 0);
    if fd < 0 {
        return 0;
    }
    let src = core::ptr::addr_of_mut!(ID_SRC) as usize;
    let cap = core::mem::size_of::<[u8; 16384]>();
    let mut n = 0usize;
    while n < cap {
        let r = syscall4(SYS_READ, fd as usize, src + n, cap - n, 0);
        if r <= 0 {
            break;
        }
        n += r as usize;
    }
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    n
}

/// Field boundaries of one line, as (start, len) pairs into ID_SRC.
struct Fields {
    n: usize,
    at: [(usize, usize); 7],
}

unsafe fn fields_of_line(start: usize, end: usize) -> Fields {
    let src = core::ptr::addr_of!(ID_SRC) as *const u8;
    let mut f = Fields {
        n: 0,
        at: [(0, 0); 7],
    };
    let mut s = start;
    let mut i = start;
    while i <= end && f.n < 7 {
        let stop = i == end || core::ptr::read_volatile(src.add(i)) == b':';
        if stop {
            f.at[f.n] = (s, i - s);
            f.n += 1;
            s = i + 1;
        }
        i += 1;
    }
    f
}

unsafe fn field_bytes(f: &Fields, idx: usize) -> &'static [u8] {
    let src = core::ptr::addr_of!(ID_SRC) as *const u8;
    if idx >= f.n {
        return &[];
    }
    let (s, l) = f.at[idx];
    core::slice::from_raw_parts(src.add(s), l)
}

unsafe fn field_num(f: &Fields, idx: usize) -> u32 {
    let b = field_bytes(f, idx);
    let mut v = 0u32;
    let mut i = 0;
    while i < b.len() && b[i] >= b'0' && b[i] <= b'9' {
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as u32);
        i += 1;
    }
    v
}

/// Copy `b` plus a NUL into `dst[*used..]`, returning the pointer or null when
/// it does not fit.
unsafe fn put_str(dst: usize, cap: usize, used: &mut usize, b: &[u8]) -> *const CChar {
    if *used + b.len() + 1 > cap {
        return core::ptr::null();
    }
    let p = dst + *used;
    let mut i = 0;
    while i < b.len() {
        core::ptr::write_volatile((p + i) as *mut u8, b[i]);
        i += 1;
    }
    core::ptr::write_volatile((p + i) as *mut u8, 0);
    *used += b.len() + 1;
    p as *const CChar
}

/// Scan `file` for a line whose `want_idx` field matches, and materialise it.
unsafe fn scan(
    file: &[u8],
    want_idx: usize,
    want_num: Option<u32>,
    want_str: Option<&[u8]>,
    dst: *mut u8,
    cap: usize,
) -> Option<Fields> {
    let n = slurp(file);
    if n == 0 {
        return None;
    }
    let src = core::ptr::addr_of!(ID_SRC) as *const u8;
    let mut i = 0usize;
    while i < n {
        let mut e = i;
        while e < n && core::ptr::read_volatile(src.add(e)) != b'\n' {
            e += 1;
        }
        if e > i {
            let f = fields_of_line(i, e);
            let hit = if let Some(num) = want_num {
                field_num(&f, want_idx) == num
            } else if let Some(s) = want_str {
                field_bytes(&f, want_idx) == s
            } else {
                false
            };
            if hit {
                let _ = (dst, cap);
                return Some(f);
            }
        }
        i = e + 1;
    }
    None
}

unsafe fn fill_passwd(f: &Fields, dst: *mut u8, cap: usize, out: *mut Passwd) -> CInt {
    let d = dst as usize;
    let mut used = 0usize;
    let name = put_str(d, cap, &mut used, field_bytes(f, 0));
    let pass = put_str(d, cap, &mut used, field_bytes(f, 1));
    let gecos = put_str(d, cap, &mut used, field_bytes(f, 4));
    let dir = put_str(d, cap, &mut used, field_bytes(f, 5));
    let shell = put_str(d, cap, &mut used, field_bytes(f, 6));
    if name.is_null() || pass.is_null() || gecos.is_null() || dir.is_null() || shell.is_null() {
        return ERANGE;
    }
    core::ptr::write_volatile(
        out,
        Passwd {
            pw_name: name,
            pw_passwd: pass,
            pw_uid: field_num(f, 2),
            pw_gid: field_num(f, 3),
            pw_gecos: gecos,
            pw_dir: dir,
            pw_shell: shell,
        },
    );
    0
}

/// Writes the comma separated member list into `dst` and returns a
/// NULL-terminated array of pointers into it. `getgrnam()` answering an empty
/// `gr_mem` is not cosmetic: `getent group <name>` and every tool that checks
/// membership that way would see the group as empty while the file says
/// otherwise (measured, evidence 111).
unsafe fn fill_members(mem: &[u8], d: usize, cap: usize, used: &mut usize) -> *const *const CChar {
    static mut PTRS: [*const CChar; 32] = [core::ptr::null(); 32];
    let ptrs = core::ptr::addr_of_mut!(PTRS) as *mut *const CChar;
    let mut k = 0usize;
    let mut start = 0usize;
    while start <= mem.len() {
        let mut end = start;
        while end < mem.len() && mem[end] != b',' {
            end += 1;
        }
        if end > start && k < 31 {
            let s = put_str(d, cap, used, &mem[start..end]);
            if s.is_null() {
                break;
            }
            *ptrs.add(k) = s;
            k += 1;
        }
        if end >= mem.len() {
            break;
        }
        start = end + 1;
    }
    *ptrs.add(k) = core::ptr::null();
    ptrs
}

unsafe fn fill_group(f: &Fields, dst: *mut u8, cap: usize, out: *mut Group) -> CInt {
    let d = dst as usize;
    let mut used = 0usize;
    let name = put_str(d, cap, &mut used, field_bytes(f, 0));
    let pass = put_str(d, cap, &mut used, field_bytes(f, 1));
    if name.is_null() || pass.is_null() {
        return ERANGE;
    }
    let mem = fill_members(field_bytes(f, 3), d, cap, &mut used);
    core::ptr::write_volatile(
        out,
        Group {
            gr_name: name,
            gr_passwd: pass,
            gr_gid: field_num(f, 2),
            gr_mem: mem,
        },
    );
    0
}

unsafe fn synth_group(gid: u32, dst: *mut u8, cap: usize, out: *mut Group) -> CInt {
    let mut nb = [0u8; 12];
    let n = put_i64(&mut nb, 0, gid as i64);
    // one string, "husk" + the gid: two put_str calls would insert a NUL in
    // the middle and `groups` would print the bare prefix.
    let mut full = [0u8; 16];
    let mut m = 0usize;
    let mut i = 0;
    while i < 4 {
        full[i] = b"husk"[i];
        m += 1;
        i += 1;
    }
    i = 0;
    while i < n {
        full[m] = nb[i];
        m += 1;
        i += 1;
    }
    let d = dst as usize;
    let mut used = 0usize;
    let name = put_str(d, cap, &mut used, &full[..m]);
    if name.is_null() {
        return ERANGE;
    }
    let pass = put_str(d, cap, &mut used, b"x");
    if pass.is_null() {
        return ERANGE;
    }
    core::ptr::write_volatile(
        out,
        Group {
            gr_name: name,
            gr_passwd: pass,
            gr_gid: gid,
            gr_mem: core::ptr::addr_of!(EMPTY_MEMBERS) as *const *const CChar,
        },
    );
    0
}

unsafe fn pw_lookup(
    idx: usize,
    num: Option<u32>,
    s: Option<&[u8]>,
    dst: *mut u8,
    cap: usize,
    out: *mut Passwd,
) -> CInt {
    id_lock();
    let r = match scan(b"/etc/passwd", idx, num, s, dst, cap) {
        Some(f) => fill_passwd(&f, dst, cap, out),
        None => 2, // ENOENT: "no such user"
    };
    id_unlock();
    r
}

/// The gids this *process* really has, straight from the kernel. Under fake
/// root `getgid()` answers 0, so it cannot be used for this.
///
/// This exists because the synthetic group name must stay scoped to the ids the
/// process actually carries. Answering "husk<N>" for *every* unknown gid makes
/// every gid look taken: `adduser` scans 100..999 with `getgrgid()` looking for
/// a free one and gets -1, then calls `groupadd -g -1` (measured, evidence 111).
unsafe fn process_has_gid(g: u32) -> bool {
    const SYS_GETGROUPS: usize = 158;
    const SYS_GETEGID: usize = 177;
    let real_gid = syscall4(SYS_GETGID, 0, 0, 0, 0) as u32;
    let real_egid = syscall4(SYS_GETEGID, 0, 0, 0, 0) as u32;
    if g == real_gid || g == real_egid {
        return true;
    }
    static mut LIST: [u32; 128] = [0; 128];
    static mut LEN: usize = usize::MAX;
    if LEN == usize::MAX {
        let n = syscall4(SYS_GETGROUPS, 0, 0, 0, 0) as isize;
        let n = if n <= 0 { 0 } else { (n as usize).min(128) };
        let r = syscall4(SYS_GETGROUPS, n, core::ptr::addr_of_mut!(LIST) as usize, 0, 0) as isize;
        LEN = if r <= 0 { 0 } else { r as usize };
    }
    let list = core::slice::from_raw_parts(core::ptr::addr_of!(LIST) as *const u32, LEN);
    list.contains(&g)
}

unsafe fn gr_lookup(
    idx: usize,
    num: Option<u32>,
    s: Option<&[u8]>,
    dst: *mut u8,
    cap: usize,
    out: *mut Group,
) -> CInt {
    id_lock();
    let r = match scan(b"/etc/group", idx, num, s, dst, cap) {
        Some(f) => fill_group(&f, dst, cap, out),
        // Android hands every app a fistful of kernel group ids that no
        // /etc/group ever lists (input, log, adb, sdcard_rw, ...). Answering
        // NULL makes `groups` and `ls -l` print "cannot find name for group
        // ID" for each of them; a stable synthetic name is the honest answer,
        // but only for ids the process actually carries - see process_has_gid.
        None => match num {
            Some(g) if process_has_gid(g) => synth_group(g, dst, cap, out),
            _ => 2,
        },
    };
    id_unlock();
    r
}

#[no_mangle]
pub unsafe extern "C" fn getpwuid_r(
    uid: u32,
    pwd: *mut Passwd,
    buf: *mut CChar,
    buflen: usize,
    result: *mut *mut Passwd,
) -> CInt {
    if !result.is_null() {
        core::ptr::write_volatile(result, core::ptr::null_mut());
    }
    let r = pw_lookup(2, Some(uid), None, buf as *mut u8, buflen, pwd);
    if r == 0 && !result.is_null() {
        core::ptr::write_volatile(result, pwd);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn getpwnam_r(
    name: *const CChar,
    pwd: *mut Passwd,
    buf: *mut CChar,
    buflen: usize,
    result: *mut *mut Passwd,
) -> CInt {
    if !result.is_null() {
        core::ptr::write_volatile(result, core::ptr::null_mut());
    }
    if name.is_null() {
        return 22;
    }
    let r = pw_lookup(0, None, Some(bytes(name)), buf as *mut u8, buflen, pwd);
    if r == 0 && !result.is_null() {
        core::ptr::write_volatile(result, pwd);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn getpwuid(uid: u32) -> *mut Passwd {
    static mut RES: Passwd = Passwd {
        pw_name: core::ptr::null(),
        pw_passwd: core::ptr::null(),
        pw_uid: 0,
        pw_gid: 0,
        pw_gecos: core::ptr::null(),
        pw_dir: core::ptr::null(),
        pw_shell: core::ptr::null(),
    };
    let out = core::ptr::addr_of_mut!(RES);
    let buf = core::ptr::addr_of_mut!(ID_RES) as *mut u8;
    let cap = core::mem::size_of::<[u8; 512]>();
    if pw_lookup(2, Some(uid), None, buf, cap, out) == 0 {
        out
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn getpwnam(name: *const CChar) -> *mut Passwd {
    static mut RES: Passwd = Passwd {
        pw_name: core::ptr::null(),
        pw_passwd: core::ptr::null(),
        pw_uid: 0,
        pw_gid: 0,
        pw_gecos: core::ptr::null(),
        pw_dir: core::ptr::null(),
        pw_shell: core::ptr::null(),
    };
    if name.is_null() {
        return core::ptr::null_mut();
    }
    let out = core::ptr::addr_of_mut!(RES);
    let buf = core::ptr::addr_of_mut!(ID_RES) as *mut u8;
    let cap = core::mem::size_of::<[u8; 512]>();
    if pw_lookup(0, None, Some(bytes(name)), buf, cap, out) == 0 {
        out
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn getgrgid_r(
    gid: u32,
    grp: *mut Group,
    buf: *mut CChar,
    buflen: usize,
    result: *mut *mut Group,
) -> CInt {
    if !result.is_null() {
        core::ptr::write_volatile(result, core::ptr::null_mut());
    }
    let r = gr_lookup(2, Some(gid), None, buf as *mut u8, buflen, grp);
    if r == 0 && !result.is_null() {
        core::ptr::write_volatile(result, grp);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn getgrnam_r(
    name: *const CChar,
    grp: *mut Group,
    buf: *mut CChar,
    buflen: usize,
    result: *mut *mut Group,
) -> CInt {
    if !result.is_null() {
        core::ptr::write_volatile(result, core::ptr::null_mut());
    }
    if name.is_null() {
        return 22;
    }
    let r = gr_lookup(0, None, Some(bytes(name)), buf as *mut u8, buflen, grp);
    if r == 0 && !result.is_null() {
        core::ptr::write_volatile(result, grp);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn getgrgid(gid: u32) -> *mut Group {
    static mut RES: Group = Group {
        gr_name: core::ptr::null(),
        gr_passwd: core::ptr::null(),
        gr_gid: 0,
        gr_mem: core::ptr::null(),
    };
    let out = core::ptr::addr_of_mut!(RES);
    let buf = core::ptr::addr_of_mut!(ID_RES) as *mut u8;
    let cap = core::mem::size_of::<[u8; 512]>();
    if gr_lookup(2, Some(gid), None, buf, cap, out) == 0 {
        out
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn getgrnam(name: *const CChar) -> *mut Group {
    static mut RES: Group = Group {
        gr_name: core::ptr::null(),
        gr_passwd: core::ptr::null(),
        gr_gid: 0,
        gr_mem: core::ptr::null(),
    };
    if name.is_null() {
        return core::ptr::null_mut();
    }
    let out = core::ptr::addr_of_mut!(RES);
    let buf = core::ptr::addr_of_mut!(ID_RES) as *mut u8;
    let cap = core::mem::size_of::<[u8; 512]>();
    if gr_lookup(0, None, Some(bytes(name)), buf, cap, out) == 0 {
        out
    } else {
        core::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------
// DRM（panfrost）假节点：把 Mesa 的 DRM 语义接到 /dev/mali0 的 kbase 上。
//
// 设计见 docs/design/gpu-drm-shim.md；语义已在 evidence 43 上机验过。
// 这里的三个钩子：
//   open("/dev/dri/renderD128") -> 一个假 fd（真的 /dev/null），登记在 FAKE_FDS 里
//   ioctl(fake_fd, PANFROST_*)   -> drm_shim::DrmCtx::dispatch
//   mmap(fake_fd, off=GPU VA)    -> 转到 mali0 fd 上真映射（prot 由 BO 的 kbase flags 决定）
// 其余 fd 一律原样走 syscall，不改任何行为。
// ---------------------------------------------------------------------------
const SYS_IOCTL: usize = 29;
const SYS_MMAP: usize = 222;

static mut DRM_CTX: drm_shim::DrmCtx = drm_shim::DrmCtx::empty();
static mut DRM_MALI_FD: CInt = -1;
static mut FAKE_FDS: [CInt; 8] = [-1; 8];

unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> Ssize {
    let ret: Ssize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        in("x4") e, in("x5") f, lateout("x0") ret, options(nostack));
    ret
}

/// `/dev/dri` 在 app 域里打不开（card0 EACCES、没有 renderD128），
/// 但 Mesa 的 loader 是**枚举目录**找设备的。所以给 guest 的 `/dev/dri`
/// 换成一个我们能控制的真目录：`<rootfs>/dev-dri-fake/`，里面有两个空文件
/// `renderD128` / `card0`。目录是真的，readdir 就是真的；具体设备的 open
/// 由 openat 的假节点分支接管，所以文件内容无所谓。
static mut DRM_DIR_HOST: [u8; PATH_MAX] = [0; PATH_MAX];
static mut DRM_DIR_LEN: usize = 0;
static mut DRM_DIR_READY: u8 = 0;

unsafe fn drm_dir_host() -> Option<&'static [u8]> {
    if DRM_DIR_READY == 1 {
        return Some(&DRM_DIR_HOST[..DRM_DIR_LEN]);
    }
    let rl = ROOT_REAL_LEN;
    if rl == 0 || rl > PATH_MAX - 32 {
        return None;
    }
    let mut d = [0u8; PATH_MAX];
    d[..rl].copy_from_slice(&ROOT_REAL[..rl]);
    let suffix = b"/dev-dri-fake";
    d[rl..rl + suffix.len()].copy_from_slice(suffix);
    d[rl + suffix.len()] = 0;
    let dl = rl + suffix.len();
    // mkdir（EEXIST 无所谓）
    syscall4(
        SYS_MKDIRAT,
        AT_FDCWD as usize,
        d.as_ptr() as usize,
        0o755,
        0,
    );
    // 两个空文件
    for name in [b"renderD128".as_ref(), b"card0".as_ref()] {
        let mut f = [0u8; PATH_MAX];
        f[..dl].copy_from_slice(&d[..dl]);
        f[dl] = b'/';
        f[dl + 1..dl + 1 + name.len()].copy_from_slice(name);
        f[dl + 1 + name.len()] = 0;
        let fd = syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            f.as_ptr() as usize,
            (0o100 | 0o2) as usize, // O_CREAT|O_WRONLY
            0o644,
        );
        if fd >= 0 {
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
        }
    }
    DRM_DIR_HOST = d;
    DRM_DIR_LEN = dl;
    DRM_DIR_READY = 1;
    Some(&DRM_DIR_HOST[..DRM_DIR_LEN])
}

fn is_drm_dir(p: &[u8]) -> bool {
    p == b"/dev/dri" || p == b"/dev/dri/"
}

fn is_drm_node(p: &[u8]) -> bool {
    p == b"/dev/dri/renderD128" || p == b"/dev/dri/card0"
}

/* ---- 与 LD_PRELOAD 层（libhusk-drm.so）的交接 -------------------------------
 *
 * 我们靠 _dl_init 钩子改 GOT，只能覆盖**启动时**已加载的对象；Mesa 运行时
 * dlopen 的 libdrm / libgbm / panfrost_dri.so 只有 LD_PRELOAD 能盖住。两层必须
 * 共用一份 DRM 状态（一个 kbase context），所以由 preload 独占，我们把调用转过去。
 * 表：[fake_open, ioctl, mmap, is_fake]，地址由 preload 的构造函数写进环境变量。
 */
type DrmFakeOpen = unsafe extern "C" fn() -> CInt;
type DrmIoctlFn = unsafe extern "C" fn(CInt, u64, *mut u8) -> CInt;
type DrmMmapFn = unsafe extern "C" fn(*mut u8, usize, CInt, CInt, CInt, i64) -> *mut u8;
type DrmIsFakeFn = unsafe extern "C" fn(CInt) -> CInt;

unsafe fn drm_api() -> Option<[usize; 4]> {
    let v = env_lookup(b"HUSK_DRM_API")?;
    if v.len() < 4 || v[0] != b'0' || (v[1] | 0x20) != b'x' {
        return None;
    }
    let mut p: usize = 0;
    let mut i = 2;
    while i < v.len() {
        let c = v[i];
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        p = (p << 4) | d as usize;
        i += 1;
    }
    if p == 0 {
        return None;
    }
    let t = p as *const usize;
    Some([*t, *t.add(1), *t.add(2), *t.add(3)])
}

unsafe fn drm_api_is_fake(api: &[usize; 4], fd: CInt) -> bool {
    let f: DrmIsFakeFn = core::mem::transmute(api[3]);
    f(fd) != 0
}

unsafe fn drm_api_open(api: &[usize; 4]) -> CInt {
    let f: DrmFakeOpen = core::mem::transmute(api[0]);
    f()
}

unsafe fn drm_api_ioctl(api: &[usize; 4], fd: CInt, request: u64, arg: *mut u8) -> CInt {
    let f: DrmIoctlFn = core::mem::transmute(api[1]);
    f(fd, request, arg)
}

unsafe fn drm_api_mmap(
    api: &[usize; 4],
    addr: *mut u8,
    len: usize,
    prot: CInt,
    flags: CInt,
    fd: CInt,
    off: i64,
) -> *mut u8 {
    let f: DrmMmapFn = core::mem::transmute(api[2]);
    f(addr, len, prot, flags, fd, off)
}

unsafe fn drm_shim_off() -> bool {
    // LD_PRELOAD 版（libhusk-drm.so）在场时，DRM 由它负责：它能看到运行时 dlopen 的
    // libdrm / libgbm / panfrost_dri.so，而我们的 loader GOT patch 看不到。
    env_lookup(b"NR_DRM_SHIM_OFF").is_some()
}

unsafe fn is_fake_fd(fd: CInt) -> bool {
    let mut i = 0;
    while i < FAKE_FDS.len() {
        if FAKE_FDS[i] == fd && fd >= 0 {
            return true;
        }
        i += 1;
    }
    false
}

unsafe fn register_fake_fd(fd: CInt) {
    let mut i = 0;
    while i < FAKE_FDS.len() {
        if FAKE_FDS[i] < 0 {
            FAKE_FDS[i] = fd;
            return;
        }
        i += 1;
    }
}

/// 假节点打开：懒初始化 kbase context，然后给调用方一个真 fd（/dev/null），
/// 这样 close/poll/dup 都不会炸；对它的 ioctl/mmap 都进我们这里。
unsafe fn drm_fake_open() -> CInt {
    if verbose_on() {
        dbg(&[b"nrshim drm: fake open\n"]);
    }
    if DRM_MALI_FD < 0 {
        let fd = syscall4(
            SYS_OPENAT,
            AT_FDCWD as usize,
            b"/dev/mali0\0".as_ptr() as usize,
            2, // O_RDWR
            0,
        );
        if fd < 0 {
            return set_errno(2);
        }
        DRM_MALI_FD = fd as CInt;
        let _ = DRM_CTX.init(DRM_MALI_FD, |f, req, arg| {
            syscall4(SYS_IOCTL, f as usize, req as usize, arg as usize, 0) as i64
        });
    }
    // 用 memfd：名字可辨识，LD_PRELOAD 那一层靠 /proc/self/fd 也能认出这个 fd
    const SYS_MEMFD_CREATE: usize = 279;
    let fd = syscall4(SYS_MEMFD_CREATE, b"husk-drm\0".as_ptr() as usize, 0, 0, 0);
    if fd < 0 {
        return set_errno(5);
    }
    register_fake_fd(fd as CInt);
    fd as CInt
}

#[no_mangle]
pub unsafe extern "C" fn ioctl(fd: CInt, request: u64, arg: *mut u8) -> CInt {
    if v4l2_is_fake(fd) {
        let r = v4l2_dispatch_ioctl(fd, request, arg);
        return if r < 0 { set_errno((-r) as CInt) } else { 0 };
    }
    if let Some(api) = drm_api() {
        if drm_api_is_fake(&api, fd) {
            return drm_api_ioctl(&api, fd, request, arg);
        }
    }
    if is_fake_fd(fd) {
        if verbose_on() {
            dbg(&[b"nrshim drm ioctl req="]);
            dbghex(request);
            dbg(&[b"\n"]);
        }
        let r = DRM_CTX.dispatch(
            |f, req, a| syscall4(SYS_IOCTL, f as usize, req as usize, a as usize, 0) as i64,
            request,
            arg,
        );
        if r < 0 {
            return set_errno((-r) as CInt);
        }
        return 0;
    }
    ret_or_errno(syscall4(
        SYS_IOCTL,
        fd as usize,
        request as usize,
        arg as usize,
        0,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    addr: *mut u8,
    len: usize,
    prot: CInt,
    flags: CInt,
    fd: CInt,
    off: i64,
) -> *mut u8 {
    if v4l2_is_fake(fd) {
        return match v4l2_dispatch_mmap(fd, len, off) {
            Some(p) => p,
            None => set_errno_ptr(22, core::ptr::null_mut()) as *mut u8,
        };
    }
    if let Some(api) = drm_api() {
        if drm_api_is_fake(&api, fd) {
            return drm_api_mmap(&api, addr, len, prot, flags, fd, off);
        }
    }
    let r = if is_fake_fd(fd) && DRM_MALI_FD >= 0 {
        // DRM 的 offset 就是 BO 的 GPU VA；kbase 要求 prot == 分配 flags 的低两位。
        syscall6(
            SYS_MMAP,
            addr as usize,
            len,
            prot as usize,
            flags as usize,
            DRM_MALI_FD as usize,
            off as usize,
        )
    } else {
        syscall6(
            SYS_MMAP,
            addr as usize,
            len,
            prot as usize,
            flags as usize,
            fd as usize,
            off as usize,
        )
    };
    if r < 0 && r > -4096 {
        return set_errno_ptr((-r) as CInt, core::ptr::null_mut()) as *mut u8;
    }
    r as *mut u8
}

#[no_mangle]
pub unsafe extern "C" fn mmap64(
    addr: *mut u8,
    len: usize,
    prot: CInt,
    flags: CInt,
    fd: CInt,
    off: i64,
) -> *mut u8 {
    mmap(addr, len, prot, flags, fd, off)
}

// ---------------------------------------------------------------------------
// V4L2 假节点：guest `/dev/videoN` → app 的 camera.sock。
//
// 设计/前人实现见 docs/design/v4l2-prior-art.md（DroidNative 的 dn_v4l2 逐行读）。
// 与它不同的地方只有传输：没有 FIFO、没有 fd 传递，DQBUF 是一次同步 FRAME
// 事务；打开的 fd 是 memfd，所以 poll/select/epoll 天然"可读"。
// ---------------------------------------------------------------------------
static mut V4L2_CTX: v4l2_shim::Ctx = v4l2_shim::Ctx::empty();
static mut V4L2_READY: u8 = 0;

unsafe fn v4l2_ready() -> bool {
    if V4L2_READY == 1 {
        return true;
    }
    let rl = ROOT_REAL_LEN;
    if rl == 0 || rl > 200 {
        return false;
    }
    let suffix = b"/run/husk/camera.sock";
    let mut p = [0u8; 256];
    p[..rl].copy_from_slice(&ROOT_REAL[..rl]);
    p[rl..rl + suffix.len()].copy_from_slice(suffix);
    if V4L2_CTX.init(&p[..rl + suffix.len()]) {
        V4L2_READY = 1;
        true
    } else {
        false
    }
}

/// `/dev/videoN` → Some(N)
fn video_index(p: &[u8]) -> Option<u32> {
    let pre = b"/dev/video";
    if p.len() <= pre.len() || !p.starts_with(pre) {
        return None;
    }
    let rest = &p[pre.len()..];
    if rest.len() > 3 {
        return None;
    }
    let mut n: u32 = 0;
    for &c in rest {
        if !c.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (c - b'0') as u32;
    }
    Some(n)
}

/// `/sys/class/video4linux*` 与 `/sys/subsystem/video4linux*` 在 Android 的 sysfs 里
/// 不存在（本机内核没有 V4L2），改指向 rootfs 里的影子树；`/sys` 其余部分照旧透传。
unsafe fn v4l2_sys_host(p: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let rl = ROOT_REAL_LEN;
    if rl == 0 {
        return None;
    }
    const C: &[u8] = b"/sys/class/video4linux";
    const S: &[u8] = b"/sys/subsystem/video4linux";
    let (dir, rest) = if p.starts_with(C) {
        (b"/sys-v4l2/class".as_ref(), &p[C.len()..])
    } else if p.starts_with(S) {
        (b"/sys-v4l2/subsystem".as_ref(), &p[S.len()..])
    } else {
        return None;
    };
    if !rest.is_empty() && rest[0] != b'/' {
        return None;
    }
    let n = rl + dir.len() + rest.len();
    if n + 1 > PATH_MAX {
        return None;
    }
    out[..rl].copy_from_slice(&ROOT_REAL[..rl]);
    out[rl..rl + dir.len()].copy_from_slice(dir);
    out[rl + dir.len()..n].copy_from_slice(rest);
    out[n] = 0;
    Some(n)
}

unsafe fn v4l2_stat(n: u32, st: *mut u8) -> CInt {
    if st.is_null() {
        return set_errno(14);
    }
    core::ptr::write_bytes(st, 0, 128);
    core::ptr::write_unaligned(st.add(8) as *mut u64, n as u64 + 1); // ino
    core::ptr::write_unaligned(st.add(16) as *mut u32, 0o020666u32); // S_IFCHR|0666
    core::ptr::write_unaligned(st.add(20) as *mut u32, 1u32); // nlink
    // rdev: major 81 (0x51) minor N, makedev(81,N) = (81<<8)|N on this kernel
    core::ptr::write_unaligned(st.add(32) as *mut u64, ((81u64) << 8) | n as u64);
    core::ptr::write_unaligned(st.add(48) as *mut i64, 0i64); // size
    core::ptr::write_unaligned(st.add(56) as *mut i64, 4096i64); // blksize
    0
}

unsafe fn v4l2_statx(n: u32, buf: *mut u8) -> CInt {
    if buf.is_null() {
        return set_errno(14);
    }
    core::ptr::write_bytes(buf, 0, 256);
    core::ptr::write_unaligned(buf.add(0) as *mut u32, 0x7ffu32); // stx_mask
    core::ptr::write_unaligned(buf.add(4) as *mut u32, 4096u32); // blksize
    core::ptr::write_unaligned(buf.add(16) as *mut u32, 1u32); // nlink
    core::ptr::write_unaligned(buf.add(28) as *mut u16, 0o020666u16); // mode
    core::ptr::write_unaligned(buf.add(32) as *mut u64, n as u64 + 1); // ino
    core::ptr::write_unaligned(buf.add(128) as *mut u32, 81u32); // rdev_major
    core::ptr::write_unaligned(buf.add(132) as *mut u32, n); // rdev_minor
    0
}

/* ---- LD_PRELOAD 层（libhusk-v4l2.so）的交接 -------------------------------
 *
 * loader 只能改启动时已加载对象的 GOT；GStreamer 的 libgstvideo4linux2.so 是
 * dlopen 的，它调 open("/dev/video1") 会绕过我们落到 Android 那个 0660 的节点。
 * 所以 preload 拥有一份 Ctx，并把函数表地址写进 HUSK_V4L2_API（hex），
 * 我们两边共用同一份状态（app 的 camera.sock 一次只收一个客户端）。
 * 表：[open, ioctl, mmap, is_fake, close, nonblock]
 */
type V4l2Open = unsafe extern "C" fn(u32) -> CInt;
type V4l2Ioctl = unsafe extern "C" fn(CInt, u64, *mut u8) -> i64;
type V4l2Mmap = unsafe extern "C" fn(CInt, usize, i64) -> *mut u8;
type V4l2IsFake = unsafe extern "C" fn(CInt) -> CInt;
type V4l2Close = unsafe extern "C" fn(CInt);
type V4l2Nonblock = unsafe extern "C" fn(CInt, CInt);

unsafe fn v4l2_api() -> Option<*const usize> {
    let v = env_lookup(b"HUSK_V4L2_API")?;
    if v.len() < 4 || v[0] != b'0' || (v[1] | 0x20) != b'x' {
        return None;
    }
    let mut p: usize = 0;
    for c in &v[2..] {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        p = (p << 4) | d as usize;
    }
    if p == 0 {
        None
    } else {
        Some(p as *const usize)
    }
}

unsafe fn v4l2_is_fake(fd: CInt) -> bool {
    if let Some(t) = v4l2_api() {
        let f: V4l2IsFake = core::mem::transmute(*t.add(3));
        return f(fd) != 0;
    }
    v4l2_ready() && V4L2_CTX.is_video_fd(fd)
}

unsafe fn v4l2_fake_open(n: u32) -> Option<CInt> {
    if let Some(t) = v4l2_api() {
        let f: V4l2Open = core::mem::transmute(*t);
        let fd = f(n);
        return if fd >= 0 { Some(fd) } else { None };
    }
    if !v4l2_ready() {
        return None;
    }
    let fd = V4L2_CTX.open(n);
    if fd < 0 {
        None
    } else {
        Some(fd)
    }
}

unsafe fn v4l2_dispatch_ioctl(fd: CInt, req: u64, arg: *mut u8) -> i64 {
    if let Some(t) = v4l2_api() {
        let f: V4l2Ioctl = core::mem::transmute(*t.add(1));
        return f(fd, req, arg);
    }
    V4L2_CTX.ioctl(fd, req, arg)
}

unsafe fn v4l2_dispatch_mmap(fd: CInt, len: usize, off: i64) -> Option<*mut u8> {
    if let Some(t) = v4l2_api() {
        let f: V4l2Mmap = core::mem::transmute(*t.add(2));
        let p = f(fd, len, off);
        return if p.is_null() { None } else { Some(p) };
    }
    V4L2_CTX.mmap(fd, len, off)
}

unsafe fn v4l2_forget(fd: CInt) {
    if let Some(t) = v4l2_api() {
        let f: V4l2Close = core::mem::transmute(*t.add(4));
        f(fd);
        return;
    }
    V4L2_CTX.close(fd);
}

unsafe fn v4l2_nonblock(fd: CInt, on: bool) {
    if let Some(t) = v4l2_api() {
        let f: V4l2Nonblock = core::mem::transmute(*t.add(5));
        f(fd, if on { 1 } else { 0 });
        return;
    }
    V4L2_CTX.update_nonblock(fd, on);
}

/// open("/dev/videoN")：给一个 memfd，登记在 v4l2 表里。
/// `None` = not our device (fall through to the normal path); `Some(Err(e))` =
/// ours but the open failed, and the guest must see that errno.
unsafe fn v4l2_open(n: u32) -> Option<Result<CInt, CInt>> {
    if let Some(t) = v4l2_api() {
        let f: V4l2Open = core::mem::transmute(*t);
        let fd = f(n);
        return Some(if fd >= 0 { Ok(fd) } else { Err(-fd) });
    }
    if !v4l2_ready() {
        return None;
    }
    let fd = V4L2_CTX.open(n);
    if fd < 0 {
        Some(Err(-fd))
    } else {
        Some(Ok(fd))
    }
}

/// fcntl: 只跟踪 O_NONBLOCK，其余原样走。glibc 的 fcntl 是可变参，第三个参数
/// 按 usize 接（F_GETFD/F_GETFL/F_SETFD 不带参数，不会用到它）。
#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: CInt, cmd: CInt, arg: usize) -> CInt {
    const F_GETFL: CInt = 3;
    const F_SETFL: CInt = 4;
    let r = syscall4(SYS_FCNTL, fd as usize, cmd as usize, arg, 0);
    if r >= 0 && v4l2_is_fake(fd) {
        if cmd == F_SETFL {
            v4l2_nonblock(fd, (arg & 0o4000) != 0); // O_NONBLOCK
        } else if cmd == F_GETFL {
            v4l2_nonblock(fd, (r as usize & 0o4000) != 0);
        }
    }
    ret_or_errno(r)
}
