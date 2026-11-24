// LinuxHub Native Runtime — preload path shim v3
// no_std, zero DT_NEEDED. Absolute or cwd-relative guest paths are resolved
// through runtime-core against the rootfd (symlink-aware, ".." clamped), and
// execve re-enters the guest ld.so with a real absolute path. No ptrace,
// no chroot, no namespaces.

#![no_std]

mod boot;
mod plan;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, AtomicUsize, Ordering};
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
    fn realloc(ptr: *mut u8, size: CSizet) -> *mut u8;
    fn free(ptr: *mut u8);
    fn readdir(dirp: *mut u8) -> *mut u8;
    fn closedir(dirp: *mut u8) -> CInt;
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
// Worst case 8 entries of 1+3+127+1+127+1 = 260 bytes = 2080. The old 2048
// byte buffer made a full table truncate its last line, which parses back as
// a *different* mount.
const MNT_SAVE_MAX: usize = 4096;
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
// 0 = not loaded, 1 = one thread is reading the file, 2 = ready. The reader is
// recorded so a re-entrant call from that same thread (abs_guest() inside the
// read) does not wait for itself.
static MNT_STATE: AtomicU8 = AtomicU8::new(0);
static MNT_OWNER: AtomicI32 = AtomicI32::new(0);
// Guards table access only - never a path computation and never a syscall that
// can re-enter abs_guest(). Writers hold it across mutation + mnt_save().
static MNT_LOCK: AtomicBool = AtomicBool::new(false);

unsafe fn mnt_lock() {
    while MNT_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while MNT_LOCK.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
}

unsafe fn mnt_unlock() {
    MNT_LOCK.store(false, Ordering::Release);
}

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
const SYS_GETGROUPS: usize = 158;
const SYS_SETRESUID: usize = 147;
const SYS_SETRESGID: usize = 149;
const SYS_GETGID: usize = 176;
const SYS_SETREUID: usize = 145;
const SYS_SETREGID: usize = 143;
const SYS_GETRESUID: usize = 148;
const SYS_GETRESGID: usize = 150;
const SYS_RENAMEAT: usize = 38;
const SYS_RENAMEAT2: usize = 276;
const SYS_FCHMOD: usize = 52;
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

// There is no cached cwd: see cwd_into(). Two threads resolving paths at the
// same time used to share one buffer, so one could read what the other was in
// the middle of writing.

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
/// The same rule `recvmsg` states inline: glibc's wrappers hand callers -1 and
/// set errno, never a raw negative value. An interposed function that can fail
/// has to translate before it returns, otherwise the caller reads a stale errno.
/// Measured 2026-11-23: `ls -l` printed "ls: /tmp" with no reason at all, because
/// libselinux called getxattr() and got -61 (ENODATA) back as the return value
/// while errno stayed 0, so coreutils' `if (err) error (0, errno, "%s", name)`
/// fired with errno == 0.
unsafe fn ssize_or_errno(r: Ssize) -> Ssize {
    if r < 0 && r > -4096 {
        *__errno_location() = (-r) as CInt;
        -1
    } else {
        r
    }
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

static NAME_STATE: AtomicU8 = AtomicU8::new(0);

/// `comm` is inherited from the file the kernel executed, and for a guest
/// process that file is always the loader - so every process in the container
/// used to show up as "libld39.so" in `ps`/`top`/`htop`, and `pgrep -x bash`
/// never matched. The real program is the first argument that is not one of the
/// loader's own options, so take that basename and rename the process once.
unsafe fn name_once() {
    if NAME_STATE.load(Ordering::Relaxed) != 0 {
        return;
    }
    NAME_STATE.store(1, Ordering::Relaxed);
    const SYS_OPENAT: usize = 56;
    const SYS_READ: usize = 63;
    const SYS_CLOSE: usize = 57;
    const SYS_PRCTL: usize = 167;
    const PR_SET_NAME: usize = 15;
    let mut buf = [0u8; 2048];
    let fd = syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        b"/proc/self/cmdline\0".as_ptr() as usize,
        0, // O_RDONLY
        0,
    );
    if fd < 0 {
        return;
    }
    let n = syscall4(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0);
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    if n <= 0 {
        return;
    }
    let n = n as usize;
    let mut i = 0usize;
    let mut idx = 0usize;
    let mut skip_value = false;
    let mut prog: Option<(usize, usize)> = None;
    while i < n {
        let start = i;
        while i < n && buf[i] != 0 {
            i += 1;
        }
        let len = i - start;
        i += 1; // the NUL itself
        idx += 1;
        if idx == 1 {
            continue; // the loader
        }
        if skip_value {
            skip_value = false;
            continue; // value of --library-path
        }
        if len == 14 && &buf[start..start + 14] == b"--library-path" {
            skip_value = true;
            continue;
        }
        if len == 7 && &buf[start..start + 7] == b"--argv0" {
            skip_value = true;
            continue;
        }
        if len >= 2 && buf[start] == b'-' && buf[start + 1] == b'-' {
            continue; // any other loader option
        }
        if len > 0 {
            prog = Some((start, len));
            break;
        }
    }
    let (start, len) = match prog {
        Some(v) => v,
        None => return,
    };
    // basename, at most TASK_COMM_LEN-1 bytes, NUL terminated
    let mut base = start;
    let mut k = start;
    while k < start + len {
        if buf[k] == b'/' {
            base = k + 1;
        }
        k += 1;
    }
    let mut name = [0u8; 16];
    let mut j = 0usize;
    while base + j < start + len && j < 15 {
        name[j] = buf[base + j];
        j += 1;
    }
    if j == 0 {
        return;
    }
    syscall4(
        SYS_PRCTL,
        PR_SET_NAME,
        name.as_ptr() as usize,
        0,
        0,
    );
}

unsafe fn rootfd() -> CInt {
    name_once();
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

/// Copy the guest cwd into the caller's buffer and return its length. The
/// kernel is the only authority - a program can move its cwd with a raw
/// fchdir, or from another thread - and a per-call buffer means concurrent
/// path resolution cannot observe a half-written one.
unsafe fn cwd_into(out: &mut [u8; PATH_MAX]) -> usize {
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
        if let Some(g) = guest_path_from_real(&real[..rl], out.as_mut_ptr(), PATH_MAX) {
            if g > 0 && g < PATH_MAX {
                return g;
            }
        }
    }
    out[0] = b'/';
    1
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
        match mnt_apply(out, n) {
            Some(m) => n = m,
            None => break,
        }
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
    let mut cw = [0u8; PATH_MAX];
    let cl = cwd_into(&mut cw);
    let c = &cw[..cl];
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
    // /dev/mqueue is a filesystem the kernel would mount; there is none here.
    // The named-queue service lives inside the guest root (see the POSIX
    // mqueue section), so it must not be passed through to the host /dev,
    // where an app cannot even create the directory.
    const MQ_PREFIX: &[u8] = b"/dev/mqueue";
    if p == MQ_PREFIX
        || (p.len() > MQ_PREFIX.len()
            && &p[..MQ_PREFIX.len()] == MQ_PREFIX
            && p[MQ_PREFIX.len()] == b'/')
    {
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
    // the *host* root, where it does not exist and never will. Only called with
    // MNT_LOCK free - mnt_mkdir_dir() is the table-free twin used under it.
    mkdirat(AT_FDCWD, MNT_DIR.as_ptr() as *const CChar, 0o755);
}

/// Guest path -> real host path with raw calls only: no table lookup, so it is
/// safe inside MNT_LOCK and inside mnt_ensure().
unsafe fn mnt_real_path(gp: &[u8], out: &mut [u8; PATH_MAX]) -> Option<usize> {
    let rfd = rootfd();
    if rfd < 0 {
        return None;
    }
    let mut name = [0u8; 256];
    let parent = rc::resolve_parent_follow(rfd, gp, &mut name).ok()?;
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    let r = real_path(parent, &name[..nl], out);
    rc::close(parent);
    r
}

unsafe fn mnt_open_table(flags: CInt, mode: CInt) -> CInt {
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(MNT_FILE.as_ptr() as *const CChar, &mut gp) {
        Some(n) => n,
        None => return -1,
    };
    let mut real = [0u8; PATH_MAX];
    let rn = match mnt_real_path(&gp[..n], &mut real) {
        Some(rn) => rn,
        None => return -1,
    };
    real[rn] = 0;
    ret_or_errno(syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        real.as_ptr() as usize,
        flags as usize,
        mode as usize,
    ))
}

unsafe fn mnt_mkdir_dir() {
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(MNT_DIR.as_ptr() as *const CChar, &mut gp) {
        Some(n) => n,
        None => return,
    };
    let rfd = rootfd();
    if rfd < 0 {
        return;
    }
    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
        Ok(fd) => fd,
        Err(_) => return,
    };
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    syscall4(SYS_MKDIRAT, parent as usize, name.as_ptr() as usize, 0o755, 0);
    rc::close(parent);
}

/// Append `s`; false when it no longer fits. A truncated target parses back as
/// a different mount, so callers drop the entry instead of writing half a line.
unsafe fn mnt_pad(out: &mut [u8], at: &mut usize, s: &[u8]) -> bool {
    let room = out.len() - *at;
    if s.len() > room {
        return false;
    }
    out[*at..*at + s.len()].copy_from_slice(s);
    *at += s.len();
    true
}

/// One-shot read of /run/husk/mounts. Never re-enters: the file is opened with
/// mnt_open_table(), which does not consult the table.
unsafe fn mnt_read_file() {
    let fd = mnt_open_table(0, 0);
    if fd < 0 {
        return;
    }
    static mut BUF: [u8; 4096] = [0; 4096];
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
    mnt_lock();
    MNT_N = 0;
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
    mnt_unlock();
}

/// Make sure the table is in memory before anybody reads it. The loading thread
/// does not hold MNT_LOCK while reading (mnt_read_file takes it only to
/// publish), and a re-entrant call from that same thread returns instead of
/// spinning on itself.
unsafe fn mnt_ensure() {
    loop {
        match MNT_STATE.load(Ordering::Acquire) {
            2 => return,
            0 => {
                if MNT_STATE
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    let tid = syscall4(SYS_GETTID, 0, 0, 0, 0) as i32;
                    MNT_OWNER.store(tid, Ordering::Release);
                    mnt_read_file();
                    MNT_OWNER.store(0, Ordering::Release);
                    MNT_STATE.store(2, Ordering::Release);
                    return;
                }
            }
            _ => {
                if MNT_OWNER.load(Ordering::Acquire) == syscall4(SYS_GETTID, 0, 0, 0, 0) as i32 {
                    return;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Rewrite /run/husk/mounts. Callers hold MNT_LOCK across mutation + save, so
/// two writers cannot interleave; everything here is table-free on purpose.
unsafe fn mnt_save() {
    let mut out = [0u8; MNT_SAVE_MAX];
    let mut at = 0usize;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let start = at;
        let ok = mnt_pad(&mut out, &mut at, if e.tmpfs != 0 { b"t" } else { b"b" })
            && mnt_pad(&mut out, &mut at, if e.ro != 0 { b"\t1\t" } else { b"\t0\t" })
            && mnt_pad(&mut out, &mut at, &e.tgt[..e.tlen as usize])
            && mnt_pad(&mut out, &mut at, b"\t")
            && mnt_pad(&mut out, &mut at, &e.src[..e.slen as usize])
            && mnt_pad(&mut out, &mut at, b"\n");
        if !ok {
            at = start;
            break;
        }
        i += 1;
    }
    mnt_mkdir_dir();
    let fd = mnt_open_table(0o1 | 0o100 | 0o1000, 0o644);
    if fd < 0 {
        return;
    }
    syscall4(SYS_WRITE, fd as usize, out.as_ptr() as usize, at, 0);
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
}

/// Delete a guest directory tree. Used only for a tmpfs store the table itself
/// created, and always with MNT_LOCK free (it goes through our own openat and
/// unlinkat, which take the lock).
unsafe fn rm_rf_guest(dir: &[u8], depth: usize) -> bool {
    if dir.is_empty() || dir.len() >= 320 || depth > 64 {
        return false;
    }
    let mut z = [0u8; 321];
    z[..dir.len()].copy_from_slice(dir);
    z[dir.len()] = 0;
    let dfd = openat(AT_FDCWD, z.as_ptr() as *const CChar, O_DIRECTORY | O_CLOEXEC, 0);
    if dfd < 0 {
        return false;
    }
    let mut ok = true;
    let mut buf = [0u8; 2048];
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
            if nm != b"." && nm != b".." && dir.len() + 1 + len < 320 {
                let mut c = [0u8; 321];
                c[..dir.len()].copy_from_slice(dir);
                c[dir.len()] = b'/';
                c[dir.len() + 1..dir.len() + 1 + len].copy_from_slice(nm);
                c[dir.len() + 1 + len] = 0;
                let cn = dir.len() + 1 + len;
                if unlinkat(AT_FDCWD, c.as_ptr() as *const CChar, 0) != 0
                    && !rm_rf_guest(&c[..cn], depth + 1)
                {
                    ok = false;
                }
            }
            off += reclen;
        }
    }
    rc::close(dfd);
    if rmdirat_path(dir) != 0 {
        ok = false;
    }
    ok
}

/// Longest-prefix substitution. `None` means "no entry matched", which is what
/// the hop loop needs: comparing the length before and after is a different
/// question (a mapping can keep the length and still happen).
unsafe fn mnt_apply(gp: &mut [u8; PATH_MAX], n: usize) -> Option<usize> {
    mnt_ensure();
    mnt_lock();
    let mut best: isize = -1;
    let mut bi = 0usize;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let tl = e.tlen as usize;
        if tl <= n && &gp[..tl] == &e.tgt[..tl] && (n == tl || gp[tl] == b'/') && (tl as isize) > best
        {
            best = tl as isize;
            bi = i;
        }
        i += 1;
    }
    if best < 0 {
        mnt_unlock();
        return None;
    }
    let e = &MNT[bi];
    let tl = e.tlen as usize;
    let sl = e.slen as usize;
    let rest = n - tl;
    if sl + rest >= PATH_MAX {
        mnt_unlock();
        return None;
    }
    let mut tmp = [0u8; PATH_MAX];
    tmp[..sl].copy_from_slice(&e.src[..sl]);
    if rest > 0 {
        tmp[sl..sl + rest].copy_from_slice(&gp[tl..n]);
    }
    mnt_unlock();
    gp[..sl + rest].copy_from_slice(&tmp[..sl + rest]);
    Some(sl + rest)
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
    mnt_ensure();
    mnt_lock();
    let mut hit = false;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let tl = e.tlen as usize;
        if e.ro != 0 && tl <= gp.len() && &gp[..tl] == &e.tgt[..tl] && (gp.len() == tl || gp[tl] == b'/')
        {
            hit = true;
            break;
        }
        i += 1;
    }
    mnt_unlock();
    hit
}

/// Reverse-map a *mapped* guest path back to the mount point the caller used.
/// Descriptors are why this exists: `open("/mnt/ro")` then
/// `openat(fd, "f", O_WRONLY)` resolves to the tmpfs source, where a
/// forward-only read-only check never fires.
unsafe fn mnt_unmap(gp: &mut [u8; PATH_MAX], n: usize) -> Option<usize> {
    mnt_lock();
    let mut best: isize = -1;
    let mut bi = 0usize;
    let mut i = 0usize;
    while i < MNT_N {
        let e = &MNT[i];
        let sl = e.slen as usize;
        if sl > 0 && sl <= n && &gp[..sl] == &e.src[..sl] && (n == sl || gp[sl] == b'/') && (sl as isize) > best
        {
            best = sl as isize;
            bi = i;
        }
        i += 1;
    }
    if best < 0 {
        mnt_unlock();
        return None;
    }
    let e = &MNT[bi];
    let sl = e.slen as usize;
    let tl = e.tlen as usize;
    let rest = n - sl;
    if tl + rest >= PATH_MAX {
        mnt_unlock();
        return None;
    }
    let mut tmp = [0u8; PATH_MAX];
    tmp[..tl].copy_from_slice(&e.tgt[..tl]);
    if rest > 0 {
        tmp[tl..tl + rest].copy_from_slice(&gp[sl..n]);
    }
    mnt_unlock();
    gp[..tl + rest].copy_from_slice(&tmp[..tl + rest]);
    Some(tl + rest)
}

/// Is this guest path inside an emulated read-only mount? Accepts either the
/// caller's view or the post-table (source) form, which is what a directory
/// descriptor hands us.
unsafe fn ro_gp(gp: &[u8]) -> bool {
    if mnt_is_ro(gp) {
        return true;
    }
    if gp.len() >= PATH_MAX {
        return false;
    }
    let mut cur = [0u8; PATH_MAX];
    cur[..gp.len()].copy_from_slice(gp);
    let mut n = gp.len();
    let mut hop = 0;
    while hop < 4 {
        match mnt_unmap(&mut cur, n) {
            Some(m) => {
                n = m;
                if mnt_is_ro(&cur[..n]) {
                    return true;
                }
            }
            None => break,
        }
        hop += 1;
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
    mnt_ensure();
    if flags & MS_REMOUNT != 0 {
        mnt_lock();
        let mut i = 0usize;
        while i < MNT_N {
            let e = &mut MNT[i];
            let tl = e.tlen as usize;
            if tl == n && &e.tgt[..tl] == &gp[..n] {
                e.ro = if flags & MS_RDONLY != 0 { 1 } else { 0 };
                mnt_save();
                mnt_unlock();
                return 0;
            }
            i += 1;
        }
        mnt_unlock();
        return set_errno(22);
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
    // Resolve the source *before* taking the lock: abs_guest() reads the table.
    let mut sp = [0u8; PATH_MAX];
    let mut sn = 0usize;
    if bind {
        if source.is_null() {
            return set_errno(14);
        }
        sn = match abs_guest(source, &mut sp) {
            Some(v) => v,
            None => return set_errno(36),
        };
        if sn >= MNT_P {
            return set_errno(36);
        }
    }
    // Refuse before the tmpfs store is created: a full table must not leave a
    // directory nobody will ever delete. Re-checked under the lock below.
    mnt_lock();
    let full = MNT_N >= MNT_MAX;
    mnt_unlock();
    if full {
        return set_errno(28);
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
        let base = b"/run/husk/t";
        let mut k = 0usize;
        while k < base.len() {
            sd[k] = base[k];
            k += 1;
        }
        // One store per mount, named from this thread and a counter - never
        // from the table index, which umount2 reuses. Naming it after the index
        // is what let a brand-new tmpfs show the files of a dead one.
        static mut TMPFS_SEQ: u64 = 0;
        TMPFS_SEQ += 1;
        let uniq = ((syscall4(SYS_GETTID, 0, 0, 0, 0) as u64) << 20) | (TMPFS_SEQ & 0xF_FFFF);
        let mut d = [0u8; 24];
        let dn = put_i64(&mut d, 0, uniq as i64);
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
        e.src[..sn].copy_from_slice(&sp[..sn]);
        e.slen = sn as u16;
    }
    mnt_lock();
    if MNT_N >= MNT_MAX {
        mnt_unlock();
        return set_errno(28);
    }
    // util-linux performs a bind as two calls (bind, then bind|remount for the
    // flags). A second call for the same target with the same source is the
    // same mount, not EBUSY; mirror the kernel by updating the flags instead.
    let mut i = 0usize;
    while i < MNT_N {
        let tl = MNT[i].tlen as usize;
        if tl == n && &MNT[i].tgt[..tl] == &gp[..n] {
            if bind && sn > 0 && sn == MNT[i].slen as usize && &MNT[i].src[..sn] == &sp[..sn] {
                MNT[i].ro = if flags & MS_RDONLY != 0 { 1 } else { 0 };
                mnt_save();
                mnt_unlock();
                return 0;
            }
            mnt_unlock();
            return set_errno(16);
        }
        i += 1;
    }
    MNT[MNT_N] = e;
    MNT_N += 1;
    mnt_save();
    mnt_unlock();
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
    mnt_ensure();
    let mut src = [0u8; MNT_P];
    let mut sn = 0usize;
    let mut was_tmpfs = false;
    let mut found = false;
    mnt_lock();
    let mut i = 0usize;
    while i < MNT_N {
        let tl = MNT[i].tlen as usize;
        if tl == n && &MNT[i].tgt[..tl] == &gp[..n] {
            sn = MNT[i].slen as usize;
            src[..sn].copy_from_slice(&MNT[i].src[..sn]);
            was_tmpfs = MNT[i].tmpfs != 0;
            let mut k = i;
            while k + 1 < MNT_N {
                MNT[k] = MNT[k + 1];
                k += 1;
            }
            MNT_N -= 1;
            mnt_save();
            found = true;
            break;
        }
        i += 1;
    }
    mnt_unlock();
    if !found {
        return set_errno(22);
    }
    if was_tmpfs {
        // The store is ours to delete. Left behind, its contents could be
        // reached again by a later mount - and a tmpfs that starts non-empty is
        // not a tmpfs.
        rm_rf_guest(&src[..sn], 0);
    }
    0
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
    let entry_errno = *__errno_location();
    mq_mkdirs();
    // The queue is a directory here, so it needs the execute bits whatever the
    // caller asked for: mode 0666 (what a program passes for a queue it only
    // wants read/write on) would otherwise make the queue unusable.
    let m = ((if mode == 0 { 0o666 } else { mode }) & 0o666) | 0o700;
    if oflag & MQ_FLAGS_CREAT != 0 {
        let r = mkdirat(AT_FDCWD, qp.as_ptr() as *const CChar, m);
        if r < 0 {
            let e = *__errno_location();
            if e != 17 {
                return -1;
            }
            if oflag & MQ_FLAGS_EXCL != 0 {
                return set_errno(17);
            }
            // A queue directory left behind by an older build may lack the
            // execute bits (0666 with no x). Repair it below, on the open
            // descriptor: a raw fchmodat here would bypass path translation.
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
    if oflag & MQ_FLAGS_CREAT != 0 {
        syscall4(SYS_FCHMOD, fd as usize, m as usize, 0, 0);
    }
    // The create path above walks through mkdirat, whose EEXIST leaves errno
    // set even though this call succeeded. POSIX says a successful call must
    // not disturb errno, and a stale value here misleads every caller that
    // inspects errno after a successful mq_open.
    *__errno_location() = entry_errno;
    fd
}

/// The fortified entry point glibc exports alongside `mq_open`. It is called
/// without a mode argument (it refuses the O_CREAT case), so the third argument
/// here is never read.
#[no_mangle]
pub unsafe extern "C" fn __mq_open_2(name: *const CChar, oflag: CInt) -> CInt {
    mq_open(name, oflag, 0)
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
    let entry_errno = *__errno_location();
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
    let rr = unlinkat(AT_FDCWD, qp.as_ptr() as *const CChar, 0x200);
    if rr == 0 {
        *__errno_location() = entry_errno;
    }
    rr
}

unsafe fn rmdirat_path(p: &[u8]) -> CInt {
    let mut z = [0u8; 321];
    z[..p.len()].copy_from_slice(p);
    z[p.len()] = 0;
    pathop(AT_FDCWD, z.as_ptr() as *const CChar, false, |fd, q| {
        syscall4(SYS_UNLINKAT, fd as usize, q as usize, 0x200, 0)
    })
}

/// Number of messages currently queued: every regular file in the queue
/// directory is one message, and `.`/`..` are the only other names.
unsafe fn mq_count(dfd: CInt) -> usize {
    let mut n = 0usize;
    let mut buf = [0u8; 1024];
    let mut pb = [0u8; 32];
    let scan = openat(AT_FDCWD, fd_proc_path(dfd, &mut pb).as_ptr() as *const CChar, O_DIRECTORY | 0o4000, 0);
    let sfd = if scan >= 0 { scan } else { dfd };
    loop {
        let r = syscall4(61, sfd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
        if r <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < r as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &buf[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if nm != b"." && nm != b".." {
                n += 1;
            }
            off += reclen;
        }
    }
    if scan >= 0 {
        rc::close(scan);
    }
    n
}

/// Scans the queue directory and remembers the best (priority, name) message.
unsafe fn mq_pick(dfd: CInt, best: &mut [u8; 64], best_len: &mut usize, best_prio: &mut u32) -> bool {
    let mut found = false;
    let mut buf = [0u8; 1024];
    // getdents64 keeps a file offset on the descriptor; without this the second
    // scan of the same queue starts at EOF and every later receive returns
    // EAGAIN even though messages are still queued.
    syscall4(SYS_LSEEK, dfd as usize, 0, 0, 0);
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
        return set_errno((-fd) as CInt);
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
    core::ptr::write_volatile(a.add(3), mq_count(mqdes) as i64);
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
    mnt_ensure();
    mnt_lock();
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
    mnt_unlock();
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
    // Built from the same table netlink answers with, so /proc/net/dev and
    // `ip addr` cannot disagree about which interfaces exist.
    let mut ifs = [IfEnt::empty(); 8];
    let n = nl_interfaces(&mut ifs);
    for e in ifs.iter().take(n) {
        vt_put(at, b"  ");
        vt_put(at, &e.name[..e.name_len]);
        vt_put(at, b": 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n");
    }
    true
}

/// Eight upper-case hex digits, the form /proc/net/route uses.
unsafe fn vt_hex8(at: &mut usize, v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 8];
    let mut i = 8;
    let mut x = v;
    loop {
        i -= 1;
        out[i] = HEX[(x & 15) as usize];
        x >>= 4;
        if i == 0 {
            break;
        }
    }
    vt_put(at, &out);
}

/// /proc/net/route: the connected routes of every interface that has one.
/// Static routes and a default gateway are not visible in this domain, so they
/// are absent rather than invented.
unsafe fn synth_net_route(at: &mut usize) -> bool {
    vt_put(
        at,
        b"Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n",
    );
    let mut ifs = [IfEnt::empty(); 8];
    let n = nl_interfaces(&mut ifs);
    for e in ifs.iter().take(n) {
        if !e.have_ip {
            continue;
        }
        vt_put(at, &e.name[..e.name_len]);
        vt_put(at, b"\t");
        // /proc/net/route prints the address bytes as a little-endian number,
        // so 192.168.1.0/24 shows up as C0A80100 / FFFFFF00.
        vt_hex8(at, (e.ip & e.mask).swap_bytes());
        vt_put(at, b"\t00000000\t0001\t0\t0\t0\t");
        vt_hex8(at, e.mask.swap_bytes());
        vt_put(at, b"\t0\t0\t0\n");
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
        if !synth_net_route(&mut at) {
            return None;
        }
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
    if p == b"/proc/sysvipc/shm" || p == b"/proc/sysvipc/sem" || p == b"/proc/sysvipc/msg" {
        synth_sysvipc(p, &mut at);
        return Some(at);
    }
    if let Some(v) = synth_sysctl(p) {
        vt_put(&mut at, v);
        return Some(at);
    }
    None
}


/// Calls `f(id)` for every `<id>.m` under a SysV store directory.
unsafe fn ipc_each<F: FnMut(i32)>(dir: &[u8], mut f: F) {
    let dfd = openat(AT_FDCWD, dir.as_ptr() as *const CChar, O_DIRECTORY, 0);
    if dfd < 0 {
        return;
    }
    let mut buf = [0u8; 2048];
    loop {
        let r = syscall4(61, dfd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
        if r <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < r as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &buf[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if len > 2 && &nm[len - 2..] == b".m" {
                let mut id = 0i32;
                let mut ok = true;
                for k in 0..len - 2 {
                    if !nm[k].is_ascii_digit() {
                        ok = false;
                        break;
                    }
                    id = id * 10 + (nm[k] - b'0') as i32;
                }
                if ok {
                    f(id);
                }
            }
            off += reclen;
        }
    }
    rc::close(dfd);
}

/// `/proc/sysvipc/{shm,sem,msg}` in the column layout `ipcs` parses. There is no
/// such file in this kernel; the numbers come from the same store the shim
/// serves shmget/semget/msgget out of.
unsafe fn synth_sysvipc(p: &[u8], at: &mut usize) {
    if p == b"/proc/sysvipc/shm" {
        vt_put(at, b"       key      shmid perms       size  cpid  lpid nattch   uid   gid  cuid  cgid      atime      dtime      ctime        rss       swap\n");
        ipc_each(b"/run/husk/ipc/shm\0", |id| {
            let mut m = ShmMeta::empty();
            if !shm_read_meta(id, &mut m) {
                return;
            }
            vt_num(at, m.key as u64);
            vt_put(at, b" ");
            vt_num(at, id as u64);
            vt_put(at, b" ");
            vt_oct(at, m.mode);
            vt_put(at, b" ");
            vt_num(at, m.size);
            vt_put(at, b" ");
            vt_num(at, m.cpid as u64);
            vt_put(at, b" 0 ");
            vt_num(at, shm_att_count(id).max(0) as u64);
            vt_put(at, b" 0 0 0 0 ");
            vt_num(at, m.ctime as u64);
            vt_put(at, b" 0 ");
            vt_num(at, m.ctime as u64);
            vt_put(at, b" 0 0\n");
        });
        return;
    }
    if p == b"/proc/sysvipc/sem" {
        vt_put(at, b"       key      semid perms      nsems   uid   gid  cuid  cgid      otime      ctime\n");
        ipc_each(b"/run/husk/ipc/sem\0", |id| {
            let mut m = [0i64; 4];
            if !sem_read_meta(id, &mut m) {
                return;
            }
            vt_num(at, m[0] as u64);
            vt_put(at, b" ");
            vt_num(at, id as u64);
            vt_put(at, b" ");
            vt_oct(at, m[2] as u32);
            vt_put(at, b" ");
            vt_num(at, m[1] as u64);
            vt_put(at, b" 0 0 0 0 0 0\n");
        });
        return;
    }
    vt_put(at, b"       key      msqid perms      cbytes       qnum lspid lrpid   uid   gid  cuid  cgid      stime      rtime      ctime\n");
    ipc_each(b"/run/husk/ipc/msg\0", |id| {
        let mut m = [0i64; 9];
        if !msg_read_meta(id, &mut m) {
            return;
        }
        vt_num(at, m[0] as u64);
        vt_put(at, b" ");
        vt_num(at, id as u64);
        vt_put(at, b" ");
        vt_oct(at, m[1] as u32);
        vt_put(at, b" 0 ");
        vt_num(at, msg_count(id) as u64);
        vt_put(at, b" ");
        vt_num(at, m[4] as u64);
        vt_put(at, b" ");
        vt_num(at, m[5] as u64);
        vt_put(at, b" 0 0 0 0 ");
        vt_num(at, m[6] as u64);
        vt_put(at, b" ");
        vt_num(at, m[7] as u64);
        vt_put(at, b" ");
        vt_num(at, m[8] as u64);
        vt_put(at, b"\n");
    });
}

unsafe fn vt_oct(at: &mut usize, v: u32) {
    let mut d = [0u8; 12];
    let mut i = 12;
    let mut v = v as u64;
    loop {
        i -= 1;
        d[i] = b'0' + (v % 8) as u8;
        v /= 8;
        if v == 0 {
            break;
        }
    }
    vt_put(at, &d[i..]);
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
    let mut id_dev = 0u64;
    let mut id_ino = 0u64;
    if syscall4(SYS_FSTAT, src as usize, st.as_ptr() as usize, 0, 0) == 0 {
        mode = rd4u(st.as_ptr().add(16)) & 0o7777;
        id_dev = core::ptr::read_unaligned(st.as_ptr() as *const u64);
        id_ino = core::ptr::read_unaligned(st.as_ptr().add(8) as *const u64);
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
            alias_register(tfd, id_dev, id_ino, to);
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

// ------------------------------------------------------- hard-link aliases
// Android refuses link(2) inside app data (EACCES), so link() is emulated by a
// copy. A copy is only an approximation: the caller may keep writing through
// its own descriptor after publishing the second name, and those writes land in
// the now-unlinked source inode. glibc's localedef does exactly that for
// /usr/lib/locale/locale-archive (mkstemp, fill in through a mapping, link the
// temp into place, keep using the original fd), which is why the first locale
// of every archive vanished. Remember each emulated link by source inode and
// re-copy the data when that inode is closed.
const ALIAS_MAX: usize = 8;
const ALIAS_PATH_MAX: usize = 512;
static mut ALIAS_DEV: [u64; ALIAS_MAX] = [0; ALIAS_MAX];
static mut ALIAS_INO: [u64; ALIAS_MAX] = [0; ALIAS_MAX];
static mut ALIAS_PATH: [[u8; ALIAS_PATH_MAX]; ALIAS_MAX] = [[0; ALIAS_PATH_MAX]; ALIAS_MAX];
static ALIAS_MASK: AtomicU32 = AtomicU32::new(0);

/// A read-only descriptor for the inode behind `fd`, via /proc/self/fd. The
/// literal path is passed straight to the kernel, so no guest path resolution
/// happens here.
unsafe fn proc_fd_read(fd: CInt) -> Ssize {
    const O_RDONLY: usize = 0;
    const O_CLOEXEC: usize = 0o2000000;
    let mut p = [0u8; 32];
    let mut n = 0usize;
    for c in b"/proc/self/fd/" {
        p[n] = *c;
        n += 1;
    }
    let mut digits = [0u8; 12];
    let mut d = 0usize;
    let mut x = if fd < 0 { 0 } else { fd as u64 };
    loop {
        digits[d] = b'0' + (x % 10) as u8;
        d += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        p[n] = digits[d];
        n += 1;
    }
    p[n] = 0;
    syscall4(SYS_OPENAT, AT_FDCWD as usize, p.as_ptr() as usize, O_RDONLY | O_CLOEXEC, 0)
}

unsafe fn alias_register(dirfd: CInt, dev: u64, ino: u64, dst: *const CChar) {
    if dst.is_null() || dev == 0 {
        return;
    }
    // Store an absolute path: the link target is opened again from close(),
    // long after the caller's dirfd/cwd context may be gone.
    let mut path = [0u8; ALIAS_PATH_MAX];
    let mut n = 0usize;
    if *dst != b'/' as CChar {
        if dirfd == AT_FDCWD {
            let r = syscall4(
                SYS_READLINKAT,
                AT_FDCWD as usize,
                b"/proc/self/cwd\0".as_ptr() as usize,
                path.as_mut_ptr() as usize,
                ALIAS_PATH_MAX - 8,
            );
            if r <= 0 {
                return;
            }
            n = r as usize;
        } else {
            // Resolve the directory descriptor to a real path now: the caller
            // may close it long before the copy is refreshed.
            let mut link = [0u8; 40];
            let mut k = 0usize;
            for c in b"/proc/self/fd/" {
                link[k] = *c;
                k += 1;
            }
            let mut digits = [0u8; 12];
            let mut d = 0usize;
            let mut x = dirfd as u64;
            loop {
                digits[d] = b'0' + (x % 10) as u8;
                d += 1;
                x /= 10;
                if x == 0 {
                    break;
                }
            }
            while d > 0 {
                d -= 1;
                link[k] = digits[d];
                k += 1;
            }
            link[k] = 0;
            let r = syscall4(
                SYS_READLINKAT,
                AT_FDCWD as usize,
                link.as_ptr() as usize,
                path.as_mut_ptr() as usize,
                ALIAS_PATH_MAX - 8,
            );
            if r <= 0 {
                return;
            }
            n = r as usize;
        }
        path[n] = b'/';
        n += 1;
    }
    let mut m = 0usize;
    while n + m + 1 < ALIAS_PATH_MAX && *dst.add(m) != 0 {
        m += 1;
    }
    if *dst.add(m) != 0 || m == 0 {
        return; // longer than we can remember: the copy stays as it is
    }
    core::ptr::copy_nonoverlapping(dst as *const u8, path.as_mut_ptr().add(n), m);
    n += m;
    path[n] = 0;
    let mask = ALIAS_MASK.load(Ordering::Relaxed);
    let mut slot = usize::MAX;
    let mut i = 0usize;
    while i < ALIAS_MAX {
        if mask & (1u32 << i) == 0 {
            slot = i;
            break;
        }
        i += 1;
    }
    if slot == usize::MAX {
        return;
    }
    ALIAS_DEV[slot] = dev;
    ALIAS_INO[slot] = ino;
    core::ptr::copy_nonoverlapping(path.as_ptr(), ALIAS_PATH[slot].as_mut_ptr(), n);
    ALIAS_PATH[slot][n] = 0;
    ALIAS_MASK.fetch_or(1u32 << slot, Ordering::Relaxed);
}

/// Re-copy an emulated link's target from the still-open source fd. Called from
/// close(), i.e. when the writer is finished with its own descriptor.
unsafe fn alias_refresh(fd: CInt) {
    if ALIAS_MASK.load(Ordering::Relaxed) == 0 || fd < 0 {
        return;
    }
    let mut st = [0u8; 128];
    if syscall4(SYS_FSTAT, fd as usize, st.as_ptr() as usize, 0, 0) != 0 {
        return;
    }
    let dev = core::ptr::read_unaligned(st.as_ptr() as *const u64);
    let ino = core::ptr::read_unaligned(st.as_ptr().add(8) as *const u64);
    const O_WRONLY: usize = 0o1;
    const O_TRUNC: usize = 0o1000;
    const O_CLOEXEC: usize = 0o2000000;
    let mut i = 0usize;
    while i < ALIAS_MAX {
        let bit = 1u32 << i;
        if ALIAS_MASK.load(Ordering::Relaxed) & bit != 0
            && ALIAS_DEV[i] == dev
            && ALIAS_INO[i] == ino
        {
            // The writer's descriptor is usually write-only, so read the same
            // inode through /proc/self/fd. Only truncate the copy once this
            // succeeds, otherwise a failed refresh would empty a good copy.
            let rfd = proc_fd_read(fd);
            if rfd < 0 {
                i += 1;
                continue;
            }
            let rfd = rfd as CInt;
            let dst = syscall4(
                SYS_OPENAT,
                AT_FDCWD as usize,
                ALIAS_PATH[i].as_ptr() as usize,
                O_WRONLY | O_TRUNC | O_CLOEXEC,
                0,
            );
            if dst >= 0 {
                let dst = dst as CInt;
                let mut buf = [0u8; 65536];
                let mut off = 0usize;
                loop {
                    let n = syscall4(
                        SYS_PREAD64,
                        rfd as usize,
                        buf.as_ptr() as usize,
                        buf.len(),
                        off,
                    );
                    if n <= 0 {
                        break;
                    }
                    let mut w = 0usize;
                    while w < n as usize {
                        let k = syscall4(
                            SYS_WRITE,
                            dst as usize,
                            buf.as_ptr().add(w) as usize,
                            n as usize - w,
                            0,
                        );
                        if k <= 0 {
                            break;
                        }
                        w += k as usize;
                    }
                    off += n as usize;
                }
                rc::close(dst);
            }
            rc::close(rfd);
            ALIAS_MASK.fetch_and(!bit, Ordering::Relaxed);
        }
        i += 1;
    }
}

#[no_mangle]
unsafe fn openat_inner(dirfd: CInt, path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    // dpkg unpacks a conffile by creating `<dest>.dpkg-new` with the mode the
    // package asks for and then writing through that very descriptor. Modes
    // such as sudo's /etc/sudoers.d/README (0440) leave the *owner* without
    // write permission, and the kernel enforces that against the real app uid,
    // not the uid we report. Root ignores those bits, so a file we create always
    // keeps owner rw: measured as "unable to open
    // '/etc/sudoers.d/README.dpkg-new': Permission denied" in the 2.0 sudo step
    // (evidence 137).
    let mode = if flags & O_CREAT != 0 { mode | 0o600 } else { mode };
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
                    // Descriptors are the ordinary way round a check that only
                    // reads the caller's string: open("/mnt/ro") once, then
                    // openat(fd, "f", O_WRONLY) resolves to the tmpfs source.
                    if flags & O_WRITEISH != 0 && ro_gp(&gp[..n]) {
                        return set_errno(30);
                    }
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
                    match plan::gpu_family() {
                        // Our DRM semantics layer on top of the vendor kbase.
                        plan::GPU_MALI => return drm_fake_open(),
                        // A render node that really opens: use it, never shadow it.
                        plan::GPU_DRM => {}
                        // No adapter for this family yet: say why in the log, then
                        // hand the guest a plain ENOENT so Mesa picks llvmpipe.
                        _ => {
                            plan::log_plan_once(b"drm node");
                            return set_errno(2);
                        }
                    }
                }
            }
            if is_drm_dir(&gp[..n]) && plan::gpu_family() == plan::GPU_MALI {
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
        match plan::gpu_family() {
            plan::GPU_MALI => return drm_fake_open(),
            plan::GPU_DRM => {}
            _ => {
                plan::log_plan_once(b"drm node");
                return set_errno(2);
            }
        }
    }
    ret_or_errno(syscall4(
        SYS_OPENAT,
        dirfd as usize,
        buf.as_ptr() as usize,
        flags as usize,
        mode as usize,
    ))
}


/// Root inside the container may write what the kernel refuses to us.
///
/// The real uid is the app's, never 0, so a file the guest itself just created
/// with a restrictive mode is off-limits to its own next open(). dpkg does
/// exactly that: it unpacks a conffile as `<dest>.dpkg-new`, applies the
/// package's mode, and then opens that name again for the content, which turned
/// into "unable to open '/etc/sudoers.d/README.dpkg-new': Permission denied"
/// while installing sudo (evidence 138). Widen the owner bit once and retry; the
/// guest still reads back whatever mode it set itself.
unsafe fn widen_owner_write(dirfd: CInt, path: *const CChar) -> bool {
    if let Some(v) = env_lookup(b"NR_FAKE_UID") {
        if v != b"0" {
            return false;
        }
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return false,
    };
    if ro_gp(&gp[..n]) {
        return false;
    }
    chmod_target(dirfd, path, 0o600, true) == 0
}

#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: CInt, path: *const CChar, flags: CInt, mode: CInt) -> CInt {
    let fresh = own_create_check(dirfd, path, flags);
    let mut r = openat_inner(dirfd, path, flags, mode);
    if r < 0
        && !path.is_null()
        && flags & O_WRITEISH != 0
        && *__errno_location() == EACCES
        && widen_owner_write(dirfd, path)
    {
        if dbg_on() {
            dbg_file(&[b"openat widened owner write ", bytes(path), b"\n"]);
        }
        r = openat_inner(dirfd, path, flags, mode);
    }
    if r >= 0 && fresh {
        own_note_fd(r);
    }
    if r >= 0 && flags & 0o20000000 != 0 {
        // O_TMPFILE: an unnamed inode, created by definition.
        own_note_fd(r);
    }
    // EINVAL out of an open is always a real defect: either the guest asked for
    // a flag combination this kernel cannot honour, or our own translation
    // mangled it. Record which, rather than let a package manager report
    // "error creating new backup file" with no way to see the request.
    if r < 0 && !path.is_null() && *__errno_location() == EINVAL && dbg_on() {
        let (b, k) = dec_buf(flags as i64);
        dbg_file(&[b"openat EINVAL flags=", &b[k..], b" ", bytes(path), b"\n"]);
    }
    // A denial here is worth a line of its own: when a package manager reports
    // "unable to open ...: Permission denied" the flag word and the exact path
    // are the whole diagnosis, and neither shows up in its own message.
    if r < 0 && !path.is_null() && dbg_on() {
        let e = *__errno_location();
        if e != 0 {
            let (b, k) = dec_buf(flags as i64);
            let (c, j) = dec_buf(e as i64);
            dbg_file(&[b"openat denied errno=", &c[j..], b" flags=", &b[k..], b" ", bytes(path), b"\n"]);
        }
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
            Ok(()) => {
                own_patch_stat(st);
                0
            }
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
                    Ok(()) => {
                        own_patch_stat(st);
                        0
                    }
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
/// Read-only mounts must refuse *metadata* writes with EROFS exactly like they
/// refuse a write through `open()`. chmod, chown, utimensat, xattr writes and
/// mknod all change the tree just as much as writing a byte does; they simply
/// do not go through `open()`, so they need their own check.
unsafe fn ro_refuse_at(dirfd: CInt, path: *const CChar) -> bool {
    if path.is_null() || *path == 0 {
        return false;
    }
    let mut gp = [0u8; PATH_MAX];
    let ro = if *path == b'/' as CChar {
        match abs_guest(path, &mut gp) {
            Some(n) => ro_gp(&gp[..n]),
            None => false,
        }
    } else {
        match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
            Ok(Some(n)) => ro_gp(&gp[..n]),
            _ => false,
        }
    };
    if ro {
        set_errno(30); // EROFS
    }
    ro
}

/// Same, for the path-only entry points (xattr) that have no directory fd.
unsafe fn ro_refuse(path: *const CChar) -> bool {
    ro_refuse_at(AT_FDCWD, path)
}

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
    // The caller's string is checked when it is absolute; `gp` covers the
    // relative form, where the string alone never carries the mount point.
    if path_is_ro(path) || ro_gp(&gp[..n]) {
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
    if ro_refuse_at(dirfd, path) {
        return -1;
    }
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
    if path.is_null() {
        return set_errno(14);
    }
    if ro_refuse_at(dirfd, path) {
        return -1;
    }
    // Under the fake-root convention the guest cannot change ownership (no
    // CAP_CHOWN), and every package manager calls it on every file. Reporting
    // success is what a user namespace would do; the real owner on disk stays
    // u0_a302. See the fake-root note above getuid() - and the table below,
    // without which the *next* process would read u0_a302 back.
    if fake_root_on() {
        own_record_chown(dirfd, path, uid, gid, follow, raw_flags);
        return 0;
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
    if ro_refuse_at(dirfd, path) {
        return -1;
    }
    // No fake-root shortcut here: setting a timestamp needs no privilege on a
    // file this process owns, and the caller reads the value back (make, git,
    // tar, rsync, `touch -h`). Swallowing the call is why
    // 04-path-reliability's utime-nofollow check used to report "now".
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
    let mut gscratch = [0u8; PATH_MAX];
    let (parent, name, own, _g) = match one_side(
        rfd,
        dirfd,
        path,
        false,
        &mut raw,
        &mut nb,
        &mut gscratch,
    ) {
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
/// Resolve one side of a two-path call into (dirfd, name, owned) and hand back
/// the guest path it denotes, so read-only mounts can be enforced on operations
/// that take two paths.
unsafe fn one_side(
    rfd: CInt,
    dirfd: CInt,
    path: *const CChar,
    follow: bool,
    raw: &mut [u8; PATH_MAX],
    nb: &mut [u8; 260],
    gpo: &mut [u8; PATH_MAX],
) -> Result<(CInt, *const CChar, bool, usize), CInt> {
    if dirfd != AT_FDCWD && *path != b'/' as CChar {
        if rfd >= 0 {
            let mut gp = [0u8; PATH_MAX];
            match resolve_dirfd_rel(dirfd, bytes(path), &mut gp) {
                Ok(Some(n)) => {
                    gpo[..n].copy_from_slice(&gp[..n]);
                    let mut name = [0u8; 256];
                    let parent = if follow {
                        rc::resolve_parent_follow(rfd, &gp[..n], &mut name)
                    } else {
                        rc::resolve_parent(rfd, &gp[..n], &mut name)
                    }?;
                    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
                    nb[..nl].copy_from_slice(&name[..nl]);
                    nb[nl] = 0;
                    return Ok((parent, nb.as_ptr() as *const CChar, true, n));
                }
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        // A descriptor outside the guest root: the kernel's own view is the
        // answer and no emulated mount can apply.
        return Ok((dirfd, path, false, 0));
    }
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest(path, &mut gp) {
        Some(n) => n,
        None => return Err(36),
    };
    gpo[..n].copy_from_slice(&gp[..n]);
    if rfd < 0 || passthrough(&gp[..n]) {
        raw[..n].copy_from_slice(&gp[..n]);
        raw[n] = 0;
        return Ok((AT_FDCWD, raw.as_ptr() as *const CChar, false, n));
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
    Ok((parent, nb.as_ptr() as *const CChar, true, n))
}

unsafe fn pathop2<F>(
    afd: CInt,
    a: *const CChar,
    bfd: CInt,
    b: *const CChar,
    follow_a: bool,
    follow_b: bool,
    ro_a: bool,
    ro_b: bool,
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
    let mut ga = [0u8; PATH_MAX];
    let mut gb = [0u8; PATH_MAX];
    let (pa, na, own_a, gla) =
        match one_side(rfd, afd, a, follow_a, &mut rawa, &mut nba, &mut ga) {
            Ok(v) => v,
            Err(e) => return set_errno(e),
        };
    let (pb, nb, own_b, glb) =
        match one_side(rfd, bfd, b, follow_b, &mut rawb, &mut nbb, &mut gb) {
            Ok(v) => v,
            Err(e) => {
                if own_a {
                    rc::close(pa);
                }
                return set_errno(e);
            }
        };
    // rename() touches both sides, link() only creates the new name; either way
    // a read-only emulated mount must refuse before the kernel sees the call.
    if (ro_a && gla > 0 && ro_gp(&ga[..gla])) || (ro_b && glb > 0 && ro_gp(&gb[..glb])) {
        if own_a {
            rc::close(pa);
        }
        if own_b {
            rc::close(pb);
        }
        return set_errno(30);
    }
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
    let r = pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_MKDIRAT, fd as usize, p as usize, mode as usize, 0)
    });
    if r == 0 {
        own_note_path(AT_FDCWD, path);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat(dirfd: CInt, path: *const CChar, mode: CInt) -> CInt {
    if verbose_on() {
        dbg(&[b"MKDIRAT fd="]);
        dbgn(&[], dirfd as i64);
        dbg(&[b"  path=", bytes(path), b"\n"]);
    }
    let r = pathop(dirfd, path, false, |fd, p| {
        syscall4(SYS_MKDIRAT, fd as usize, p as usize, mode as usize, 0)
    });
    if r == 0 {
        own_note_path(dirfd, path);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn rmdir(path: *const CChar) -> CInt {
    let r = pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, 0x200, 0)
    });
    if r == 0 {
        own_clear_path(AT_FDCWD, path);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const CChar) -> CInt {
    let r = pathop(AT_FDCWD, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, 0, 0)
    });
    if r == 0 {
        own_clear_path(AT_FDCWD, path);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat(dirfd: CInt, path: *const CChar, flags: CInt) -> CInt {
    let r = pathop(dirfd, path, false, |fd, p| {
        syscall4(SYS_UNLINKAT, fd as usize, p as usize, flags as usize, 0)
    });
    if r == 0 {
        own_clear_path(dirfd, path);
    }
    r
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

/// The identity fiction is not fixed at 0. A container session *is* a user, and
/// every tool that shows one (`su`, `login`, `setpriv`, PAM modules, shells
/// printing `$UID`) expresses that by calling setuid(). Pretending to be root
/// forever means `id -un` says root inside a user's own session, and that the
/// only way to get a user session is proot's fake `su` - which is exactly the
/// proot-only patch we want to delete. So the fiction follows the calls, and
/// NR_FAKE_UID / NR_FAKE_GID seed it for a launcher that starts as the user.
static mut FAKE_UID: u32 = 0;
static mut FAKE_GID: u32 = 0;
static mut FAKE_ID_READY: u8 = 0;

unsafe fn parse_dec(v: &[u8]) -> u32 {
    let mut n: u64 = 0;
    let mut any = false;
    for &c in v {
        if !(b'0'..=b'9').contains(&c) {
            break;
        }
        any = true;
        n = n * 10 + (c - b'0') as u64;
        if n > u32::MAX as u64 {
            return u32::MAX;
        }
    }
    if any {
        n as u32
    } else {
        0
    }
}

unsafe fn fake_id() -> (u32, u32) {
    if FAKE_ID_READY == 0 {
        FAKE_ID_READY = 1;
        if let Some(v) = env_lookup(b"NR_FAKE_UID") {
            FAKE_UID = parse_dec(v);
        }
        if let Some(v) = env_lookup(b"NR_FAKE_GID") {
            FAKE_GID = parse_dec(v);
        }
    }
    (FAKE_UID, FAKE_GID)
}

#[no_mangle]
pub unsafe extern "C" fn getuid() -> u32 {
    if fake_root_on() {
        return fake_id().0;
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
        return fake_id().1;
    }
    syscall4(SYS_GETGID, 0, 0, 0, 0) as u32
}

#[no_mangle]
pub unsafe extern "C" fn getegid() -> u32 {
    getgid()
}

/// A container user has *its* groups, not the Android app's fistful
/// (input/log/adb/sdcard_rw/net_bt/...). Those ids also make bash's and
/// `id`'s group-name lookups print "cannot find name for group ID" noise, which
/// LinuxHub 2.0 worked around by appending synthetic `android_gid_N` lines to
/// /etc/group. Interposing getgroups removes the cause instead: under the fake
/// identity the process reports exactly one group, its primary one.
#[no_mangle]
pub unsafe extern "C" fn getgroups(size: CInt, list: *mut u32) -> CInt {
    if !fake_root_on() {
        return ret_or_errno(syscall4(SYS_GETGROUPS, size as usize, list as usize, 0, 0));
    }
    let gid = fake_id().1;
    if size == 0 {
        return 1;
    }
    if size < 1 || list.is_null() {
        return set_errno(22);
    }
    *list = gid;
    1
}

// ------------------------------------------------ fictional ownership, on disk
//
// `chown_target()` below answers one process, and that was enough for files
// dpkg unpacks itself - but the rootfs is really owned by the app uid
// (u0_a265), and a program that looks at a path some *other* process created
// reads that number back. sudo refused to start with "/etc/sudo.conf is owned
// by uid 10265, should be 0" (evidence 139 §3.2). A container's answer is the
// same in every process, so the answer has to outlive one.
//
// What a container's userspace observes is: root owns everything it did not
// create, and a path belongs to whoever created or chowned it. So this is an
// *exception* list - nothing is stored for 0:0, which is the default - and it is
// keyed by inode rather than path, which is what makes rename() and hard links
// need no bookkeeping ("x.dpkg-new" renamed into place keeps its owner).
//
//   /var/lib/husk/owners     one record per line: "dev ino uid gid"
//
// Writers append a line with O_APPEND and a single write() - no
// read-modify-write, so two processes touching different files cannot lose each
// other's records - and rewrite the whole file once the appended tail passes
// OWN_LOG_LIMIT. Readers load it once, on first use, and keep an in-memory hash;
// it has to stay small, because every guest process loads it.
const OWN_FILE: &[u8] = b"/var/lib/husk/owners\0";
const OWN_DIR: &[u8] = b"/var/lib/husk\0";
const OWN_INIT_CAP: usize = 256;
const OWN_LOG_LIMIT: usize = 1 << 20;
const O_APPEND_W: CInt = 0o2000;

#[derive(Clone, Copy)]
struct OwnSlot {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    state: u8, // 0 empty, 1 used, 2 tombstone
}

static mut OWN_TAB: *mut OwnSlot = core::ptr::null_mut();
static mut OWN_CAP: usize = 0; // power of two; 0 = nothing allocated yet
static mut OWN_FILL: usize = 0; // used + tombstone slots
static mut OWN_APPEND_FD: CInt = -1;
static mut OWN_APPENDED: usize = 0;
static OWN_STATE: AtomicU8 = AtomicU8::new(0); // 0 not loaded, 1 loading, 2 ready
static OWN_OWNER: AtomicI32 = AtomicI32::new(0);
static OWN_LOCK: AtomicBool = AtomicBool::new(false);

/// The fiction is part of the fake root, and can be switched off on its own
/// (NR_OWNERS=0) so a regression can be attributed to it and not to a rebuild.
unsafe fn owners_on() -> bool {
    if !fake_root_on() {
        return false;
    }
    !matches!(env_lookup(b"NR_OWNERS"), Some(v) if v == b"0" || v == b"off")
}

unsafe fn own_lock() {
    while OWN_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while OWN_LOCK.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
}

unsafe fn own_unlock() {
    OWN_LOCK.store(false, Ordering::Release);
}

#[inline]
fn own_hash(dev: u64, ino: u64) -> usize {
    let mut h = ino.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dev.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (h >> 17) as usize
}

/// (slot, is it this key?) - `slot` is None only when every slot is spoken for,
/// which the grow-on-load policy makes impossible in practice.
unsafe fn own_slot(dev: u64, ino: u64) -> (Option<usize>, bool) {
    if OWN_CAP == 0 {
        return (None, false);
    }
    let mask = OWN_CAP - 1;
    let mut i = own_hash(dev, ino) & mask;
    let mut free = None;
    let mut n = 0usize;
    while n < OWN_CAP {
        let s = &*OWN_TAB.add(i);
        if s.state == 0 {
            return (Some(match free {
                Some(f) => f,
                None => i,
            }), false);
        }
        if s.state == 1 {
            if s.dev == dev && s.ino == ino {
                return (Some(i), true);
            }
        } else if free.is_none() {
            free = Some(i);
        }
        i = (i + 1) & mask;
        n += 1;
    }
    (free, false)
}

unsafe fn own_insert_raw(dev: u64, ino: u64, uid: u32, gid: u32) {
    let (slot, found) = own_slot(dev, ino);
    if let Some(i) = slot {
        let s = &mut *OWN_TAB.add(i);
        if !found && s.state != 1 {
            OWN_FILL += 1;
        }
        s.dev = dev;
        s.ino = ino;
        s.uid = uid;
        s.gid = gid;
        s.state = 1;
    }
}

/// Allocate (or double) the table. Callers hold OWN_LOCK.
unsafe fn own_grow() -> bool {
    let new_cap = if OWN_CAP == 0 {
        OWN_INIT_CAP
    } else {
        OWN_CAP * 2
    };
    let bytes = new_cap * core::mem::size_of::<OwnSlot>();
    let p = malloc(bytes) as *mut OwnSlot;
    if p.is_null() {
        return false;
    }
    core::ptr::write_bytes(p as *mut u8, 0, bytes);
    let old_tab = OWN_TAB;
    let old_cap = OWN_CAP;
    OWN_TAB = p;
    OWN_CAP = new_cap;
    OWN_FILL = 0;
    let mut i = 0usize;
    while i < old_cap {
        let s = *old_tab.add(i);
        if s.state == 1 {
            own_insert_raw(s.dev, s.ino, s.uid, s.gid);
        }
        i += 1;
    }
    if !old_tab.is_null() {
        free(old_tab as *mut u8);
    }
    true
}

/// Record (or, for 0:0, clear) the owner of one inode. Callers hold OWN_LOCK.
/// Returns whether anything changed, which is what decides if a record is worth
/// appending: root creating a file is the common case (every file dpkg unpacks)
/// and its answer *is* the default, so logging it would only bloat the file.
unsafe fn own_set_locked(dev: u64, ino: u64, uid: u32, gid: u32) -> bool {
    if ino == 0 {
        return false;
    }
    if OWN_CAP == 0 && !own_grow() {
        return false;
    }
    // Keep the load factor under 3/4, or every later lookup walks a long chain.
    if (OWN_FILL + 1) * 4 >= OWN_CAP * 3 && !own_grow() {
        return false;
    }
    let (slot, found) = own_slot(dev, ino);
    let i = match slot {
        Some(i) => i,
        None => return false,
    };
    if uid == 0 && gid == 0 {
        if found {
            (*OWN_TAB.add(i)).state = 2;
            return true;
        }
        return false;
    }
    let s = &mut *OWN_TAB.add(i);
    if found && s.uid == uid && s.gid == gid {
        return false;
    }
    if !found && s.state != 1 {
        OWN_FILL += 1;
    }
    s.dev = dev;
    s.ino = ino;
    s.uid = uid;
    s.gid = gid;
    s.state = 1;
    true
}

unsafe fn own_apply_line(line: &[u8]) {
    let mut f = [0u64; 4];
    let mut nf = 0usize;
    let mut i = 0usize;
    while i < line.len() && nf < 4 {
        while i < line.len() && (line[i] == b' ' || line[i] == b'\t' || line[i] == b'\r') {
            i += 1;
        }
        let mut v = 0u64;
        let mut any = false;
        while i < line.len() && line[i] != b' ' && line[i] != b'\t' && line[i] != b'\r' {
            let c = line[i];
            if !(b'0'..=b'9').contains(&c) {
                return;
            }
            if v > (u64::MAX - 9) / 10 {
                return;
            }
            v = v * 10 + (c - b'0') as u64;
            any = true;
            i += 1;
        }
        if !any {
            return;
        }
        f[nf] = v;
        nf += 1;
    }
    if nf == 4 {
        own_set_locked(f[0], f[1], f[2] as u32, f[3] as u32);
    }
}

unsafe fn own_mkdir_dir() {
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(OWN_DIR.as_ptr() as *const CChar, &mut gp) {
        Some(n) => n,
        None => return,
    };
    let rfd = rootfd();
    if rfd < 0 {
        return;
    }
    let mut name = [0u8; 256];
    let parent = match rc::resolve_parent_follow(rfd, &gp[..n], &mut name) {
        Ok(fd) => fd,
        Err(_) => return,
    };
    let nl = name.iter().position(|c| *c == 0).unwrap_or(0);
    syscall4(SYS_MKDIRAT, parent as usize, name.as_ptr() as usize, 0o755, 0);
    rc::close(parent);
}

/// Guest path of the table -> real host path, with raw calls only (no table
/// lookups), so it is safe to call with OWN_LOCK held and inside own_ensure().
unsafe fn own_open_file(flags: CInt, mode: CInt) -> CInt {
    let mut gp = [0u8; PATH_MAX];
    let n = match abs_guest_raw(OWN_FILE.as_ptr() as *const CChar, &mut gp) {
        Some(n) => n,
        None => return -1,
    };
    let mut real = [0u8; PATH_MAX];
    let rn = match mnt_real_path(&gp[..n], &mut real) {
        Some(rn) => rn,
        None => return -1,
    };
    real[rn] = 0;
    ret_or_errno(syscall4(
        SYS_OPENAT,
        AT_FDCWD as usize,
        real.as_ptr() as usize,
        flags as usize,
        mode as usize,
    ))
}

unsafe fn own_read_file() {
    static mut BUF: [u8; 4096] = [0; 4096];
    static mut LINE: [u8; 128] = [0; 128];
    let fd = own_open_file(0, 0);
    if fd < 0 {
        return;
    }
    let mut ll = 0usize;
    own_lock();
    loop {
        let n = syscall4(
            SYS_READ,
            fd as usize,
            core::ptr::addr_of_mut!(BUF) as usize,
            BUF.len(),
            0,
        );
        if n <= 0 {
            break;
        }
        let mut i = 0usize;
        while i < n as usize {
            let c = BUF[i];
            i += 1;
            if c == b'\n' {
                if ll > 0 {
                    let line =
                        core::slice::from_raw_parts(core::ptr::addr_of!(LINE) as *const u8, ll);
                    own_apply_line(line);
                }
                ll = 0;
            } else if ll < 128 {
                LINE[ll] = c;
                ll += 1;
            } else {
                ll = 0; // overlong line: drop it rather than parse half a record
            }
        }
    }
    own_unlock();
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
}

unsafe fn own_ensure() {
    loop {
        match OWN_STATE.load(Ordering::Acquire) {
            2 => return,
            0 => {
                if OWN_STATE
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    let tid = syscall4(SYS_GETTID, 0, 0, 0, 0) as i32;
                    OWN_OWNER.store(tid, Ordering::Release);
                    own_read_file();
                    OWN_OWNER.store(0, Ordering::Release);
                    OWN_STATE.store(2, Ordering::Release);
                    return;
                }
            }
            _ => {
                if OWN_OWNER.load(Ordering::Acquire) == syscall4(SYS_GETTID, 0, 0, 0, 0) as i32 {
                    return;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Rewrite the table file from scratch: what a log past OWN_LOG_LIMIT is
/// compacted into. Callers hold OWN_LOCK.
unsafe fn own_rewrite() {
    if OWN_APPEND_FD >= 0 {
        syscall4(SYS_CLOSE, OWN_APPEND_FD as usize, 0, 0, 0);
        OWN_APPEND_FD = -1;
    }
    let bytes = 32 + OWN_FILL * 48;
    let out = malloc(bytes);
    if out.is_null() {
        return;
    }
    let mut at = 0usize;
    let mut i = 0usize;
    while i < OWN_CAP {
        let s = *OWN_TAB.add(i);
        i += 1;
        if s.state != 1 {
            continue;
        }
        let (d, k) = dec_buf(s.dev as i64);
        let (q, j) = dec_buf(s.ino as i64);
        let (u, a) = dec_buf(s.uid as i64);
        let (g, b) = dec_buf(s.gid as i64);
        for part in [&d[k..], b" ", &q[j..], b" ", &u[a..], b" ", &g[b..], b"\n"] {
            core::ptr::copy_nonoverlapping(part.as_ptr(), out.add(at), part.len());
            at += part.len();
        }
    }
    own_mkdir_dir();
    let fd = own_open_file(0o1 | 0o1000 | 0o100 | 0o2000000, 0o644);
    if fd >= 0 {
        syscall4(SYS_WRITE, fd as usize, out as usize, at, 0);
        syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    }
    free(out);
    OWN_APPENDED = 0;
}

/// Append one record. Callers hold OWN_LOCK.
unsafe fn own_append(dev: u64, ino: u64, uid: u32, gid: u32) {
    if dev > i64::MAX as u64 || ino > i64::MAX as u64 {
        return;
    }
    if OWN_APPENDED > OWN_LOG_LIMIT {
        own_rewrite();
    }
    if OWN_APPEND_FD < 0 {
        own_mkdir_dir();
        OWN_APPEND_FD = own_open_file(0o1 | O_APPEND_W | 0o100 | 0o2000000, 0o644);
        if OWN_APPEND_FD < 0 {
            return;
        }
    }
    let (d, k) = dec_buf(dev as i64);
    let (q, j) = dec_buf(ino as i64);
    let (u, a) = dec_buf(uid as i64);
    let (g, b) = dec_buf(gid as i64);
    let mut out = [0u8; 96];
    let mut at = 0usize;
    for part in [&d[k..], b" ", &q[j..], b" ", &u[a..], b" ", &g[b..], b"\n"] {
        if at + part.len() > out.len() {
            return;
        }
        out[at..at + part.len()].copy_from_slice(part);
        at += part.len();
    }
    let w = syscall4(SYS_WRITE, OWN_APPEND_FD as usize, out.as_ptr() as usize, at, 0);
    if w != at as isize {
        syscall4(SYS_CLOSE, OWN_APPEND_FD as usize, 0, 0, 0);
        OWN_APPEND_FD = -1;
        return;
    }
    OWN_APPENDED += at;
}

/// Who does this inode belong to, inside? Defaults to 0:0 - the whole point is
/// that root owns what nobody bothered to claim.
unsafe fn own_lookup(dev: u64, ino: u64) -> (u32, u32) {
    if ino == 0 {
        return (0, 0);
    }
    if OWN_STATE.load(Ordering::Acquire) != 2 {
        own_ensure();
    }
    own_lock();
    let r = match own_slot(dev, ino) {
        (Some(i), true) => {
            let s = &*OWN_TAB.add(i);
            (s.uid, s.gid)
        }
        _ => (0, 0),
    };
    own_unlock();
    r
}

/// Record an owner. Not a no-op even for 0:0: an inode can be reused after its
/// file is deleted, and the old record would then claim the new file.
unsafe fn own_set(dev: u64, ino: u64, uid: u32, gid: u32) {
    if !owners_on() || ino == 0 || ino > i64::MAX as u64 {
        return;
    }
    if ROOT_DEV != 0 && dev != ROOT_DEV {
        return;
    }
    if OWN_STATE.load(Ordering::Acquire) != 2 {
        own_ensure();
    }
    own_lock();
    if own_set_locked(dev, ino, uid, gid) {
        own_append(dev, ino, uid, gid);
    }
    own_unlock();
}

/// Only the rootfs carries a fiction. Anything else the guest can stat (the
/// passthrough /proc, /dev, the vendor nodes) keeps its real numbers.
#[inline]
unsafe fn own_fictional(dev: u64) -> bool {
    ROOT_DEV != 0 && dev == ROOT_DEV
}

unsafe fn own_patch_stat(st: *mut u8) {
    if !owners_on() {
        return;
    }
    let dev = rd8u(st);
    if !own_fictional(dev) {
        return;
    }
    let (uid, gid) = own_lookup(dev, rd8u(st.add(8)));
    wr4(st.add(24), uid);
    wr4(st.add(28), gid);
}

unsafe fn own_patch_statx(buf: *mut u8) {
    if !owners_on() {
        return;
    }
    let dev = own_dev_pack(rd4u(buf.add(136)), rd4u(buf.add(140)));
    if !own_fictional(dev) {
        return;
    }
    let (uid, gid) = own_lookup(dev, rd8u(buf.add(32)));
    wr4(buf.add(20), uid);
    wr4(buf.add(24), gid);
}

/// glibc's own device packing (gnu_dev_makedev), so a record written from
/// `struct stat` matches one looked up from `struct statx`.
fn own_dev_pack(major: u32, minor: u32) -> u64 {
    let ma = major as u64;
    let mi = minor as u64;
    (mi & 0xff) | ((ma & 0xfff) << 8) | ((mi & !0xffu64) << 12) | ((ma & !0xfffu64) << 32)
}

/// Note the owner of a file this process just created. Files go to root unless
/// the code that created them is running as somebody else - `apt` downloading
/// as `_apt`, a user's shell writing into its home - and those are the ones a
/// later root process would be told it cannot trust.
unsafe fn own_note_fd(fd: CInt) {
    if !owners_on() {
        return;
    }
    let mut st = [0u8; 128];
    if syscall4(SYS_FSTAT, fd as usize, st.as_mut_ptr() as usize, 0, 0) != 0 {
        return;
    }
    let dev = rd8u(st.as_ptr());
    if !own_fictional(dev) {
        return;
    }
    let (uid, gid) = fake_id();
    own_set(dev, rd8u(st.as_ptr().add(8)), uid, gid);
}

unsafe fn own_note_path(dirfd: CInt, path: *const CChar) {
    if !owners_on() || path.is_null() {
        return;
    }
    let (uid, gid) = fake_id();
    let mut st = [0u8; 128];
    if fstatat(dirfd, path, st.as_mut_ptr(), AT_SYMLINK_NOFOLLOW) != 0 {
        return;
    }
    let dev = rd8u(st.as_ptr());
    if !own_fictional(dev) {
        return;
    }
    own_set(dev, rd8u(st.as_ptr().add(8)), uid, gid);
}

/// chown() under the fake root: the kernel would say EPERM, and a container
/// says "done". Record it instead of dropping it, which is what makes the *next*
/// process see the same answer.
unsafe fn own_record_chown(
    dirfd: CInt,
    path: *const CChar,
    uid: u32,
    gid: u32,
    follow: bool,
    raw_flags: CInt,
) {
    if !owners_on() {
        return;
    }
    let mut st = [0u8; 128];
    let empty = path.is_null() || *path == 0;
    let r = if empty && raw_flags & AT_EMPTY_PATH != 0 {
        syscall4(SYS_FSTAT, dirfd as usize, st.as_mut_ptr() as usize, 0, 0)
    } else if empty {
        return;
    } else {
        fstatat(
            dirfd,
            path,
            st.as_mut_ptr(),
            if follow { 0 } else { AT_SYMLINK_NOFOLLOW },
        ) as Ssize
    };
    if r != 0 {
        return;
    }
    let dev = rd8u(st.as_ptr());
    if !own_fictional(dev) {
        return;
    }
    let ino = rd8u(st.as_ptr().add(8));
    // -1 means "leave this half alone", exactly as fchownat() defines it.
    let (mut nu, mut ng) = own_lookup(dev, ino);
    if uid != u32::MAX {
        nu = uid;
    }
    if gid != u32::MAX {
        ng = gid;
    }
    own_set(dev, ino, nu, ng);
}

/// Is there a record for this inode? Used to avoid a write when there is
/// nothing to clear.
unsafe fn own_has(dev: u64, ino: u64) -> bool {
    if OWN_STATE.load(Ordering::Acquire) != 2 || OWN_FILL == 0 {
        return false;
    }
    own_lock();
    let r = matches!(own_slot(dev, ino), (Some(_), true));
    own_unlock();
    r
}

/// A deleted inode can be handed out again; clear its record when the last name
/// goes away, or the next file to land there would answer with the old owner.
unsafe fn own_clear_path(dirfd: CInt, path: *const CChar) {
    if !owners_on() || path.is_null() || OWN_STATE.load(Ordering::Acquire) != 2 || OWN_FILL == 0 {
        return;
    }
    let mut st = [0u8; 128];
    if fstatat(dirfd, path, st.as_mut_ptr(), AT_SYMLINK_NOFOLLOW) != 0 {
        return;
    }
    let dev = rd8u(st.as_ptr());
    if !own_fictional(dev) {
        return;
    }
    // A hard link that keeps the inode alive keeps the owner with it.
    if rd4u(st.as_ptr().add(20)) != 1 {
        return;
    }
    let ino = rd8u(st.as_ptr().add(8));
    if own_has(dev, ino) {
        own_set(dev, ino, 0, 0);
    }
}

/// O_CREAT on a path that already exists is an open, not a create, and must not
/// re-own the file. Root needs no record at all (0:0 is the default), so the
/// extra stat is paid only when a *user* session is the one writing.
unsafe fn own_create_check(dirfd: CInt, path: *const CChar, flags: CInt) -> bool {
    if !owners_on() || path.is_null() || flags & 0o100 == 0 {
        return false;
    }
    let (uid, gid) = fake_id();
    if uid == 0 && gid == 0 {
        return false;
    }
    let mut st = [0u8; 128];
    fstatat(dirfd, path, st.as_mut_ptr(), AT_SYMLINK_NOFOLLOW) != 0
}

#[inline]
unsafe fn wr4(p: *mut u8, v: u32) {
    let mut i = 0;
    while i < 4 {
        core::ptr::write_volatile(p.add(i), (v >> (8 * i)) as u8);
        i += 1;
    }
}
// The other half of the same fiction: apt drops to `_apt` for downloads and back
// to root for the install. Neither call can succeed for us (the process is
// u0_a302 and stays u0_a302), so under NR_FAKE_ROOT they are accepted and
// ignored - the process never actually changes identity either way.
#[no_mangle]
pub unsafe extern "C" fn setuid(_uid: u32) -> CInt {
    log_id(b"setuid", _uid as i64);
    if fake_root_on() {
        fake_id();
        FAKE_UID = _uid;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETUID, _uid as usize, 0, 0, 0))
}

#[no_mangle]
pub unsafe extern "C" fn seteuid(_uid: u32) -> CInt {
    log_id(b"seteuid", _uid as i64);
    if fake_root_on() {
        fake_id();
        FAKE_UID = _uid;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETRESUID, usize::MAX, _uid as usize, usize::MAX, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setgid(_gid: u32) -> CInt {
    log_id(b"setgid", _gid as i64);
    if fake_root_on() {
        fake_id();
        FAKE_GID = _gid;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETGID, _gid as usize, 0, 0, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setegid(_gid: u32) -> CInt {
    log_id(b"setegid", _gid as i64);
    if fake_root_on() {
        fake_id();
        FAKE_GID = _gid;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETRESGID, usize::MAX, _gid as usize, usize::MAX, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setgroups(_n: usize, _list: *const u32) -> CInt {
    log_id(b"setgroups", _n as i64);
    if fake_root_on() { 0 } else { ret_or_errno(syscall4(SYS_SETGROUPS, _n, _list as usize, 0, 0)) }
}

#[no_mangle]
pub unsafe extern "C" fn setresuid(_r: u32, _e: u32, _s: u32) -> CInt {
    log_id(b"setresuid", _r as i64);
    if fake_root_on() {
        fake_id();
        // -(uid_t)1 means "leave this one alone", as setresuid(2) says.
        let v = if _r != u32::MAX {
            _r
        } else if _e != u32::MAX {
            _e
        } else {
            _s
        };
        if v != u32::MAX {
            FAKE_UID = v;
        }
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETRESUID, _r as usize, _e as usize, _s as usize, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setreuid(_r: u32, _e: u32) -> CInt {
    log_id(b"setreuid", _r as i64);
    if fake_root_on() {
        fake_id();
        let v = if _r != u32::MAX {
            _r
        } else if _e != u32::MAX {
            _e
        } else {
            FAKE_UID
        };
        FAKE_UID = v;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETREUID, _r as usize, _e as usize, 0, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setregid(_r: u32, _e: u32) -> CInt {
    log_id(b"setregid", _r as i64);
    if fake_root_on() {
        fake_id();
        let v = if _r != u32::MAX {
            _r
        } else if _e != u32::MAX {
            _e
        } else {
            FAKE_GID
        };
        FAKE_GID = v;
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETREGID, _r as usize, _e as usize, 0, 0))
}

#[no_mangle]
pub unsafe extern "C" fn getresuid(r: *mut u32, e: *mut u32, s: *mut u32) -> CInt {
    if fake_root_on() {
        let id = fake_id();
        if !r.is_null() {
            *r = id.0;
        }
        if !e.is_null() {
            *e = id.0;
        }
        if !s.is_null() {
            *s = id.0;
        }
        return 0;
    }
    ret_or_errno(syscall4(SYS_GETRESUID, r as usize, e as usize, s as usize, 0))
}

#[no_mangle]
pub unsafe extern "C" fn getresgid(r: *mut u32, e: *mut u32, s: *mut u32) -> CInt {
    if fake_root_on() {
        let id = fake_id();
        if !r.is_null() {
            *r = id.1;
        }
        if !e.is_null() {
            *e = id.1;
        }
        if !s.is_null() {
            *s = id.1;
        }
        return 0;
    }
    ret_or_errno(syscall4(SYS_GETRESGID, r as usize, e as usize, s as usize, 0))
}

#[no_mangle]
pub unsafe extern "C" fn setresgid(_r: u32, _e: u32, _s: u32) -> CInt {
    log_id(b"setresgid", _r as i64);
    if fake_root_on() {
        fake_id();
        let v = if _r != u32::MAX {
            _r
        } else if _e != u32::MAX {
            _e
        } else {
            _s
        };
        if v != u32::MAX {
            FAKE_GID = v;
        }
        return 0;
    }
    ret_or_errno(syscall4(SYS_SETRESGID, _r as usize, _e as usize, _s as usize, 0))
}

// mknod(2) had no translation at all, so `mkfifo /tmp/x` asked the kernel for
// the *host* "/tmp/x" and came back ENOENT. FIFOs are ordinary tooling (daemons
// that use mkfifo + a reader, tar extraction of pipes); char/block nodes are not
// creatable without CAP_MKNOD, which is a separate wall.
#[no_mangle]
pub unsafe extern "C" fn mknodat(dirfd: CInt, path: *const CChar, mode: CInt, dev: u64) -> CInt {
    let r = pathop(dirfd, path, false, |fd, p| {
        syscall5(
            SYS_MKNODAT,
            fd as usize,
            p as usize,
            mode as usize,
            dev as usize,
            0,
        )
    });
    if r == 0 {
        own_note_path(dirfd, path);
    }
    r
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
        true,
        true,
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
    pathop2(afd, from, bfd, to, false, false, true, true, |fa, pa, fb, pb| {
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
    pathop2(afd, from, bfd, to, false, false, true, true, |fa, pa, fb, pb| {
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
    let r = pathop(AT_FDCWD, linkpath, false, |fd, p| {
        syscall4(SYS_SYMLINKAT, target as usize, fd as usize, p as usize, 0)
    });
    if r == 0 {
        own_note_path(AT_FDCWD, linkpath);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat(
    target: *const CChar,
    dirfd: CInt,
    linkpath: *const CChar,
) -> CInt {
    let r = pathop(dirfd, linkpath, false, |fd, p| {
        syscall4(SYS_SYMLINKAT, target as usize, fd as usize, p as usize, 0)
    });
    if r == 0 {
        own_note_path(dirfd, linkpath);
    }
    r
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
        false,
        true,
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
    let follow_old = flags & AT_SYMLINK_FOLLOW != 0;
    let make_link = |fa: CInt, pa: *const CChar, fb: CInt, pb: *const CChar, fl: usize| {
        let r = syscall5(SYS_LINKAT, fa as usize, pa as usize, fb as usize, pb as usize, fl);
        if r >= 0 {
            return r;
        }
        // Raw syscalls return -errno; errno itself is only written by the
        // wrappers, so the decision has to be made on the return value.
        if r == -13 || r == -1 {
            return copy_file(fa, pa, fb, pb);
        }
        r
    };
    if follow_old {
        // Pin the old object first. Resolving its final symlink and then still
        // passing AT_SYMLINK_FOLLOW let the kernel look that name up a second
        // time, so a swap to an absolute symlink resolved against the host
        // root. `/proc/self/fd/N` denotes the inode; there is no name to swap.
        let fd = match target_fd(ofd, old, true) {
            Ok(fd) => fd,
            Err(e) => return set_errno(e),
        };
        let mut pb = [0u8; 32];
        let pp = fd_proc_path(fd, &mut pb);
        let r = pathop2(
            AT_FDCWD,
            pp.as_ptr() as *const CChar,
            nfd,
            new,
            false,
            false,
            false,
            true,
            |fa, pa, fb, qb| make_link(fa, pa, fb, qb, AT_SYMLINK_FOLLOW as usize),
        );
        rc::close(fd);
        return r;
    }
    pathop2(ofd, old, nfd, new, false, false, false, true, |fa, pa, fb, pb| {
        make_link(fa, pa, fb, pb, 0)
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
        return ssize_or_errno(syscall4(
            SYS_READLINKAT,
            dirfd as usize,
            path as usize,
            out as usize,
            len,
        ));
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
            return ssize_or_errno(syscall4(
                SYS_READLINKAT,
                AT_FDCWD as usize,
                gp.as_ptr() as usize,
                out as usize,
                len,
            ))
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
    ssize_or_errno(r)
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
    if fd >= 0 && nl_is_fake(fd) {
        nl_forget(fd);
    }
    alias_refresh(fd);
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
    if r == 0 {
        own_patch_statx(buf);
    }
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

// glibc's scandir() is a separate entry point and does not go through opendir.
// dpkg-deb uses it to enumerate its temporary control directory, so without
// this interposer the create happens in the guest but the scan happens in the
// host /tmp (measured on the RMX3357 test device).
type ScandirSelect = Option<unsafe extern "C" fn(*const u8) -> CInt>;
type ScandirCompar = Option<unsafe extern "C" fn(*const *const u8, *const *const u8) -> CInt>;

unsafe fn scandir_free(list: *mut *mut u8, count: usize) {
    if list.is_null() {
        return;
    }
    let mut i = 0;
    while i < count {
        free(*list.add(i));
        i += 1;
    }
    free(list as *mut u8);
}

unsafe fn scandir_sort(list: *mut *mut u8, count: usize, compar: ScandirCompar) {
    let Some(compar) = compar else { return };
    let mut i = 1usize;
    while i < count {
        let value = *list.add(i);
        let mut j = i;
        while j > 0 {
            let prev = *list.add(j - 1);
            let lhs = &prev as *const *mut u8 as *const *const u8;
            let rhs = &value as *const *mut u8 as *const *const u8;
            if compar(lhs, rhs) <= 0 {
                break;
            }
            *list.add(j) = prev;
            j -= 1;
        }
        *list.add(j) = value;
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn scandir(
    path: *const CChar,
    namelist: *mut *mut *mut u8,
    select: ScandirSelect,
    compar: ScandirCompar,
) -> CInt {
    if namelist.is_null() {
        *__errno_location() = 22; // EINVAL
        return -1;
    }
    *namelist = core::ptr::null_mut();

    let dir = opendir(path);
    if dir.is_null() {
        return -1;
    }

    let item_size = core::mem::size_of::<*mut u8>();
    let mut capacity = 8usize;
    let mut list = malloc(capacity * item_size) as *mut *mut u8;
    if list.is_null() {
        closedir(dir);
        *__errno_location() = 12; // ENOMEM
        return -1;
    }

    let mut count = 0usize;
    loop {
        *__errno_location() = 0;
        let ent = readdir(dir);
        if ent.is_null() {
            let error = *__errno_location();
            if error != 0 {
                scandir_free(list, count);
                closedir(dir);
                *__errno_location() = error;
                return -1;
            }
            break;
        }
        if let Some(select) = select {
            if select(ent) == 0 {
                continue;
            }
        }

        if count == capacity {
            let Some(next) = capacity.checked_mul(2) else {
                scandir_free(list, count);
                closedir(dir);
                *__errno_location() = 12; // ENOMEM
                return -1;
            };
            let Some(bytes) = next.checked_mul(item_size) else {
                scandir_free(list, count);
                closedir(dir);
                *__errno_location() = 12; // ENOMEM
                return -1;
            };
            let grown = realloc(list as *mut u8, bytes) as *mut *mut u8;
            if grown.is_null() {
                scandir_free(list, count);
                closedir(dir);
                *__errno_location() = 12; // ENOMEM
                return -1;
            }
            list = grown;
            capacity = next;
        }

        // aarch64 glibc dirent: d_ino +0, d_off +8, d_reclen +16,
        // d_type +18, d_name +19. Copy the kernel-provided record verbatim so
        // the caller owns an ordinary libc dirent and can free() it itself.
        let reclen = core::ptr::read_unaligned(ent.add(16) as *const u16) as usize;
        if reclen < 19 {
            scandir_free(list, count);
            closedir(dir);
            *__errno_location() = 5; // EIO
            return -1;
        }
        let copy = malloc(reclen);
        if copy.is_null() {
            scandir_free(list, count);
            closedir(dir);
            *__errno_location() = 12; // ENOMEM
            return -1;
        }
        core::ptr::copy_nonoverlapping(ent, copy, reclen);
        *list.add(count) = copy;
        count += 1;
    }

    closedir(dir);
    scandir_sort(list, count, compar);
    *namelist = list;
    count as CInt
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

/// The xattr family has no *at() form, and rebuilding "<parent>/<name>" leaves
/// the kernel free to resolve the last component again: a rename of that name
/// to an absolute symlink between our resolve and the syscall makes its target
/// resolve against the host root. Pin the object as an O_PATH descriptor and
/// hand the kernel `/proc/self/fd/N` instead - an inode, not a name.
unsafe fn xattr_pinned<F>(path: *const CChar, f: F) -> Ssize
where
    F: FnOnce(*const CChar) -> Ssize,
{
    if path.is_null() {
        *__errno_location() = 14;
        return -1;
    }
    let fd = match target_fd(AT_FDCWD, path, true) {
        Ok(fd) => fd,
        Err(e) => {
            *__errno_location() = e;
            return -1;
        }
    };
    let mut pb = [0u8; 32];
    let pp = fd_proc_path(fd, &mut pb);
    let r = f(pp.as_ptr() as *const CChar);
    rc::close(fd);
    r
}

#[no_mangle]
pub unsafe extern "C" fn lgetxattr(
    path: *const CChar,
    name: *const CChar,
    value: *mut u8,
    size: CSizet,
) -> Ssize {
    ssize_or_errno(pathonly(path, false, |p| {
        syscall5(
            SYS_LGETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            0,
        )
    }))
}

#[no_mangle]
pub unsafe extern "C" fn getxattr(
    path: *const CChar,
    name: *const CChar,
    value: *mut u8,
    size: CSizet,
) -> Ssize {
    ssize_or_errno(xattr_pinned(path, |p| {
        syscall5(
            SYS_GETXATTR,
            p as usize,
            name as usize,
            value as usize,
            size,
            0,
        )
    }))
}

#[no_mangle]
pub unsafe extern "C" fn setxattr(
    path: *const CChar,
    name: *const CChar,
    value: *const u8,
    size: CSizet,
    flags: CInt,
) -> CInt {
    if ro_refuse(path) {
        return -1;
    }
    ret_or_errno(xattr_pinned(path, |p| {
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
    if ro_refuse(path) {
        return -1;
    }
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
    ssize_or_errno(xattr_pinned(path, |p| {
        syscall4(SYS_LISTXATTR, p as usize, list as usize, size, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn llistxattr(path: *const CChar, list: *mut CChar, size: CSizet) -> Ssize {
    ssize_or_errno(pathonly(path, false, |p| {
        syscall4(SYS_LLISTXATTR, p as usize, list as usize, size, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn removexattr(path: *const CChar, name: *const CChar) -> CInt {
    if ro_refuse(path) {
        return -1;
    }
    ret_or_errno(xattr_pinned(path, |p| {
        syscall4(SYS_REMOVEXATTR, p as usize, name as usize, 0, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn lremovexattr(path: *const CChar, name: *const CChar) -> CInt {
    if ro_refuse(path) {
        return -1;
    }
    ret_or_errno(pathonly(path, false, |p| {
        syscall4(SYS_LREMOVEXATTR, p as usize, name as usize, 0, 0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn statfs(path: *const CChar, buf: *mut u8) -> CInt {
    ret_or_errno(xattr_pinned(path, |p| {
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
    // The path exactly as this call named it (guest-absolute, NUL terminated).
    // A script's interpreter has to receive *this* string and not the resolved
    // target, because that is what the kernel hands over - and it is what makes
    // a symlink dispatcher work at all: /usr/sbin/addgroup is a symlink to the
    // adduser script and decides what to do from its own $0.
    let mut as_given = [0u8; PATH_MAX];
    if n < PATH_MAX {
        as_given[..n].copy_from_slice(&gp[..n]);
        as_given[n] = 0;
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
        sargs[k] = as_given.as_ptr() as *const CChar;
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
    // `--library-path` replaces the loader's normal search path, so every
    // exec must inherit the same complete list as the top-level process.
    // Rebuilding only `<root>/usr/lib/aarch64-linux-gnu` here dropped private
    // package directories (measured first with systemd's
    // libsystemd-shared-255.so) and made otherwise valid postinst programs
    // fail with "cannot open shared object file". The launcher exports the
    // exact path it gave the loader; the fallback keeps direct device tests
    // working when they use only NR_ROOTFS/NR_LD.
    if exec_library_path(root, envp, &mut lib).is_none() {
        return set_errno(36);
    }

    let mut args: [*const CChar; 64] = [core::ptr::null(); 64];
    let mut k = 0usize;
    args[k] = ld2.as_ptr() as *const CChar;
    k += 1;
    args[k] = b"--library-path\0".as_ptr() as *const CChar;
    k += 1;
    args[k] = lib.as_ptr() as *const CChar;
    k += 1;
    // argv[0] the way the caller wrote it. The program argument below is a
    // *resolved* path, so without this every symlink-dispatched program sees
    // its target's name; that is what turned `addgroup --system X` into an
    // adduser call (evidence 137).
    if !argv.is_null() && !(*argv).is_null() {
        args[k] = b"--argv0\0".as_ptr() as *const CChar;
        k += 1;
        args[k] = *argv;
        k += 1;
    }
    args[k] = tgt.as_ptr() as *const CChar;
    k += 1;
    if is_script {
        if shebang_arg_len > 0 && k < 62 {
            args[k] = shebang_arg.as_ptr() as *const CChar;
            k += 1;
        }
        if k < 62 {
            args[k] = as_given.as_ptr() as *const CChar;
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

const DEFAULT_LIBRARY_DIRS: &[&[u8]] = &[
    b"lib/aarch64-linux-gnu",
    b"usr/lib/aarch64-linux-gnu",
    b"usr/lib/aarch64-linux-gnu/systemd",
    b"usr/lib/aarch64-linux-gnu/mutter-14",
    b"usr/lib/gnome-shell",
    b"usr/lib/aarch64-linux-gnu/gvfs",
];

/// Build the loader `--library-path` used by nested execs.
///
/// NR_LIBRARY_PATH is an internal launcher-to-shim contract: it contains the
/// absolute host paths the top-level loader received. It is optional for
/// callers that only set the documented NR_ROOTFS/NR_LD pair; in that case we
/// derive the same multiarch/private directories below the guest root.
unsafe fn exec_library_path(
    root: &[u8],
    envp: *const *const CChar,
    out: &mut [u8; PATH_MAX],
) -> Option<usize> {
    let env = if envp.is_null() {
        environ as *const *const CChar
    } else {
        envp
    };
    if let Some(p) = env_lookup_list(env, b"NR_LIBRARY_PATH") {
        if !p.is_empty() && p.len() < PATH_MAX {
            out[..p.len()].copy_from_slice(p);
            out[p.len()] = 0;
            return Some(p.len());
        }
    }
    let mut n = 0usize;
    let mut i = 0usize;
    while i < DEFAULT_LIBRARY_DIRS.len() {
        if i != 0 {
            if n >= PATH_MAX {
                return None;
            }
            out[n] = b':';
            n += 1;
        }
        let dir = DEFAULT_LIBRARY_DIRS[i];
        if root.len() + 1 + dir.len() + 1 > PATH_MAX {
            return None;
        }
        out[n..n + root.len()].copy_from_slice(root);
        n += root.len();
        out[n] = b'/';
        n += 1;
        out[n..n + dir.len()].copy_from_slice(dir);
        n += dir.len();
        i += 1;
    }
    out[n] = 0;
    Some(n)
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
    // A netlink socket we handed out ourselves: the address is bookkeeping the
    // caller asked for, and there is no kernel object behind it to bind.
    if nl_is_fake(sockfd) {
        return 0;
    }
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
    // `pid` may legitimately be NULL. glibc's spawni.c only writes the child
    // pid through it when the caller passed a pointer, and Ubuntu's localedef
    // passes NULL while piping a compressed charmap through `gzip -d -c`
    // (measured on the guest `localedef`; see evidence 133). Rejecting NULL
    // here made every `.gz` charmap look missing: `localedef -f UTF-8` said
    // "character map file `UTF-8' not found" although `UTF-8.gz` opened fine.
    if path.is_null() {
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
    if !pid.is_null() {
        *pid = child;
    }
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
    let r = ret_or_errno(syscall4(SYS_FSTAT, fd as usize, st as usize, 0, 0));
    if r == 0 {
        own_patch_stat(st);
    }
    r
}

// fstat() is its own exported symbol on glibc, and the keeper of a descriptor
// is exactly who asks "whose is this?" before it trusts the fd.
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: CInt, st: *mut u8) -> CInt {
    fstat64(fd, st)
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
            plan::log_plan_once(b"mali0 open");
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

// ------------------------------------------------------- SysV IPC: shm ---
//
// The kernel here has SysV IPC compiled out: `shmget`, `semget` and `msgget`
// answer ENOSYS even from a plain shell, and `/proc/sysvipc` does not exist.
// A container still has to hand programs shared memory the old way — X11,
// PostgreSQL, Qt's shared pixmaps and a lot of older daemons all ask for it —
// so the service is rebuilt from what a namespace of this shape does have:
// files and mmap.
//
//   /run/husk/ipc/shm/<id>     the segment
//   /run/husk/ipc/shm/<id>.m   key size mode cuid cgid cpid ctime rmid
//   /run/husk/ipc/shm/<id>.a   one line per attached (host) pid
//   /run/husk/ipc/keys         "key id" pairs
//   /run/husk/ipc/nextid       the id counter
//   /run/husk/ipc/lock         flock() target for every metadata operation
//
// `shmat` is `mmap(MAP_SHARED)` on the segment, so the pages really are shared
// between guest processes and the kernel does the coherence; only the
// bookkeeping is ours.

const IPC_ROOT: &[u8] = b"/run/husk/ipc\0";
const IPC_SHM: &[u8] = b"/run/husk/ipc/shm\0";
const IPC_LOCK: &[u8] = b"/run/husk/ipc/lock\0";
const IPC_NEXTID: &[u8] = b"/run/husk/ipc/nextid\0";
const IPC_KEYS: &[u8] = b"/run/husk/ipc/keys\0";

const SYS_FLOCK_NR: usize = 32;
const SYS_MUNMAP_NR: usize = 215;

const IPC_CREAT: CInt = 0o1000;
const IPC_EXCL: CInt = 0o2000;
const SHM_RDONLY: CInt = 0o10000;
const IPC_RMID: CInt = 0;
const IPC_SET: CInt = 1;
const IPC_STAT: CInt = 2;
const IPC_INFO: CInt = 3;
const SHM_LOCK: CInt = 11;
const SHM_UNLOCK: CInt = 12;

#[derive(Clone, Copy)]
struct ShmMeta {
    key: i32,
    size: u64,
    mode: u32,
    cuid: u32,
    cgid: u32,
    cpid: i32,
    ctime: i64,
    rmid: i32,
}

impl ShmMeta {
    const fn empty() -> Self {
        ShmMeta { key: 0, size: 0, mode: 0o660, cuid: 0, cgid: 0, cpid: 0, ctime: 0, rmid: 0 }
    }
}

static IPC_LOCKW: AtomicU8 = AtomicU8::new(0);
// Attached segments of this process: (mapped address, segment id).
static mut SHM_ATT: [(usize, i32); 64] = [(0, -1); 64];

unsafe fn ipc_lock() {
    while IPC_LOCKW
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

unsafe fn ipc_unlock() {
    IPC_LOCKW.store(0, Ordering::Release);
}

unsafe fn ipc_put_dec(out: &mut [u8], at: &mut usize, v: i64) {
    let mut d = [0u8; 20];
    let mut n = 0usize;
    let neg = v < 0;
    let mut u = if neg { (-(v as i128)) as u64 } else { v as u64 };
    if u == 0 {
        d[0] = b'0';
        n = 1;
    }
    while u != 0 {
        d[n] = b'0' + (u % 10) as u8;
        u /= 10;
        n += 1;
    }
    if neg {
        out[*at] = b'-';
        *at += 1;
    }
    while n > 0 {
        n -= 1;
        out[*at] = d[n];
        *at += 1;
    }
}

/// Parses whitespace separated integers; returns how many it filled.
unsafe fn ipc_parse_ints(buf: &[u8], out: &mut [i64]) -> usize {
    let mut n = 0usize;
    let mut i = 0usize;
    while i < buf.len() && n < out.len() {
        while i < buf.len() && (buf[i] == b' ' || buf[i] == b'\n' || buf[i] == b'\t') {
            i += 1;
        }
        if i >= buf.len() {
            break;
        }
        let mut neg = false;
        if buf[i] == b'-' {
            neg = true;
            i += 1;
        }
        let mut v = 0i64;
        let mut any = false;
        while i < buf.len() && buf[i].is_ascii_digit() {
            v = v * 10 + (buf[i] - b'0') as i64;
            i += 1;
            any = true;
        }
        if !any {
            break;
        }
        out[n] = if neg { -v } else { v };
        n += 1;
    }
    n
}

unsafe fn ipc_mkdirs() {
    mkdirat(AT_FDCWD, IPC_ROOT.as_ptr() as *const CChar, 0o700);
    mkdirat(AT_FDCWD, IPC_SHM.as_ptr() as *const CChar, 0o700);
}

/// Takes the metadata lock. Returns the descriptor, or -1 with errno set.
unsafe fn ipc_lockf() -> CInt {
    let fd = openat(AT_FDCWD, IPC_LOCK.as_ptr() as *const CChar, 0o2 | 0o100, 0o600);
    if fd < 0 {
        return -1;
    }
    let r = syscall4(SYS_FLOCK_NR, fd as usize, 2 /* LOCK_EX */, 0, 0);
    if r < 0 {
        rc::close(fd);
        return ret_or_errno(r);
    }
    fd
}

unsafe fn ipc_unlockf(fd: CInt) {
    syscall4(SYS_FLOCK_NR, fd as usize, 8 /* LOCK_UN */, 0, 0);
    rc::close(fd);
}

/// Copies `pref` + decimal `v` + `suf` (NUL terminated) into `out`.
unsafe fn ipc_name(pref: &[u8], v: i64, suf: &[u8], out: &mut [u8; 96]) -> usize {
    let mut at = 0usize;
    out[..pref.len()].copy_from_slice(pref);
    at += pref.len();
    ipc_put_dec(out, &mut at, v);
    out[at..at + suf.len()].copy_from_slice(suf);
    at += suf.len();
    out[at] = 0;
    at
}

unsafe fn ipc_read_file(path: &[u8], buf: &mut [u8]) -> isize {
    let fd = openat(AT_FDCWD, path.as_ptr() as *const CChar, 0, 0);
    if fd < 0 {
        return -1;
    }
    let r = syscall4(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
    rc::close(fd);
    r
}

unsafe fn ipc_write_file(path: &[u8], data: &[u8]) -> CInt {
    let fd = openat(
        AT_FDCWD,
        path.as_ptr() as *const CChar,
        0o1 | 0o100 | 0o1000, // WRONLY|CREAT|TRUNC
        0o600,
    );
    if fd < 0 {
        return -1;
    }
    let mut left = data.len();
    let mut p = data.as_ptr();
    while left > 0 {
        let w = syscall4(SYS_WRITE, fd as usize, p as usize, left, 0) as isize;
        if w <= 0 {
            rc::close(fd);
            return set_errno(5);
        }
        left -= w as usize;
        p = p.add(w as usize);
    }
    rc::close(fd);
    0
}

unsafe fn shm_meta_path(id: i32, out: &mut [u8; 96]) -> usize {
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b".m\0", out);
    out.iter().position(|c| *c == 0).unwrap_or(0)
}

unsafe fn shm_read_meta(id: i32, m: &mut ShmMeta) -> bool {
    let mut p = [0u8; 96];
    let n = shm_meta_path(id, &mut p);
    let mut buf = [0u8; 256];
    let r = ipc_read_file(&p[..n], &mut buf);
    if r <= 0 {
        return false;
    }
    let mut v = [0i64; 8];
    if ipc_parse_ints(&buf[..r as usize], &mut v) < 8 {
        return false;
    }
    *m = ShmMeta {
        key: v[0] as i32,
        size: v[1] as u64,
        mode: v[2] as u32,
        cuid: v[3] as u32,
        cgid: v[4] as u32,
        cpid: v[5] as i32,
        ctime: v[6],
        rmid: v[7] as i32,
    };
    true
}

unsafe fn shm_write_meta(id: i32, m: &ShmMeta) -> CInt {
    let mut p = [0u8; 96];
    let n = shm_meta_path(id, &mut p);
    let mut b = [0u8; 160];
    let mut at = 0usize;
    for v in [
        m.key as i64,
        m.size as i64,
        m.mode as i64,
        m.cuid as i64,
        m.cgid as i64,
        m.cpid as i64,
        m.ctime,
        m.rmid as i64,
    ] {
        ipc_put_dec(&mut b, &mut at, v);
        b[at] = b' ';
        at += 1;
    }
    b[at] = b'\n';
    at += 1;
    ipc_write_file(&p[..n], &b[..at])
}

/// Live attachments, and prunes the entries of processes that are gone.
unsafe fn shm_att_count(id: i32) -> i64 {
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b".a\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    let mut buf = [0u8; 1024];
    let r = ipc_read_file(&p[..n], &mut buf);
    let mut count = 0i64;
    if r > 0 {
        let mut v = [0i64; 128];
        let got = ipc_parse_ints(&buf[..r as usize], &mut v);
        for i in 0..got {
            let pid = v[i] as CInt;
            if pid > 0 && syscall4(SYS_KILL_NR, pid as usize, 0, 0, 0) == 0 {
                count += 1;
            }
        }
    }
    count
}

unsafe fn shm_att_add(id: i32) {
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b".a\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    let fd = openat(
        AT_FDCWD,
        p.as_ptr() as *const CChar,
        0o2 | 0o100 | 0o2000, // RDWR|CREAT|APPEND
        0o600,
    );
    if fd < 0 {
        return;
    }
    let mut b = [0u8; 24];
    let mut at = 0usize;
    ipc_put_dec(&mut b, &mut at, syscall4(172, 0, 0, 0, 0) as i64); // getpid
    b[at] = b'\n';
    at += 1;
    syscall4(SYS_WRITE, fd as usize, b.as_ptr() as usize, at, 0);
    rc::close(fd);
}

unsafe fn shm_att_remove(id: i32) {
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b".a\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    let mut buf = [0u8; 1024];
    let r = ipc_read_file(&p[..n], &mut buf);
    if r <= 0 {
        return;
    }
    let me = syscall4(172, 0, 0, 0, 0) as i64;
    let mut v = [0i64; 128];
    let got = ipc_parse_ints(&buf[..r as usize], &mut v);
    let mut out = [0u8; 1024];
    let mut at = 0usize;
    let mut dropped = false;
    for i in 0..got {
        if !dropped && v[i] == me {
            dropped = true;
            continue;
        }
        ipc_put_dec(&mut out, &mut at, v[i]);
        out[at] = b'\n';
        at += 1;
    }
    ipc_write_file(&p[..n], &out[..at]);
}

unsafe fn keys_find(key: i32) -> i32 {
    let mut buf = [0u8; 4096];
    let r = ipc_read_file(IPC_KEYS, &mut buf);
    if r <= 0 {
        return -1;
    }
    let mut v = [0i64; 512];
    let got = ipc_parse_ints(&buf[..r as usize], &mut v);
    let mut i = 0usize;
    while i + 1 < got {
        if v[i] as i32 == key {
            return v[i + 1] as i32;
        }
        i += 2;
    }
    -1
}

unsafe fn keys_add(key: i32, id: i32) {
    let fd = openat(
        AT_FDCWD,
        IPC_KEYS.as_ptr() as *const CChar,
        0o2 | 0o100 | 0o2000,
        0o600,
    );
    if fd < 0 {
        return;
    }
    let mut b = [0u8; 48];
    let mut at = 0usize;
    ipc_put_dec(&mut b, &mut at, key as i64);
    b[at] = b' ';
    at += 1;
    ipc_put_dec(&mut b, &mut at, id as i64);
    b[at] = b'\n';
    at += 1;
    syscall4(SYS_WRITE, fd as usize, b.as_ptr() as usize, at, 0);
    rc::close(fd);
}

/// Drops key lines that point at `id`. A dead segment must not leave a key
/// behind: the next `shmget(key, O_CREAT)` would find the stale id, fail to read
/// its meta, and answer ENOENT instead of creating a fresh segment.
unsafe fn keys_remove_id(id: i32) {
    let mut buf = [0u8; 4096];
    let r = ipc_read_file(IPC_KEYS, &mut buf);
    if r <= 0 {
        return;
    }
    let mut v = [0i64; 512];
    let got = ipc_parse_ints(&buf[..r as usize], &mut v);
    let mut out = [0u8; 4096];
    let mut at = 0usize;
    let mut i = 0usize;
    while i + 1 < got {
        if v[i + 1] as i32 != id {
            ipc_put_dec(&mut out, &mut at, v[i]);
            out[at] = b' ';
            at += 1;
            ipc_put_dec(&mut out, &mut at, v[i + 1]);
            out[at] = b'\n';
            at += 1;
        }
        i += 2;
    }
    ipc_write_file(IPC_KEYS, &out[..at]);
}

/// New id from the counter file. The caller holds the lock.
unsafe fn ipc_nextid() -> i32 {
    let mut buf = [0u8; 64];
    let r = ipc_read_file(IPC_NEXTID, &mut buf);
    let mut v = [0i64; 1];
    let cur = if r > 0 && ipc_parse_ints(&buf[..r as usize], &mut v) == 1 {
        v[0]
    } else {
        0
    };
    let next = cur + 1;
    let mut b = [0u8; 24];
    let mut at = 0usize;
    ipc_put_dec(&mut b, &mut at, next);
    b[at] = b'\n';
    at += 1;
    ipc_write_file(IPC_NEXTID, &b[..at]);
    next as i32
}

unsafe fn shm_destroy(id: i32) {
    keys_remove_id(id);
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b"\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
    if shm_meta_path(id, &mut p) > 0 {
        let n = p.iter().position(|c| *c == 0).unwrap_or(0);
        unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
    }
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b".a\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
}

#[no_mangle]
pub unsafe extern "C" fn shmget(key: i32, size: usize, shmflg: CInt) -> CInt {
    ipc_mkdirs();
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = shmget_locked(key, size, shmflg);
    ipc_unlockf(lf);
    r
}

unsafe fn shmget_locked(key: i32, size: usize, shmflg: CInt) -> CInt {
    let create = shmflg & IPC_CREAT != 0;
    let excl = shmflg & IPC_EXCL != 0;
    let private = key == 0;
    if !private {
        let id = keys_find(key);
        if id >= 0 {
            let mut m = ShmMeta::empty();
            if !shm_read_meta(id, &mut m) {
                // The segment died but its key line survived; drop it and let
                // the create path below do its job.
                keys_remove_id(id);
            } else {
            if create && excl {
                return set_errno(17); // EEXIST
            }
            if size != 0 && size as u64 > m.size {
                return set_errno(22); // EINVAL
            }
            return id;
            }
        }
        if !create {
            return set_errno(2); // ENOENT
        }
    }
    if size == 0 {
        return set_errno(22);
    }
    let id = ipc_nextid();
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", id as i64, b"\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    let fd = openat(
        AT_FDCWD,
        p.as_ptr() as *const CChar,
        0o2 | 0o100, // RDWR|CREAT
        0o600,
    );
    if fd < 0 {
        return -1;
    }
    let tr = syscall4(46 /* ftruncate */, fd as usize, size, 0, 0);
    rc::close(fd);
    if tr < 0 {
        return ret_or_errno(tr);
    }
    let mut now = [0i64; 2];
    syscall4(113, 0, now.as_mut_ptr() as usize, 0, 0); // clock_gettime(REALTIME)
    let m = ShmMeta {
        key,
        size: size as u64,
        mode: (shmflg as u32) & 0o777,
        cuid: 0,
        cgid: 0,
        cpid: syscall4(172, 0, 0, 0, 0) as i32,
        ctime: now[0],
        rmid: 0,
    };
    shm_write_meta(id, &m);
    if !private {
        keys_add(key, id);
    }
    id
}

#[no_mangle]
pub unsafe extern "C" fn shmat(shmid: CInt, shmaddr: usize, shmflg: CInt) -> usize {
    ipc_lock();
    let r = shmat_inner(shmid, shmaddr, shmflg);
    ipc_unlock();
    r
}

unsafe fn shmat_inner(shmid: CInt, shmaddr: usize, shmflg: CInt) -> usize {
    let mut m = ShmMeta::empty();
    if !shm_read_meta(shmid, &mut m) {
        *__errno_location() = 22;
        return usize::MAX; // (void *) -1
    }
    let mut p = [0u8; 96];
    ipc_name(b"/run/husk/ipc/shm/", shmid as i64, b"\0", &mut p);
    let n = p.iter().position(|c| *c == 0).unwrap_or(0);
    let ro = shmflg & SHM_RDONLY != 0;
    let fd = openat(
        AT_FDCWD,
        p.as_ptr() as *const CChar,
        if ro { 0 } else { 0o2 },
        0,
    );
    if fd < 0 {
        return usize::MAX;
    }
    let prot = if ro { 1 } else { 1 | 2 };
    let addr = syscall6(
        SYS_MMAP,
        shmaddr,
        m.size as usize,
        prot,
        1, // MAP_SHARED
        fd as usize,
        0,
    );
    rc::close(fd);
    if addr as isize == -1 {
        return usize::MAX;
    }
    let slot = SHM_ATT.iter().position(|e| e.1 < 0);
    if let Some(i) = slot {
        SHM_ATT[i] = (addr as usize, shmid);
    }
    shm_att_add(shmid);
    addr as usize
}

#[no_mangle]
pub unsafe extern "C" fn shmdt(shmaddr: usize) -> CInt {
    ipc_lock();
    let mut found = -1i32;
    for i in 0..SHM_ATT.len() {
        if SHM_ATT[i].1 >= 0 && SHM_ATT[i].0 == shmaddr {
            found = SHM_ATT[i].1;
            SHM_ATT[i] = (0, -1);
            break;
        }
    }
    ipc_unlock();
    if found < 0 {
        return set_errno(22);
    }
    let r = syscall4(SYS_MUNMAP_NR, shmaddr, 0, 0, 0);
    let lf = ipc_lockf();
    if lf >= 0 {
        shm_att_remove(found);
        let mut m = ShmMeta::empty();
        if shm_read_meta(found, &mut m) && m.rmid != 0 && shm_att_count(found) == 0 {
            shm_destroy(found);
        }
        ipc_unlockf(lf);
    }
    ret_or_errno(r)
}

#[no_mangle]
pub unsafe extern "C" fn shmctl(shmid: CInt, cmd: CInt, buf: usize) -> CInt {
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = shmctl_locked(shmid, cmd, buf);
    ipc_unlockf(lf);
    r
}

unsafe fn shmctl_locked(shmid: CInt, cmd: CInt, buf: usize) -> CInt {
    let mut m = ShmMeta::empty();
    if !shm_read_meta(shmid, &mut m) {
        return set_errno(22);
    }
    match cmd {
        IPC_STAT => {
            if buf == 0 {
                return set_errno(14);
            }
            let nattch = shm_att_count(shmid);
            let b = buf as *mut u8;
            core::ptr::write_bytes(b, 0, 112);
            core::ptr::write_unaligned(b as *mut i32, m.key);
            core::ptr::write_unaligned((b.add(4)) as *mut u32, m.cuid);
            core::ptr::write_unaligned((b.add(8)) as *mut u32, m.cgid);
            core::ptr::write_unaligned((b.add(12)) as *mut u32, m.cuid);
            core::ptr::write_unaligned((b.add(16)) as *mut u32, m.cgid);
            core::ptr::write_unaligned((b.add(20)) as *mut u32, m.mode);
            core::ptr::write_unaligned((b.add(48)) as *mut u64, m.size);
            core::ptr::write_unaligned((b.add(72)) as *mut i64, m.ctime);
            core::ptr::write_unaligned((b.add(80)) as *mut i32, m.cpid);
            core::ptr::write_unaligned((b.add(88)) as *mut u64, nattch as u64);
            0
        }
        IPC_SET => {
            if buf == 0 {
                return set_errno(14);
            }
            let b = buf as *const u8;
            m.mode = core::ptr::read_unaligned((b.add(20)) as *const u32);
            m.cuid = core::ptr::read_unaligned((b.add(12)) as *const u32);
            m.cgid = core::ptr::read_unaligned((b.add(16)) as *const u32);
            shm_write_meta(shmid, &m)
        }
        IPC_RMID => {
            m.rmid = 1;
            shm_write_meta(shmid, &m);
            // The segment dies when the last process detaches; if nobody is
            // attached the grace period is already over.
            if shm_att_count(shmid) == 0 {
                shm_destroy(shmid);
            }
            0
        }
        SHM_LOCK | SHM_UNLOCK => 0, // nothing to pin: the pages are file-backed
        IPC_INFO => set_errno(22),
        _ => set_errno(22),
    }
}

// ------------------------------------------------ SysV IPC: semaphores ---
//
// Same problem as shared memory (`semget` is ENOSYS here) and the same shape
// of answer: the counting semaphores live in a small file, and every
// check-and-update happens under `flock` on that file, so the all-or-nothing
// rule of `semop` holds across processes without a broker in between.
//
//   /run/husk/ipc/sem/<id>     nsems int32 values
//   /run/husk/ipc/sem/<id>.m   key nsems mode rmid

const IPC_SEM: &[u8] = b"/run/husk/ipc/sem\0";
const SYS_PREAD: usize = 67;
const SYS_PWRITE: usize = 68;
const SEM_MAX: usize = 128;
const SEM_UNDO: CInt = 0o1000;
const IPC_NOWAIT: CInt = 0o4000;
const SETVAL: CInt = 16;
const GETVAL: CInt = 12;
const GETPID: CInt = 11;
const GETNCNT: CInt = 14;
const GETZCNT: CInt = 15;
const GETALL: CInt = 13;
const SETALL: CInt = 17;

unsafe fn sem_mkdirs() {
    ipc_mkdirs();
    mkdirat(AT_FDCWD, IPC_SEM.as_ptr() as *const CChar, 0o700);
}

unsafe fn sem_meta_path(id: i32, out: &mut [u8; 96]) -> usize {
    ipc_name(b"/run/husk/ipc/sem/", id as i64, b".m\0", out);
    out.iter().position(|c| *c == 0).unwrap_or(0)
}

unsafe fn sem_val_path(id: i32, out: &mut [u8; 96]) -> usize {
    ipc_name(b"/run/husk/ipc/sem/", id as i64, b"\0", out);
    out.iter().position(|c| *c == 0).unwrap_or(0)
}

/// key nsems mode rmid
unsafe fn sem_read_meta(id: i32, out: &mut [i64; 4]) -> bool {
    let mut p = [0u8; 96];
    let n = sem_meta_path(id, &mut p);
    let mut buf = [0u8; 128];
    let r = ipc_read_file(&p[..n], &mut buf);
    if r <= 0 {
        return false;
    }
    ipc_parse_ints(&buf[..r as usize], out) == 4
}

unsafe fn sem_write_meta(id: i32, m: &[i64; 4]) -> CInt {
    let mut p = [0u8; 96];
    let n = sem_meta_path(id, &mut p);
    let mut b = [0u8; 96];
    let mut at = 0usize;
    for v in m.iter() {
        ipc_put_dec(&mut b, &mut at, *v);
        b[at] = b' ';
        at += 1;
    }
    b[at] = b'\n';
    at += 1;
    ipc_write_file(&p[..n], &b[..at])
}

unsafe fn sem_get(id: i32, idx: usize) -> i32 {
    let mut p = [0u8; 96];
    let n = sem_val_path(id, &mut p);
    let fd = openat(AT_FDCWD, p.as_ptr() as *const CChar, 0o2, 0);
    if fd < 0 {
        return -1;
    }
    let mut v = 0i32;
    let r = syscall4(SYS_PREAD, fd as usize, &mut v as *mut i32 as usize, 4, idx * 4);
    rc::close(fd);
    if r != 4 {
        return -1;
    }
    v
}

unsafe fn sem_set(id: i32, idx: usize, v: i32) -> bool {
    let mut p = [0u8; 96];
    let n = sem_val_path(id, &mut p);
    let fd = openat(AT_FDCWD, p.as_ptr() as *const CChar, 0o2, 0);
    if fd < 0 {
        return false;
    }
    let r = syscall4(SYS_PWRITE, fd as usize, &v as *const i32 as usize, 4, idx * 4);
    rc::close(fd);
    r == 4
}

/// Locks the value file for a whole semop, so "check every op, then apply every
/// op" is atomic against other processes and other threads.
unsafe fn sem_lock(id: i32) -> CInt {
    let mut p = [0u8; 96];
    let n = sem_val_path(id, &mut p);
    let fd = openat(AT_FDCWD, p.as_ptr() as *const CChar, 0o2, 0);
    if fd < 0 {
        return -1;
    }
    let r = syscall4(SYS_FLOCK_NR, fd as usize, 2, 0, 0);
    if r < 0 {
        rc::close(fd);
        return ret_or_errno(r);
    }
    fd
}

unsafe fn sem_unlock(fd: CInt) {
    syscall4(SYS_FLOCK_NR, fd as usize, 8, 0, 0);
    rc::close(fd);
}

#[no_mangle]
pub unsafe extern "C" fn semget(key: i32, nsems: CInt, semflg: CInt) -> CInt {
    let entry_errno = *__errno_location();
    sem_mkdirs();
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = semget_locked(key, nsems, semflg);
    ipc_unlockf(lf);
    if r >= 0 {
        *__errno_location() = entry_errno;
    }
    r
}

unsafe fn semget_locked(key: i32, nsems: CInt, semflg: CInt) -> CInt {
    let create = semflg & IPC_CREAT != 0;
    let excl = semflg & IPC_EXCL != 0;
    let private = key == 0;
    if !private {
        let id = keys_find(key);
        if id >= 0 {
            let mut m = [0i64; 4];
            if !sem_read_meta(id, &mut m) {
                keys_remove_id(id);
            } else {
                if create && excl {
                    return set_errno(17);
                }
                if nsems != 0 && nsems as i64 > m[1] {
                    return set_errno(22);
                }
                return id;
            }
        }
        if !create {
            return set_errno(2);
        }
    }
    if nsems <= 0 || nsems as usize > SEM_MAX {
        return set_errno(22);
    }
    let id = ipc_nextid();
    let mut p = [0u8; 96];
    let n = sem_val_path(id, &mut p);
    let fd = openat(AT_FDCWD, p.as_ptr() as *const CChar, 0o2 | 0o100, 0o600);
    if fd < 0 {
        return -1;
    }
    let zeros = [0i32; SEM_MAX];
    syscall4(SYS_PWRITE, fd as usize, zeros.as_ptr() as usize, nsems as usize * 4, 0);
    let _ = syscall4(46, fd as usize, nsems as usize * 4, 0, 0);
    rc::close(fd);
    let m = [key as i64, nsems as i64, (semflg as u32 & 0o777) as i64, 0];
    sem_write_meta(id, &m);
    if !private {
        keys_add(key, id);
    }
    id
}

/// struct sembuf { unsigned short sem_num; short sem_op; short sem_flg; } — 6
/// bytes, packed, and that is what crosses this ABI.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SembufRaw {
    num: u16,
    op: i16,
    flg: i16,
}

#[no_mangle]
pub unsafe extern "C" fn semop(semid: CInt, sops: usize, nsops: usize) -> CInt {
    semop_impl(semid, sops, nsops, None)
}

#[no_mangle]
pub unsafe extern "C" fn semtimedop(
    semid: CInt,
    sops: usize,
    nsops: usize,
    timeout: usize,
) -> CInt {
    semop_impl(semid, sops, nsops, Some(timeout))
}

unsafe fn semop_impl(semid: CInt, sops: usize, nsops: usize, timeout: Option<usize>) -> CInt {
    if sops == 0 || nsops == 0 || nsops > SEM_MAX {
        return set_errno(22);
    }
    let mut m = [0i64; 4];
    if !sem_read_meta(semid, &mut m) || m[3] != 0 {
        return set_errno(22);
    }
    let nsems = m[1] as usize;
    let ops = core::slice::from_raw_parts(sops as *const SembufRaw, nsops);
    for o in ops.iter() {
        if o.num as usize >= nsems {
            return set_errno(22);
        }
    }
    // SEM_UNDO is not implemented; say so rather than silently changing the
    // meaning of a program's semaphores.
    for o in ops.iter() {
        if o.flg & SEM_UNDO as i16 != 0 {
            return set_errno(95); // EOPNOTSUPP
        }
    }
    let nowait = ops.iter().any(|o| o.flg & IPC_NOWAIT as i16 != 0);
    let mut deadline = [0i64; 2];
    let timed = if let Some(t) = timeout {
        if t != 0 {
            let ts = core::slice::from_raw_parts(t as *const i64, 2);
            deadline = [ts[0], ts[1]];
        }
        t != 0
    } else {
        false
    };
    let mut base = [0i64; 2];
    if timed {
        syscall4(113, 0, base.as_mut_ptr() as usize, 0, 0); // CLOCK_REALTIME
    }
    loop {
        let fd = sem_lock(semid);
        if fd < 0 {
            return -1;
        }
        // Read the whole set once.
        let mut vals = [0i32; SEM_MAX];
        let mut ok = true;
        for i in 0..nsems {
            let mut v = 0i32;
            let r = syscall4(
                SYS_PREAD,
                fd as usize,
                &mut v as *mut i32 as usize,
                4,
                i * 4,
            );
            if r != 4 {
                ok = false;
                break;
            }
            vals[i] = v;
        }
        if !ok {
            sem_unlock(fd);
            return set_errno(22);
        }
        let mut ready = true;
        for o in ops.iter() {
            let v = vals[o.num as usize];
            if o.op < 0 && v < -(o.op as i32) {
                ready = false;
                break;
            }
            if o.op == 0 && v != 0 {
                ready = false;
                break;
            }
        }
        if ready {
            for o in ops.iter() {
                vals[o.num as usize] += o.op as i32;
            }
            let mut wrote = true;
            for i in 0..nsems {
                let r = syscall4(
                    SYS_PWRITE,
                    fd as usize,
                    &vals[i] as *const i32 as usize,
                    4,
                    i * 4,
                );
                if r != 4 {
                    wrote = false;
                }
            }
            sem_unlock(fd);
            return if wrote { 0 } else { set_errno(5) };
        }
        sem_unlock(fd);
        if nowait {
            return set_errno(11); // EAGAIN
        }
        if timed {
            let mut now = [0i64; 2];
            syscall4(113, 0, now.as_mut_ptr() as usize, 0, 0);
            let elapsed = (now[0] - base[0]) * 1_000_000_000 + (now[1] - base[1]);
            let limit = deadline[0] * 1_000_000_000 + deadline[1];
            if elapsed >= limit {
                return set_errno(110); // ETIMEDOUT
            }
        }
        let ts = [0i64, 2_000_000];
        syscall4(115, ts.as_ptr() as usize, 0, 0, 0);
    }
}

#[no_mangle]
pub unsafe extern "C" fn semctl(semid: CInt, semnum: CInt, cmd: CInt, arg: usize) -> CInt {
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = semctl_locked(semid, semnum, cmd, arg);
    ipc_unlockf(lf);
    r
}

unsafe fn semctl_locked(semid: CInt, semnum: CInt, cmd: CInt, arg: usize) -> CInt {
    let mut m = [0i64; 4];
    if !sem_read_meta(semid, &mut m) {
        return set_errno(22);
    }
    let nsems = m[1] as usize;
    match cmd {
        IPC_RMID => {
            m[3] = 1;
            sem_write_meta(semid, &m);
            keys_remove_id(semid);
            let mut p = [0u8; 96];
            let n = sem_val_path(semid, &mut p);
            unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
            let n = sem_meta_path(semid, &mut p);
            unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
            0
        }
        GETVAL => {
            if semnum < 0 || semnum as usize >= nsems {
                return set_errno(22);
            }
            sem_get(semid, semnum as usize)
        }
        SETVAL => {
            if semnum < 0 || semnum as usize >= nsems {
                return set_errno(22);
            }
            if sem_set(semid, semnum as usize, arg as i32) {
                0
            } else {
                set_errno(22)
            }
        }
        GETPID | GETNCNT | GETZCNT => 0,
        GETALL => {
            if arg == 0 {
                return set_errno(14);
            }
            let out = core::slice::from_raw_parts_mut(arg as *mut u16, nsems);
            for i in 0..nsems {
                let v = sem_get(semid, i);
                if v < 0 {
                    return set_errno(22);
                }
                out[i] = v as u16;
            }
            0
        }
        SETALL => {
            if arg == 0 {
                return set_errno(14);
            }
            let inp = core::slice::from_raw_parts(arg as *const u16, nsems);
            for i in 0..nsems {
                sem_set(semid, i, inp[i] as i32);
            }
            0
        }
        IPC_STAT => {
            // glibc 2.39 arm64 struct semid_ds, verified field by field on the
            // device: ipc_perm 0..48, sem_otime 48, sem_ctime 56, sem_nsems 64.
            // Only those 72 bytes are touched; whatever reserved tail the
            // caller's struct has is left alone (writing past what we know
            // corrupts callers that allocate the smaller layout).
            if arg == 0 {
                return set_errno(14);
            }
            let b = arg as *mut u8;
            core::ptr::write_bytes(b, 0, 72);
            core::ptr::write_unaligned(b as *mut i32, m[0] as i32);
            core::ptr::write_unaligned((b.add(20)) as *mut u32, m[2] as u32);
            core::ptr::write_unaligned((b.add(64)) as *mut u64, nsems as u64);
            0
        }
        IPC_SET => 0,
        _ => set_errno(22),
    }
}

// -------------------------------------------------- SysV IPC: messages ---
//
//   /run/husk/ipc/msg/<id>/          one queue, a directory
//   /run/husk/ipc/msg/<id>/<mtype>_<nsec>_<pid>
// The file name starts with the message type zero-padded to 20 digits, so a
// lexicographic scan of the directory is exactly the kernel's ordering: by
// type, then FIFO.

const IPC_MSG: &[u8] = b"/run/husk/ipc/msg\0";
const MSG_NOERROR: CInt = 0o10000;
const MSG_EXCEPT: CInt = 0o20000;

unsafe fn msg_mkdirs() {
    ipc_mkdirs();
    mkdirat(AT_FDCWD, IPC_MSG.as_ptr() as *const CChar, 0o700);
}

unsafe fn msg_meta_path(id: i32, out: &mut [u8; 96]) -> usize {
    ipc_name(b"/run/husk/ipc/msg/", id as i64, b".m\0", out);
    out.iter().position(|c| *c == 0).unwrap_or(0)
}

unsafe fn msg_dir_path(id: i32, out: &mut [u8; 96]) -> usize {
    ipc_name(b"/run/husk/ipc/msg/", id as i64, b"\0", out);
    out.iter().position(|c| *c == 0).unwrap_or(0)
}

/// key mode qbytes rmid lspid lrpid stime rtime ctime
unsafe fn msg_read_meta(id: i32, out: &mut [i64; 9]) -> bool {
    let mut p = [0u8; 96];
    let n = msg_meta_path(id, &mut p);
    let mut buf = [0u8; 160];
    let r = ipc_read_file(&p[..n], &mut buf);
    if r <= 0 {
        return false;
    }
    ipc_parse_ints(&buf[..r as usize], out) == 9
}

unsafe fn msg_write_meta(id: i32, m: &[i64; 9]) -> CInt {
    let mut p = [0u8; 96];
    let n = msg_meta_path(id, &mut p);
    let mut b = [0u8; 200];
    let mut at = 0usize;
    for v in m.iter() {
        ipc_put_dec(&mut b, &mut at, *v);
        b[at] = b' ';
        at += 1;
    }
    b[at] = b'\n';
    at += 1;
    ipc_write_file(&p[..n], &b[..at])
}

#[no_mangle]
pub unsafe extern "C" fn msgget(key: i32, msgflg: CInt) -> CInt {
    let entry_errno = *__errno_location();
    msg_mkdirs();
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = msgget_locked(key, msgflg);
    ipc_unlockf(lf);
    if r >= 0 {
        *__errno_location() = entry_errno;
    }
    r
}

unsafe fn msgget_locked(key: i32, msgflg: CInt) -> CInt {
    let create = msgflg & IPC_CREAT != 0;
    let excl = msgflg & IPC_EXCL != 0;
    let private = key == 0;
    if !private {
        let id = keys_find(key);
        if id >= 0 {
            let mut m = [0i64; 9];
            if !msg_read_meta(id, &mut m) {
                keys_remove_id(id);
            } else {
                if create && excl {
                    return set_errno(17);
                }
                return id;
            }
        }
        if !create {
            return set_errno(2);
        }
    }
    let id = ipc_nextid();
    let mut p = [0u8; 96];
    let n = msg_dir_path(id, &mut p);
    mkdirat(AT_FDCWD, p.as_ptr() as *const CChar, 0o700);
    let m = [
        key as i64,
        (msgflg as u32 & 0o777) as i64,
        16384, // qbytes
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    msg_write_meta(id, &m);
    if !private {
        keys_add(key, id);
    }
    id
}

/// Picks the message the kernel would pick, and returns its name.
unsafe fn msg_pick(
    id: i32,
    msgtyp: i64,
    except: bool,
    out: &mut [u8; 64],
    out_len: &mut usize,
) -> bool {
    let mut p = [0u8; 96];
    let n = msg_dir_path(id, &mut p);
    let dfd = openat(AT_FDCWD, p.as_ptr() as *const CChar, O_DIRECTORY, 0);
    if dfd < 0 {
        return false;
    }
    // Read one directory listing, then rewind for the next call.
    let mut buf = [0u8; 4096];
    let mut best_type = u64::MAX;
    let mut best_name = [0u8; 64];
    let mut best_len = 0usize;
    let mut any = false;
    let use_lowest = msgtyp < 0;
    let limit_type = if msgtyp < 0 { (-msgtyp) as u64 } else { msgtyp as u64 };
    loop {
        let r = syscall4(61, dfd as usize, buf.as_mut_ptr() as usize, buf.len(), 0) as isize;
        if r <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < r as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &buf[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if nm != b"." && nm != b".." && nm.len() > 20 {
                let mut t = 0u64;
                let mut ok = true;
                for k in 0..20 {
                    if !nm[k].is_ascii_digit() {
                        ok = false;
                        break;
                    }
                    t = t * 10 + (nm[k] - b'0') as u64;
                }
                if ok {
                    let take = if msgtyp == 0 {
                        true
                    } else if msgtyp > 0 {
                        if except { t != limit_type } else { t == limit_type }
                    } else {
                        t <= limit_type
                    };
                    if take {
                        if use_lowest {
                            if t < best_type || !any {
                                best_type = t;
                                best_name[..len].copy_from_slice(nm);
                                best_len = len;
                                any = true;
                            }
                        } else if !any {
                            // First in directory order is already the oldest of
                            // its type, and types come in name order.
                            best_type = t;
                            best_name[..len].copy_from_slice(nm);
                            best_len = len;
                            any = true;
                        } else if msgtyp == 0 && t < best_type {
                            best_type = t;
                            best_name[..len].copy_from_slice(nm);
                            best_len = len;
                        }
                    }
                }
            }
            off += reclen;
        }
    }
    rc::close(dfd);
    if any {
        out[..best_len].copy_from_slice(&best_name[..best_len]);
        *out_len = best_len;
    }
    any
}

#[no_mangle]
pub unsafe extern "C" fn msgsnd(msqid: CInt, msgp: usize, msgsz: usize, msgflg: CInt) -> CInt {
    msgsnd_impl(msqid, msgp, msgsz, msgflg)
}

unsafe fn msgsnd_impl(msqid: CInt, msgp: usize, msgsz: usize, _msgflg: CInt) -> CInt {
    let mut m = [0i64; 9];
    if !msg_read_meta(msqid, &mut m) {
        return set_errno(22);
    }
    if msgp == 0 || msgsz > 8192 {
        return set_errno(22);
    }
    let mtype = core::ptr::read_unaligned(msgp as *const i64);
    if mtype < 1 {
        return set_errno(22);
    }
    let mut p = [0u8; 96];
    let n = msg_dir_path(msqid, &mut p);
    let dfd = openat(AT_FDCWD, p.as_ptr() as *const CChar, O_DIRECTORY, 0);
    if dfd < 0 {
        return -1;
    }
    let mut name = [0u8; 64];
    let mut at = 0usize;
    // 20 digits so the name sorts by type numerically
    let mut digits = [b'0'; 20];
    let mut t = mtype as u64;
    let mut k = 20;
    while k > 0 {
        k -= 1;
        digits[k] = b'0' + (t % 10) as u8;
        t /= 10;
    }
    name[..20].copy_from_slice(&digits);
    at = 20;
    name[at] = b'_';
    at += 1;
    let mut now = [0i64; 2];
    syscall4(113, 4 /* MONOTONIC */, now.as_mut_ptr() as usize, 0, 0);
    ipc_put_dec(&mut name, &mut at, now[0] * 1_000_000_000 + now[1]);
    name[at] = b'_';
    at += 1;
    ipc_put_dec(&mut name, &mut at, syscall4(172, 0, 0, 0, 0) as i64);
    let fd = syscall4(
        SYS_OPENAT,
        dfd as usize,
        name.as_ptr() as usize,
        0o1 | 0o100 | 0o200,
        0o600,
    ) as CInt;
    if fd < 0 {
        rc::close(dfd);
        return ret_or_errno(fd as Ssize);
    }
    let body = msgp + 8;
    let mut left = msgsz;
    let mut q = body;
    while left > 0 {
        let w = syscall4(SYS_WRITE, fd as usize, q, left, 0) as isize;
        if w <= 0 {
            syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
            rc::close(dfd);
            return set_errno(5);
        }
        left -= w as usize;
        q += w as usize;
    }
    syscall4(SYS_CLOSE, fd as usize, 0, 0, 0);
    rc::close(dfd);
    m[6] = m[6].max(0); // stime touched below by the caller-visible STAT
    let mut now2 = [0i64; 2];
    syscall4(113, 0, now2.as_mut_ptr() as usize, 0, 0);
    m[6] = now2[0];
    m[4] = syscall4(172, 0, 0, 0, 0) as i64;
    msg_write_meta(msqid, &m);
    0
}

#[no_mangle]
pub unsafe extern "C" fn msgrcv(
    msqid: CInt,
    msgp: usize,
    msgsz: usize,
    msgtyp: i64,
    msgflg: CInt,
) -> isize {
    let mut m = [0i64; 9];
    if !msg_read_meta(msqid, &mut m) {
        return set_errno(22) as isize;
    }
    if msgp == 0 {
        return set_errno(14) as isize;
    }
    let nowait = msgflg & IPC_NOWAIT != 0;
    let except = msgflg & MSG_EXCEPT != 0 && msgtyp > 0;
    loop {
        let mut name = [0u8; 64];
        let mut nl = 0usize;
        if msg_pick(msqid, msgtyp, except, &mut name, &mut nl) {
            let mut p = [0u8; 96];
            let n = msg_dir_path(msqid, &mut p);
            let fd = openat(AT_FDCWD, p.as_ptr() as *const CChar, O_DIRECTORY, 0);
            if fd < 0 {
                return -1;
            }
            let ffd = syscall4(SYS_OPENAT, fd as usize, name.as_ptr() as usize, 0, 0) as CInt;
            rc::close(fd);
            if ffd < 0 {
                continue;
            }
            // The type comes back from the name, the text from the file. The
            // real length is asked of the file, not of the read: a message
            // longer than the caller's buffer must fail with E2BIG *and stay
            // on the queue*, which is what the kernel does.
            let mut t = 0i64;
            for k in 0..20 {
                t = t * 10 + (name[k] - b'0') as i64;
            }
            let size = syscall4(SYS_LSEEK, ffd as usize, 0, 2, 0) as isize;
            if size > 0 && size as usize > msgsz && msgflg & MSG_NOERROR == 0 {
                syscall4(SYS_CLOSE, ffd as usize, 0, 0, 0);
                return set_errno(7) as isize; // E2BIG
            }
            let want = if size < 0 { 0 } else { (size as usize).min(msgsz) };
            let mut total = 0usize;
            syscall4(SYS_LSEEK, ffd as usize, 0, 0, 0);
            while total < want {
                let r = syscall4(SYS_READ, ffd as usize, msgp + 8 + total, want - total, 0) as isize;
                if r <= 0 {
                    break;
                }
                total += r as usize;
            }
            syscall4(SYS_CLOSE, ffd as usize, 0, 0, 0);
            let mut pp = [0u8; 96];
            let pn = msg_dir_path(msqid, &mut pp);
            let _ = pn;
            let dfd = openat(AT_FDCWD, pp.as_ptr() as *const CChar, O_DIRECTORY, 0);
            if dfd >= 0 {
                unlinkat(dfd, name.as_ptr() as *const CChar, 0);
                rc::close(dfd);
            }
            core::ptr::write_unaligned(msgp as *mut i64, t);
            let mut now2 = [0i64; 2];
            syscall4(113, 0, now2.as_mut_ptr() as usize, 0, 0);
            m[7] = now2[0];
            m[5] = syscall4(172, 0, 0, 0, 0) as i64;
            msg_write_meta(msqid, &m);
            return total as isize;
        }
        if nowait {
            return set_errno(42) as isize; // ENOMSG
        }
        let ts = [0i64, 2_000_000];
        syscall4(115, ts.as_ptr() as usize, 0, 0, 0);
    }
}

#[no_mangle]
pub unsafe extern "C" fn msgctl(msqid: CInt, cmd: CInt, buf: usize) -> CInt {
    let lf = ipc_lockf();
    if lf < 0 {
        return -1;
    }
    let r = msgctl_locked(msqid, cmd, buf);
    ipc_unlockf(lf);
    r
}

unsafe fn msgctl_locked(msqid: CInt, cmd: CInt, buf: usize) -> CInt {
    let mut m = [0i64; 9];
    if !msg_read_meta(msqid, &mut m) {
        return set_errno(22);
    }
    match cmd {
        IPC_RMID => {
            keys_remove_id(msqid);
            let mut p = [0u8; 96];
            let n = msg_dir_path(msqid, &mut p);
            let dfd = openat(AT_FDCWD, p.as_ptr() as *const CChar, O_DIRECTORY, 0);
            if dfd >= 0 {
                let mut b = [0u8; 1024];
                loop {
                    let r = syscall4(61, dfd as usize, b.as_mut_ptr() as usize, b.len(), 0) as isize;
                    if r <= 0 {
                        break;
                    }
                    let mut off = 0usize;
                    while off < r as usize {
                        let reclen = u16::from_le_bytes([b[off + 16], b[off + 17]]) as usize;
                        if reclen == 0 {
                            break;
                        }
                        let nm = &b[off + 19..off + reclen];
                        let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
                        let nm = &nm[..len];
                        if nm != b"." && nm != b".." {
                            unlinkat(dfd, nm.as_ptr() as *const CChar, 0);
                        }
                        off += reclen;
                    }
                }
                rc::close(dfd);
            }
            unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0x200);
            let n = msg_meta_path(msqid, &mut p);
            unlinkat(AT_FDCWD, p.as_ptr() as *const CChar, 0);
            0
        }
        IPC_STAT => {
            if buf == 0 {
                return set_errno(14);
            }
            let qnum = msg_count(msqid);
            let b = buf as *mut u8;
            core::ptr::write_bytes(b, 0, 120);
            core::ptr::write_unaligned(b as *mut i32, m[0] as i32);
            core::ptr::write_unaligned((b.add(20)) as *mut u32, m[1] as u32);
            core::ptr::write_unaligned((b.add(48)) as *mut i64, m[6]);
            core::ptr::write_unaligned((b.add(56)) as *mut i64, m[7]);
            core::ptr::write_unaligned((b.add(64)) as *mut i64, m[8]);
            core::ptr::write_unaligned((b.add(72)) as *mut u64, m[2] as u64);
            core::ptr::write_unaligned((b.add(80)) as *mut u64, qnum as u64);
            core::ptr::write_unaligned((b.add(88)) as *mut u64, m[2] as u64);
            core::ptr::write_unaligned((b.add(96)) as *mut i32, m[4] as i32);
            core::ptr::write_unaligned((b.add(100)) as *mut i32, m[5] as i32);
            0
        }
        IPC_SET => {
            if buf == 0 {
                return set_errno(14);
            }
            let b = buf as *const u8;
            m[1] = core::ptr::read_unaligned((b.add(20)) as *const u32) as i64;
            m[2] = core::ptr::read_unaligned((b.add(88)) as *const u64) as i64;
            msg_write_meta(msqid, &m)
        }
        _ => set_errno(22),
    }
}

unsafe fn msg_count(id: i32) -> usize {
    let mut p = [0u8; 96];
    let n = msg_dir_path(id, &mut p);
    let dfd = openat(AT_FDCWD, p.as_ptr() as *const CChar, O_DIRECTORY, 0);
    if dfd < 0 {
        return 0;
    }
    let mut b = [0u8; 2048];
    let mut count = 0usize;
    loop {
        let r = syscall4(61, dfd as usize, b.as_mut_ptr() as usize, b.len(), 0) as isize;
        if r <= 0 {
            break;
        }
        let mut off = 0usize;
        while off < r as usize {
            let reclen = u16::from_le_bytes([b[off + 16], b[off + 17]]) as usize;
            if reclen == 0 {
                break;
            }
            let nm = &b[off + 19..off + reclen];
            let len = nm.iter().position(|c| *c == 0).unwrap_or(nm.len());
            let nm = &nm[..len];
            if nm != b"." && nm != b".." {
                count += 1;
            }
            off += reclen;
        }
    }
    rc::close(dfd);
    count
}

// ------------------------------------------------- AF_NETLINK emulation ---
//
// `socket(AF_NETLINK, ...)` succeeds here, but `bind` and `sendmsg` on it come
// back EACCES: SELinux refuses netlink_route_socket to `untrusted_app`. That is
// a limit on the socket, not on the information — the interface list and the
// addresses come back from the SIOCGIF* ioctls, which is exactly why `ifconfig`
// already works while `ip addr` and `ip route` die on their first message.
//
// So the socket is replaced by a datagram socketpair: the caller keeps one end,
// we keep the other, and every request is answered by writing a complete
// netlink dump into it. recvmsg / poll / close stay native, so a client that
// polls or reads with a timeout sees ordinary socket behaviour.
//
// Answered: RTM_GETLINK, RTM_GETADDR, RTM_GETROUTE (NETLINK_ROUTE).
// NETLINK_SOCK_DIAG gets an empty dump, so `ss` prints a table instead of
// dying; connection enumeration needs /proc/net/*, which this domain cannot
// read (see docs/design/network.md).

const AF_NETLINK_DOM: CInt = 16;
const AF_UNIX_DOM: usize = 1;
const SOCK_DGRAM_KIND: usize = 2;
const SYS_SOCKET_NR: usize = 198;
const SYS_SOCKETPAIR_NR: usize = 199;
const SYS_SENDTO_NR: usize = 206;
const SYS_SENDMSG_NR: usize = 211;
const SYS_RECVMSG_NR: usize = 212;

const NL_ROUTE_PROTO: CInt = 0;
const NL_SOCK_DIAG_PROTO: CInt = 4;
const NL_MAX: usize = 16;

#[derive(Clone, Copy)]
struct NlSlot {
    client: CInt,
    peer: CInt,
    proto: CInt,
    seq: u32,
    ack: u8,
}

static mut NL_SLOTS: [NlSlot; NL_MAX] = [NlSlot {
    client: -1,
    peer: -1,
    proto: -1,
    seq: 0,
    ack: 0,
}; NL_MAX];
static NL_LOCKW: AtomicU8 = AtomicU8::new(0);
/// Number of live fake netlink sockets. Zero on every ordinary program, which
/// keeps `send`/`sendto`/`sendmsg` at a single relaxed load for the vast
/// majority of traffic.
static NL_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe fn nl_lock() {
    while NL_LOCKW
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

unsafe fn nl_unlock() {
    NL_LOCKW.store(0, Ordering::Release);
}

unsafe fn nl_slot(fd: CInt) -> Option<usize> {
    for i in 0..NL_MAX {
        if NL_SLOTS[i].client == fd && fd >= 0 {
            return Some(i);
        }
    }
    None
}

unsafe fn nl_is_fake(fd: CInt) -> bool {
    if NL_COUNT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    if nl_slot(fd).is_none() {
        return false;
    }
    // A slot outlives a close() that happened inside libc (that path does not
    // come through our close()), and the descriptor number is then free for any
    // later socket. Treat it as ours only while it still answers as one of our
    // AF_UNIX datagram ends; otherwise drop the stale slot. Without this, a
    // recycled descriptor gets netlink answers on a UDP socket and glibc's
    // getaddrinfo sees a source address of family AF_NETLINK.
    let mut addr = [0u8; 128];
    let mut alen: u32 = 128;
    let r = syscall4(
        204, // getsockname
        fd as usize,
        addr.as_mut_ptr() as usize,
        core::ptr::addr_of_mut!(alen) as usize,
        0,
    );
    if r < 0 {
        nl_forget(fd);
        return false;
    }
    let fam = core::ptr::read_unaligned(addr.as_ptr() as *const u16);
    if fam != AF_UNIX_DOM as u16 {
        nl_forget(fd);
        return false;
    }
    true
}

unsafe fn nl_install(client: CInt, peer: CInt, proto: CInt) -> bool {
    nl_lock();
    let mut ok = false;
    for i in 0..NL_MAX {
        if NL_SLOTS[i].client < 0 {
            NL_SLOTS[i] = NlSlot {
                client,
                peer,
                proto,
                seq: 0,
                ack: 0,
            };
            NL_COUNT.store(NL_COUNT.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
            ok = true;
            break;
        }
    }
    nl_unlock();
    ok
}

unsafe fn nl_forget(fd: CInt) {
    nl_lock();
    for i in 0..NL_MAX {
        if NL_SLOTS[i].client == fd {
            NL_SLOTS[i] = NlSlot {
                client: -1,
                peer: -1,
                proto: -1,
                seq: 0,
                ack: 0,
            };
            let c = NL_COUNT.load(Ordering::Relaxed);
            NL_COUNT.store(c.saturating_sub(1), Ordering::Relaxed);
            break;
        }
    }
    nl_unlock();
}

#[no_mangle]
pub unsafe extern "C" fn socket(domain: CInt, typ: CInt, proto: CInt) -> CInt {
    if domain == AF_NETLINK_DOM && env_lookup(b"NR_NL_OFF").is_none() {
        if verbose_on() {
            dbg(&[b"nl socket af_netlink proto="]);
            dbgn(&[], proto as i64);
        }
        let mut fds = [0 as CInt; 2];
        let kind = SOCK_DGRAM_KIND | (typ as usize & 0o2000000); // keep SOCK_CLOEXEC
        // socketpair(domain, type, protocol, sv): the array goes in x3.
        let r = syscall4(
            SYS_SOCKETPAIR_NR,
            AF_UNIX_DOM,
            kind,
            0,
            fds.as_mut_ptr() as usize,
        );
        if r < 0 {
            return ret_or_errno(r);
        }
        // The client end never blocks: once a dump has been consumed there is
        // simply nothing more, which netlink clients already handle as EAGAIN.
        syscall4(SYS_FCNTL, fds[0] as usize, 4 /* F_SETFL */, 0o4000, 0);
        if !nl_install(fds[0], fds[1], proto) {
            syscall4(SYS_CLOSE, fds[0] as usize, 0, 0, 0);
            syscall4(SYS_CLOSE, fds[1] as usize, 0, 0, 0);
            return set_errno(23); // ENFILE
        }
        return fds[0];
    }
    ret_or_errno(syscall4(
        SYS_SOCKET_NR,
        domain as usize,
        typ as usize,
        proto as usize,
        0,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn getsockname(fd: CInt, addr: *mut u8, len: *mut u32) -> CInt {
    if nl_is_fake(fd) {
        if addr.is_null() || len.is_null() {
            return set_errno(14);
        }
        *len = 12;
        core::ptr::write_bytes(addr, 0, 12);
        core::ptr::write_unaligned(addr as *mut u16, AF_NETLINK_DOM as u16);
        core::ptr::write_unaligned(addr.add(4) as *mut u32, syscall4(172, 0, 0, 0, 0) as u32);
        return 0;
    }
    ret_or_errno(syscall5(
        204, // getsockname
        fd as usize,
        addr as usize,
        len as usize,
        0,
        0,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn sendto(
    fd: CInt,
    buf: usize,
    len: usize,
    flags: CInt,
    addr: usize,
    addrlen: u32,
) -> Ssize {
    if nl_is_fake(fd) {
        return nl_handle(fd, buf as *const u8, len);
    }
    ret_or_errno(syscall6(
        SYS_SENDTO_NR,
        fd as usize,
        buf,
        len,
        flags as usize,
        addr,
        addrlen as usize,
    )) as Ssize
}

#[no_mangle]
pub unsafe extern "C" fn send(fd: CInt, buf: usize, len: usize, flags: CInt) -> Ssize {
    if nl_is_fake(fd) {
        return nl_handle(fd, buf as *const u8, len);
    }
    // sendto is a six-argument syscall: leaving x4/x5 unset hands the kernel a
    // garbage destination pointer, which comes back as EINVAL. That is what made
    // every curl "Send failure: Invalid argument" while a raw connect() and
    // python's send() worked.
    ret_or_errno(syscall6(
        SYS_SENDTO_NR,
        fd as usize,
        buf,
        len,
        flags as usize,
        0,
        0,
    )) as Ssize
}

#[no_mangle]
pub unsafe extern "C" fn sendmsg(fd: CInt, msg: *const u8, flags: CInt) -> Ssize {
    if nl_is_fake(fd) && !msg.is_null() {
        // struct msghdr (arm64): name 0, namelen 8, iov 16, iovlen 24.
        let iov = core::ptr::read_unaligned(msg.add(16) as *const usize);
        let iovlen = core::ptr::read_unaligned(msg.add(24) as *const usize);
        let mut tmp = [0u8; 1024];
        let mut at = 0usize;
        for i in 0..iovlen.min(8) {
            let base = core::ptr::read_unaligned((iov + i * 16) as *const usize);
            let len = core::ptr::read_unaligned((iov + i * 16 + 8) as *const usize);
            if base == 0 || len == 0 {
                continue;
            }
            let n = len.min(tmp.len() - at);
            core::ptr::copy_nonoverlapping(base as *const u8, tmp.as_mut_ptr().add(at), n);
            at += n;
            if at == tmp.len() {
                break;
            }
        }
        return nl_handle(fd, tmp.as_ptr(), at);
    }
    ret_or_errno(syscall4(SYS_SENDMSG_NR, fd as usize, msg as usize, flags as usize, 0)) as Ssize
}

/// iproute2 refuses a datagram whose sender address is missing ("Sender
/// address length == 0"), and a *connected* socketpair never reports one, so
/// the reply is dressed as if it had come from the kernel.
#[no_mangle]
/// iproute2 refuses a datagram whose sender address is missing ("Sender
/// address length == 0"), and a *connected* socketpair never reports one, so
/// the reply is dressed as if it had come from the kernel.
unsafe fn dress_name(msg: *mut u8) {
    if msg.is_null() {
        return;
    }
    let name = core::ptr::read_unaligned(msg as *const usize);
    let namelen = msg.add(8) as *mut u32;
    // A connected socketpair reports no sender address at all (namelen 0), and
    // a netlink client refuses such a datagram. Every client that talks to
    // netlink passes a sockaddr_nl here, so fill it unconditionally.
    if name != 0 {
        core::ptr::write_bytes(name as *mut u8, 0, 12);
        core::ptr::write_unaligned(name as *mut u16, AF_NETLINK_DOM as u16);
        *namelen = 12;
    }
}

/// A dump that already ran dry answers with NLMSG_DONE carrying the sequence
/// number of the request, so a client stuck in a read loop stops instead of
/// spinning on EAGAIN.
unsafe fn nl_empty_done(fd: CInt, msg: *mut u8) -> Ssize {
    if msg.is_null() {
        return -11;
    }
    let iov = core::ptr::read_unaligned(msg.add(16) as *const usize);
    if iov == 0 {
        return -11;
    }
    let base = core::ptr::read_unaligned(iov as *const usize) as *mut u8;
    let blen = core::ptr::read_unaligned((iov + 8) as *const usize);
    if base.is_null() {
        return -11;
    }
    if blen < 20 {
        // The peek pass (iov_len 0, MSG_PEEK|MSG_TRUNC) only wants the size.
        return 20;
    }
    let (seq, typ, flags) = match nl_slot(fd) {
        Some(i) => {
            let t = if NL_SLOTS[i].ack != 0 {
                NLMSG_ERROR
            } else {
                NLMSG_DONE
            };
            (NL_SLOTS[i].seq, t, if t == NLMSG_DONE { NLM_F_MULTI } else { 0 })
        }
        None => (0, NLMSG_DONE, NLM_F_MULTI),
    };
    dbg(&[b"nl read past the end of a dump, type="]);
    dbgn(&[], typ as i64);
    core::ptr::write_bytes(base, 0, 20);
    core::ptr::write_unaligned(base as *mut u32, 20u32);
    core::ptr::write_unaligned(base.add(4) as *mut u16, typ);
    core::ptr::write_unaligned(base.add(6) as *mut u16, flags);
    core::ptr::write_unaligned(base.add(8) as *mut u32, seq);
    core::ptr::write_unaligned(base.add(12) as *mut u32, nl_pid());
    core::ptr::write_unaligned(base.add(16) as *mut u32, 0u32); // error 0
    20
}

#[no_mangle]
pub unsafe extern "C" fn recvmsg(fd: CInt, msg: *mut u8, flags: CInt) -> Ssize {
    let fake = nl_is_fake(fd);
    let mut r = syscall4(SYS_RECVMSG_NR, fd as usize, msg as usize, flags as usize, 0);
    if fake {
        dress_name(msg);
        if r == -11 {
            // iproute2's __rtnl_recvmsg() loops on EAGAIN without a bound, so
            // an empty queue must look like "the dump is over", not like
            // "nothing yet".
            r = nl_empty_done(fd, msg);
        }
    }
    // glibc's wrappers hand callers -1 and errno, never a negative raw value.
    if r < 0 && r > -4096 {
        *__errno_location() = (-r) as CInt;
        return -1;
    }
    r
}

// --------------------------------------------------------------- if_*index ---
//
// glibc's own if_nametoindex(3) and if_indextoname(3) go through an internal
// netlink socket that this SELinux domain cannot bind, so they fail with EACCES
// even though the kernel answers SIOCGIFINDEX / SIOCGIFNAME for us. The two
// exported names are answered here instead, which covers every caller that
// links against them; a libc-internal caller (there is none in path we use)
// would need a different mechanism and is listed as a known gap.

const SIOCGIFINDEX_REQ: usize = 0x8933;
const SIOCGIFNAME_REQ: usize = 0x8910;

/// One AF_INET dgram socket for the ioctl, or -1.
unsafe fn ifprobe_sock() -> CInt {
    let fd = syscall4(SYS_SOCKET_NR, 2 /* AF_INET */, SOCK_DGRAM_KIND, 0, 0) as CInt;
    fd
}

#[no_mangle]
pub unsafe extern "C" fn if_nametoindex(name: *const u8) -> u32 {
    if name.is_null() {
        return set_errno(22) as u32;
    }
    let sock = ifprobe_sock();
    if sock < 0 {
        return 0;
    }
    let mut req = [0u8; 40];
    let mut i = 0usize;
    while i < 15 {
        let c = *name.add(i);
        if c == 0 {
            break;
        }
        req[i] = c;
        i += 1;
    }
    let r = syscall4(SYS_IOCTL_NR, sock as usize, SIOCGIFINDEX_REQ, req.as_mut_ptr() as usize, 0);
    syscall4(SYS_CLOSE, sock as usize, 0, 0, 0);
    if r < 0 {
        *__errno_location() = 19; // ENODEV
        return 0;
    }
    u32::from_ne_bytes([req[16], req[17], req[18], req[19]])
}

#[no_mangle]
pub unsafe extern "C" fn if_indextoname(index: u32, buf: *mut u8) -> *mut u8 {
    if buf.is_null() {
        return core::ptr::null_mut();
    }
    let sock = ifprobe_sock();
    if sock < 0 {
        return core::ptr::null_mut();
    }
    let mut req = [0u8; 40];
    req[16..20].copy_from_slice(&index.to_ne_bytes());
    let r = syscall4(SYS_IOCTL_NR, sock as usize, SIOCGIFNAME_REQ, req.as_mut_ptr() as usize, 0);
    syscall4(SYS_CLOSE, sock as usize, 0, 0, 0);
    if r < 0 {
        *__errno_location() = 19; // ENODEV
        return core::ptr::null_mut();
    }
    core::ptr::copy_nonoverlapping(req.as_ptr(), buf, 16);
    buf
}

// ------------------------------------------------------------- the request

const RTM_NEWLINK: u16 = 16;
const RTM_NEWADDR: u16 = 20;
const RTM_NEWROUTE: u16 = 24;
const RTM_GETLINK: u16 = 18;
const RTM_GETADDR: u16 = 22;
const RTM_GETROUTE: u16 = 26;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_MULTI: u16 = 2;

const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_TXQLEN: u16 = 13;
const IFLA_OPERSTATE: u16 = 16;

const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_LABEL: u16 = 3;
const IFA_BROADCAST: u16 = 4;

const RTA_DST: u16 = 1;
const RTA_OIF: u16 = 4;
const RTA_PREFSRC: u16 = 7;

const RT_TABLE_MAIN: u8 = 254;
const RTPROT_KERNEL: u8 = 2;
const RT_SCOPE_LINK: u8 = 253;
const RT_SCOPE_HOST: u8 = 254;
const RTN_UNICAST: u8 = 1;
const ARPHRD_ETHER: u16 = 1;
const ARPHRD_LOOPBACK: u16 = 772;

#[derive(Clone, Copy)]
struct IfEnt {
    name: [u8; 16],
    name_len: usize,
    index: i32,
    flags: u32,
    mtu: i32,
    hwtype: u16,
    hwaddr: [u8; 6],
    ip: u32,
    mask: u32,
    have_ip: bool,
}

impl IfEnt {
    const fn empty() -> Self {
        IfEnt {
            name: [0; 16],
            name_len: 0,
            index: 0,
            flags: 0,
            mtu: 1500,
            hwtype: ARPHRD_ETHER,
            hwaddr: [0; 6],
            ip: 0,
            mask: 0,
            have_ip: false,
        }
    }
}

const SYS_IOCTL_NR: usize = 29;
const SIOCGIFCONF: usize = 0x8912;
const SIOCGIFFLAGS: usize = 0x8913;
const SIOCGIFNETMASK: usize = 0x891b;
const SIOCGIFMTU: usize = 0x8921;
const SIOCGIFHWADDR: usize = 0x8927;
const SIOCGIFINDEX: usize = 0x8933;

/// One `ifreq` buffer, `name` at 0 and the union at 16.
unsafe fn if_ioctl(sock: CInt, name: &[u8], req: usize, out: &mut [u8; 40]) -> bool {
    core::ptr::write_bytes(out.as_mut_ptr(), 0, 40);
    let n = name.len().min(15);
    out[..n].copy_from_slice(&name[..n]);
    let r = syscall4(
        SYS_IOCTL_NR,
        sock as usize,
        req,
        out.as_mut_ptr() as usize,
        0,
    );
    r >= 0
}

/// Interface table from the kernel, through the ioctls this domain is allowed
/// to use. Returns how many entries were filled.
unsafe fn nl_interfaces(out: &mut [IfEnt; 8]) -> usize {
    let mut n = 0usize;
    let sock = syscall4(SYS_SOCKET_NR, 2 /* AF_INET */, SOCK_DGRAM_KIND, 0, 0) as CInt;
    if sock < 0 {
        return 0;
    }
    let mut list = [0u8; 2048];
    let mut ifc = [0u8; 16];
    core::ptr::write_unaligned(ifc.as_mut_ptr() as *mut i32, list.len() as i32);
    core::ptr::write_unaligned(ifc.as_mut_ptr().add(8) as *mut u64, list.as_ptr() as u64);
    let r = syscall4(
        SYS_IOCTL_NR,
        sock as usize,
        SIOCGIFCONF,
        ifc.as_mut_ptr() as usize,
        0,
    );
    if r < 0 {
        syscall4(SYS_CLOSE, sock as usize, 0, 0, 0);
        return 0;
    }
    let used = (core::ptr::read_unaligned(ifc.as_ptr() as *const i32) as usize).min(list.len());
    let mut off = 0usize;
    while off + 40 <= used && n < out.len() {
        let raw = &list[off..off + 40];
        let mut e = IfEnt::empty();
        let nm = &raw[..16];
        e.name_len = nm.iter().position(|c| *c == 0).unwrap_or(16);
        e.name = [0; 16];
        e.name[..e.name_len].copy_from_slice(&nm[..e.name_len]);
        let fam = u16::from_ne_bytes([raw[16], raw[17]]);
        if e.name_len > 0 {
            let name = &e.name[..e.name_len];
            let mut buf = [0u8; 40];
            if if_ioctl(sock, name, SIOCGIFFLAGS, &mut buf) {
                e.flags = u16::from_ne_bytes([buf[16], buf[17]]) as u32;
            }
            if if_ioctl(sock, name, SIOCGIFINDEX, &mut buf) {
                e.index = i32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]);
            }
            if if_ioctl(sock, name, SIOCGIFMTU, &mut buf) {
                e.mtu = i32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]);
            }
            if if_ioctl(sock, name, SIOCGIFHWADDR, &mut buf) {
                let ht = u16::from_ne_bytes([buf[16], buf[17]]);
                if ht != 0 {
                    e.hwtype = ht;
                    e.hwaddr.copy_from_slice(&buf[18..24]);
                }
            } else if name == b"lo" {
                // SIOCGIFHWADDR is refused in this domain; the type is still
                // known, and saying "ether" for the loopback is just wrong.
                e.hwtype = ARPHRD_LOOPBACK;
            }
            if if_ioctl(sock, name, SIOCGIFNETMASK, &mut buf) {
                // sin_addr is in network order; keep the host value so prefix
                // arithmetic (leading_zeros, &) means what it looks like.
                e.mask = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
            }
            if fam == 2 {
                e.ip = u32::from_be_bytes([raw[20], raw[21], raw[22], raw[23]]);
                e.have_ip = true;
            }
            out[n] = e;
            n += 1;
        }
        off += 40;
    }
    syscall4(SYS_CLOSE, sock as usize, 0, 0, 0);
    n
}

unsafe fn nl_room(buf: &[u8], at: usize, need: usize) -> bool {
    at + need <= buf.len()
}

unsafe fn nl_push(buf: &mut [u8], at: &mut usize, data: &[u8]) {
    if nl_room(buf, *at, data.len()) {
        buf[*at..*at + data.len()].copy_from_slice(data);
        *at += data.len();
    }
}

unsafe fn nl_push_u32(buf: &mut [u8], at: &mut usize, v: u32) {
    let b = v.to_ne_bytes();
    nl_push(buf, at, &b);
}

unsafe fn nl_push_u16(buf: &mut [u8], at: &mut usize, v: u16) {
    let b = v.to_ne_bytes();
    nl_push(buf, at, &b);
}

/// Opens a netlink message at `at`, returns the offset of its length field.
unsafe fn nl_begin(buf: &mut [u8], at: &mut usize, typ: u16, flags: u16, seq: u32) -> usize {
    let hdr = *at;
    nl_push_u32(buf, at, 0); // len, fixed up later
    nl_push_u16(buf, at, typ);
    nl_push_u16(buf, at, flags);
    nl_push_u32(buf, at, seq);
    // The port id in a reply is the requester's, and iproute2 drops every
    // message whose nlmsg_pid is not the value it got back from getsockname()
    // on its own socket. That is our pid, which is what getsockname() reports
    // for the socket this shim hands out.
    nl_push_u32(buf, at, nl_pid());
    hdr
}

/// Our own port id, as seen by getsockname(). Cached: one getpid per process.
static NL_PID: AtomicI32 = AtomicI32::new(0);

unsafe fn nl_pid() -> u32 {
    let mut p = NL_PID.load(Ordering::Relaxed);
    if p == 0 {
        p = syscall4(172 /* getpid */, 0, 0, 0, 0) as i32;
        NL_PID.store(p, Ordering::Relaxed);
    }
    p as u32
}

unsafe fn nl_attr(buf: &mut [u8], at: &mut usize, typ: u16, data: &[u8]) {
    let len = 4 + data.len();
    nl_push_u16(buf, at, len as u16);
    nl_push_u16(buf, at, typ);
    nl_push(buf, at, data);
    while *at % 4 != 0 {
        nl_push(buf, at, &[0u8]);
    }
}

unsafe fn nl_close_msg(buf: &mut [u8], hdr: usize, end: usize) {
    let len = (end - hdr) as u32;
    core::ptr::write_unaligned(buf.as_mut_ptr().add(hdr) as *mut u32, len);
}

unsafe fn nl_put_link(buf: &mut [u8], at: &mut usize, e: &IfEnt, seq: u32) {
    let hdr = nl_begin(buf, at, RTM_NEWLINK, NLM_F_MULTI, seq);
    nl_push(buf, at, &[0u8, 0]); // family AF_UNSPEC, pad
    nl_push_u16(buf, at, e.hwtype);
    nl_push_u32(buf, at, e.index as u32);
    nl_push_u32(buf, at, e.flags);
    nl_push_u32(buf, at, 0); // change
    nl_attr(buf, at, IFLA_ADDRESS, &e.hwaddr);
    nl_attr(buf, at, IFLA_IFNAME, &e.name[..e.name_len + 1]);
    let mtu = (e.mtu.max(0) as u32).to_ne_bytes();
    nl_attr(buf, at, IFLA_MTU, &mtu);
    nl_attr(buf, at, IFLA_TXQLEN, &1000u32.to_ne_bytes());
    nl_attr(buf, at, IFLA_OPERSTATE, &[if e.flags & 1 != 0 { 6 } else { 0 }]);
    nl_close_msg(buf, hdr, *at);
}

unsafe fn nl_put_addr(buf: &mut [u8], at: &mut usize, e: &IfEnt, seq: u32) {
    let hdr = nl_begin(buf, at, RTM_NEWADDR, NLM_F_MULTI, seq);
    let prefix = if e.mask == 0 { 0 } else { (!e.mask).leading_zeros() as u8 };
    let scope = if e.ip & 0xff == 127 { RT_SCOPE_HOST } else { RT_SCOPE_LINK };
    nl_push(buf, at, &[2u8, prefix, 0, scope]); // AF_INET
    nl_push_u32(buf, at, e.index as u32);
    nl_attr(buf, at, IFA_ADDRESS, &e.ip.to_be_bytes());
    nl_attr(buf, at, IFA_LOCAL, &e.ip.to_be_bytes());
    nl_attr(buf, at, IFA_LABEL, &e.name[..e.name_len + 1]);
    nl_close_msg(buf, hdr, *at);
}

unsafe fn nl_put_route(
    buf: &mut [u8],
    at: &mut usize,
    dst: u32,
    prefix: u8,
    scope: u8,
    ifindex: i32,
    src: u32,
    seq: u32,
) {
    let hdr = nl_begin(buf, at, RTM_NEWROUTE, NLM_F_MULTI, seq);
    nl_push(buf, at, &[2u8, prefix, 0, 0]); // family, dst_len, src_len, tos
    nl_push(buf, at, &[RT_TABLE_MAIN, RTPROT_KERNEL, scope, RTN_UNICAST]);
    nl_push_u32(buf, at, 0); // flags
    nl_attr(buf, at, RTA_DST, &dst.to_be_bytes());
    nl_attr(buf, at, RTA_OIF, &(ifindex as u32).to_ne_bytes());
    if src != 0 {
        nl_attr(buf, at, RTA_PREFSRC, &src.to_be_bytes());
    }
    nl_close_msg(buf, hdr, *at);
}

unsafe fn nl_done(buf: &mut [u8], at: &mut usize, seq: u32) {
    let hdr = nl_begin(buf, at, NLMSG_DONE, NLM_F_MULTI, seq);
    nl_push_u32(buf, at, 0); // error
    nl_close_msg(buf, hdr, *at);
}

/// NLMSG_ERROR carrying an errno (0 = the ACK every netlink talk ends with).
/// The payload is a full struct nlmsgerr: the errno plus the request header the
/// kernel echoes back. iproute2 rejects a shorter one as "ERROR truncated".
unsafe fn nl_ack(buf: &mut [u8], at: &mut usize, seq: u32, err: i32) {
    let hdr = nl_begin(buf, at, NLMSG_ERROR, 0, seq);
    nl_push_u32(buf, at, err as u32);
    for _ in 0..16 {
        nl_push(buf, at, &[0u8]);
    }
    nl_close_msg(buf, hdr, *at);
}

/// rtnetlink allocates four verbs per object class - NEW, DEL, GET, SET - so
/// anything from RTM_NEWLINK (16) upward whose low two bits are not `10` is a
/// mutation, not a query. Message numbers are per protocol, hence the caller
/// checks the slot's proto first.
fn nl_route_is_modify(t: u16) -> bool {
    t >= 16 && (t & 3) != 2
}

/// Turns one request into one complete reply datagram and hands it to the peer
/// end. Returns the request length, so the caller sees a normal short write.
unsafe fn nl_handle(fd: CInt, req: *const u8, len: usize) -> Ssize {
    let idx = match nl_slot(fd) {
        Some(i) => i,
        None => return set_errno(22) as Ssize,
    };
    let peer = NL_SLOTS[idx].peer;
    let mut out = [0u8; 8192];
    let mut at = 0usize;
    let mut off = 0usize;
    while off + 16 <= len {
        let mlen = core::ptr::read_unaligned(req.add(off) as *const u32) as usize;
        let mtype = core::ptr::read_unaligned(req.add(off + 4) as *const u16);
        let mflags = core::ptr::read_unaligned(req.add(off + 6) as *const u16);
        let seq = core::ptr::read_unaligned(req.add(off + 8) as *const u32);
        if mlen < 16 || off + mlen > len {
            break;
        }
        NL_SLOTS[idx].seq = seq;
        if verbose_on() {
            dbg(&[b"nl req type="]);
            dbgn(&[b" flags="], mtype as i64);
            dbgn(&[], mflags as i64);
        }
        let dump = mflags & 0x300 == 0x300; // NLM_F_ROOT | NLM_F_MATCH
        let ack = mflags & 0x4 != 0; // NLM_F_ACK
        NL_SLOTS[idx].ack = ack as u8;
        let mark = at;
        let mut handled = true;
        let mut ifs = [IfEnt::empty(); 8];
        let nif = nl_interfaces(&mut ifs);
        match mtype {
            RTM_GETLINK => {
                // ifi_index 0 means "not specified": `ip link show lo` sends a
                // zero index plus IFLA_IFNAME, and answering "no such device"
                // for index 0 is exactly what iproute2 reads as a missing link.
                let named = i32::from_ne_bytes([
                    req.add(off + 20).read(),
                    req.add(off + 21).read(),
                    req.add(off + 22).read(),
                    req.add(off + 23).read(),
                ]);
                let want = if dump || mlen < 24 || named <= 0 { -1 } else { named };
                // `ip link show lo` names the interface in IFLA_IFNAME instead
                // of using its index, so the name has to be read out as well.
                let mut want_name = [0u8; 16];
                let mut want_nlen = 0usize;
                if !dump {
                    let mut a = off + 32; // nlmsghdr + ifinfomsg
                    while a + 4 <= off + mlen {
                        let alen =
                            core::ptr::read_unaligned(req.add(a) as *const u16) as usize;
                        let atyp =
                            core::ptr::read_unaligned(req.add(a + 2) as *const u16);
                        if alen < 4 || a + alen > off + mlen {
                            break;
                        }
                        if atyp == IFLA_IFNAME {
                            let p = req.add(a + 4);
                            let mut k = 0usize;
                            while k < 16 && k + 4 <= alen {
                                let c = *p.add(k);
                                if c == 0 {
                                    break;
                                }
                                want_name[k] = c;
                                k += 1;
                            }
                            want_nlen = k;
                        }
                        a += (alen + 3) & !3;
                    }
                }
                for i in 0..nif {
                    if want >= 0 && ifs[i].index != want {
                        continue;
                    }
                    if want_nlen > 0 && &ifs[i].name[..want_nlen] != &want_name[..want_nlen] {
                        continue;
                    }
                    nl_put_link(&mut out, &mut at, &ifs[i], seq);
                }
            }
            RTM_GETADDR => {
                for i in 0..nif {
                    if ifs[i].have_ip {
                        nl_put_addr(&mut out, &mut at, &ifs[i], seq);
                    }
                }
            }
            RTM_GETROUTE => {
                // `ip route get <dst>` is a single-shot RTM_GETROUTE that expects
                // one best-match route. That is a FIB lookup we cannot perform,
                // and answering with the first table entry would be a confident
                // wrong answer, so say plainly that it is not supported.
                if !dump {
                    handled = false;
                    nl_ack(&mut out, &mut at, seq, -95 /* EOPNOTSUPP */);
                    off += (mlen + 3) & !3;
                    continue;
                }
                for i in 0..nif {
                    if !ifs[i].have_ip {
                        continue;
                    }
                    let prefix = if ifs[i].mask == 0 {
                        0
                    } else {
                        (!ifs[i].mask).leading_zeros() as u8
                    };
                    let net = ifs[i].ip & ifs[i].mask;
                    let scope = if ifs[i].ip & 0xff == 127 { RT_SCOPE_HOST } else { RT_SCOPE_LINK };
                    nl_put_route(
                        &mut out,
                        &mut at,
                        net,
                        prefix,
                        scope,
                        ifs[i].index,
                        ifs[i].ip,
                        seq,
                    );
                }
            }
            _ => {
                // Everything else is either a verb that would change the host's
                // networking (impossible from untrusted_app) or a read we do
                // not synthesise. Neither may be answered with a bare "ok":
                // silent success is how `ip addr add` looked like it worked
                // while nothing changed.
                let err = if NL_SLOTS[idx].proto == NL_ROUTE_PROTO
                    && nl_route_is_modify(mtype)
                {
                    -1 // EPERM - we cannot touch the host's networking
                } else {
                    -95 // EOPNOTSUPP - a read surface we do not provide
                };
                handled = false;
                nl_ack(&mut out, &mut at, seq, err);
            }
        }
        if !handled {
            off += (mlen + 3) & !3;
            continue;
        }
        if dump {
            nl_done(&mut out, &mut at, seq);
        } else if ack {
            // Without the dump flag the caller sent rtnl_talk(.., NULL): it does
            // not read data messages at all, it waits for NLMSG_ERROR with
            // error 0 and prints "Unexpected reply!!!" at anything else.
            nl_ack(&mut out, &mut at, seq, 0);
        } else if at == mark {
            // Nothing to say: no such interface.
            nl_ack(&mut out, &mut at, seq, -19 /* ENODEV */);
        }
        off += (mlen + 3) & !3;
    }
    if at == 0 {
        nl_done(&mut out, &mut at, 0);
    }
    // Both ends of a socketpair are connected, so a plain write is the whole
    // story; sendto() without a destination is EDESTADDRREQ on some paths.
    let w = syscall4(SYS_WRITE, peer as usize, out.as_ptr() as usize, at, 0);
    if w < 0 {
        return ret_or_errno(w) as Ssize;
    }
    len as Ssize
}
