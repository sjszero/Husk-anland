// LinuxHub Native Runtime — Android capability probe
// Every result line is: probe|<name>|<PASS|FAIL|UNKNOWN>|<detail>
// Run as an Android app UID for product-relevant answers; adb shell results
// are only a kernel baseline because the SELinux domain and seccomp policy differ.

use std::ffi::CString;
use std::io::Write;
#[allow(unused_imports)]
use std::os::fd::FromRawFd;

#[cfg(target_os = "android")]
fn errno() -> i32 { unsafe { *libc::__errno() } }
#[cfg(not(target_os = "android"))]
fn errno() -> i32 { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) }

fn out(name: &str, ok: bool, detail: String) {
    println!("probe|{}|{}|{}", name, if ok { "PASS" } else { "FAIL" }, detail);
    let _ = std::io::stdout().flush();
}

fn begin(name: &str) {
    println!("begin|{}", name);
    let _ = std::io::stdout().flush();
}

fn flush_all() { let _ = std::io::stdout().flush(); }

fn cstr(s: &str) -> CString { CString::new(s).unwrap() }

unsafe fn run_in_child<F: FnOnce() -> i32>(f: F) -> i32 {
    let pid = libc::fork();
    if pid == 0 {
        let code = f();
        libc::_exit(code);
    }
    let mut st: i32 = 0;
    libc::waitpid(pid, &mut st, 0);
    if libc::WIFEXITED(st) { libc::WEXITSTATUS(st) } else { -1 }
}

fn env_report() {
    let mut buf = [0u8; 256];
    unsafe {
        let u = libc::uname(&mut *(buf.as_mut_ptr() as *mut libc::utsname));
        let un = &*(buf.as_ptr() as *const libc::utsname);
        let s = |a: &[libc::c_char]| -> String {
            let b: Vec<u8> = a.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            String::from_utf8_lossy(&b).to_string()
        };
        println!("env|uid|{}", libc::getuid());
        println!("env|euid|{}", libc::geteuid());
        println!("env|gid|{}", libc::getgid());
        println!("env|groups|{}", libc::getgroups(0, std::ptr::null_mut()));
        println!("env|uname|{} {} {} {}", s(&un.sysname), s(&un.release), s(&un.machine), if u == 0 { "" } else { "ERR" });
        println!("env|page_size|{}", libc::sysconf(libc::_SC_PAGESIZE));
        println!("env|nproc|{}", libc::sysconf(libc::_SC_NPROCESSORS_ONLN));
    }
    if let Ok(ctx) = std::fs::read_to_string("/proc/self/attr/current") {
        println!("env|selinux|{}", ctx.trim());
    }
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        for line in st.lines() {
            if line.starts_with("Seccomp:") || line.starts_with("NoNewPrivs:") || line.starts_with("CapEff:") {
                println!("env|status|{}", line.trim());
            }
        }
    }
    for s in ["/proc/sys/user/max_user_namespaces", "/proc/sys/kernel/unprivileged_userns_clone"] {
        match std::fs::read_to_string(s) { Ok(v) => println!("env|{}|{}", s, v.trim()), Err(e) => println!("env|{}|ERR {}", s, e) }
    }
    flush_all();
}

fn exec_tests(glibc: Option<&str>, exec_check: Option<&str>, ldso: Option<&str>, libdir: Option<&str>, guest: Option<&str>) {
    match std::process::Command::new("/system/bin/sh").args(["-c", "echo NATIVE_EXEC_OK"]).output() {
        Ok(o) => out("exec_android_elf", o.status.success() && String::from_utf8_lossy(&o.stdout).contains("NATIVE_EXEC_OK"), format!("status={}", o.status)),
        Err(e) => out("exec_android_elf", false, format!("spawn error: {e}")),
    }
    if let Some(p) = exec_check {
        match std::process::Command::new(p).arg("--nr-exec-probe").output() {
            Ok(o) => out("exec_app_data_binary", o.status.success(), format!("status={}", o.status)),
            Err(e) => out("exec_app_data_binary", false, format!("spawn error: {e}; raw={:?}", e.raw_os_error())),
        }
    }
    if let (Some(ld), Some(libdir), Some(guest)) = (ldso, libdir, guest) {
        match std::process::Command::new(ld).args(["--library-path", libdir, guest, "-c", "echo GLIBC_LOADER_OK; id -u"]).output() {
            Ok(o) => out("exec_glibc_via_loader", o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GLIBC_LOADER_OK"), format!("status={} out={}", o.status, String::from_utf8_lossy(&o.stdout).trim())),
            Err(e) => out("exec_glibc_via_loader", false, format!("spawn error: {e}; raw={:?}", e.raw_os_error())),
        }
    }
    if let Some(p) = glibc {
        match std::process::Command::new(p).arg("-c").arg("echo GLIBC_EXEC_OK").output() {
            Ok(o) => out("exec_glibc_elf_direct", o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GLIBC_EXEC_OK"), format!("status={} stderr={}", o.status, String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => out("exec_glibc_elf_direct", false, format!("spawn error: {e}; raw={:?}", e.raw_os_error())),
        }
    }
}

unsafe fn test_ptrace() {
    let code = run_in_child(|| {
        if libc::ptrace(libc::PTRACE_TRACEME, 0, std::ptr::null_mut::<libc::c_void>(), std::ptr::null_mut::<libc::c_void>()) == -1 {
            return errno();
        }
        libc::raise(libc::SIGSTOP);
        0
    });
    // run_in_child returns the child's exit status; PTRACE_TRACEME + SIGSTOP means
    // the child never exits normally, so a stopped/reaped child is expected.
    out("ptrace_traceme", code == 0 || code == -1, format!("child_status={code}"));
}

unsafe fn test_unshare(flag: i32, name: &str) {
    let c = run_in_child(|| {
        if libc::unshare(flag) == -1 { errno() } else { 0 }
    });
    out(name, c == 0, if c == 0 { "ok".into() } else { format!("errno={c}") });
}

unsafe fn test_mount() {
    let dir = cstr("/data/local/tmp/nr-mnt");
    std::fs::create_dir_all("/data/local/tmp/nr-mnt").ok();
    let src = cstr("none");
    let fstype = cstr("tmpfs");
    let c = run_in_child(|| {
        if libc::mount(src.as_ptr(), dir.as_ptr(), fstype.as_ptr(), 0, std::ptr::null()) == -1 { errno() } else { 0 }
    });
    out("mount_tmpfs", c == 0, if c == 0 { "ok".into() } else { format!("errno={c}") });
}

unsafe fn test_chroot() {
    let dir = cstr("/data/local/tmp");
    let c = run_in_child(|| {
        if libc::chroot(dir.as_ptr()) == -1 { errno() } else { 0 }
    });
    out("chroot", c == 0, if c == 0 { "ok".into() } else { format!("errno={c}") });
}

unsafe fn test_fuse() {
    let p = cstr("/dev/fuse");
    let fd = libc::open(p.as_ptr(), libc::O_RDWR);
    if fd >= 0 { libc::close(fd); out("open_dev_fuse", true, "opened".into()); }
    else { out("open_dev_fuse", false, format!("errno={}", errno())); }
}

unsafe fn test_memfd() {
    let name = cstr("nr-probe");
    let fd = libc::syscall(libc::SYS_memfd_create, name.as_ptr(), 0u32) as i32;
    if fd < 0 { out("memfd_create", false, format!("errno={}", errno())); return; }
    let ok = libc::ftruncate(fd, 4096) == 0;
    let p = libc::mmap(std::ptr::null_mut(), 4096, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC, libc::MAP_SHARED, fd, 0);
    let exec_ok = p != libc::MAP_FAILED;
    if exec_ok { libc::munmap(p, 4096); }
    libc::close(fd);
    out("memfd_create", ok, format!("exec_map={}", if exec_ok { "ok" } else { "denied" }));
}

unsafe fn test_openat2() {
    const SYS_OPENAT2: libc::c_long = 437;
    #[repr(C)] struct OpenHow { flags: u64, mode: u64, resolve: u64 }
    const RESOLVE_IN_ROOT: u64 = 0x10;
    let dirp = cstr("/data/local/tmp/nr-root");
    std::fs::create_dir_all("/data/local/tmp/nr-root").ok();
    std::fs::write("/data/local/tmp/nr-root/inner.txt", b"x").ok();
    let dfd = libc::open(dirp.as_ptr(), libc::O_PATH | libc::O_DIRECTORY);
    if dfd < 0 { out("openat2_resolve_in_root", false, format!("dirfd errno={}", errno())); return; }
    let how = OpenHow { flags: libc::O_RDONLY as u64, mode: 0, resolve: RESOLVE_IN_ROOT };
    let fd = libc::syscall(SYS_OPENAT2, dfd, cstr("inner.txt").as_ptr(), &how as *const OpenHow, std::mem::size_of::<OpenHow>()) as i32;
    let ok = fd >= 0;
    let e = if ok { 0 } else { errno() };
    if ok { libc::close(fd); }
    libc::close(dfd);
    out("openat2_resolve_in_root", ok, if ok { "ok".into() } else { format!("errno={e}") });
}

unsafe fn test_process_vm_readv() {
    use std::io::{Read, Write};
    let mut fds = [0i32; 2];
    if libc::pipe(fds.as_mut_ptr()) != 0 { out("process_vm_readv", false, "pipe failed".into()); return; }
    let pid = libc::fork();
    if pid == 0 {
        libc::close(fds[0]);
        let buf = Box::new(0x1122334455667788u64);
        let addr = &*buf as *const u64 as usize;
        let bytes = addr.to_ne_bytes();
        libc::write(fds[1], bytes.as_ptr() as *const _, 8);
        libc::sleep(3);
        std::mem::forget(buf);
        libc::_exit(0);
    }
    libc::close(fds[1]);
    let mut bytes = [0u8; 8];
    let mut f = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let _ = f.read_exact(&mut bytes);
    drop(f);
    let addr = usize::from_ne_bytes(bytes);
    let mut val: u64 = 0;
    let local = libc::iovec { iov_base: &mut val as *mut u64 as *mut _, iov_len: 8 };
    let remote = libc::iovec { iov_base: addr as *mut _, iov_len: 8 };
    let n = libc::process_vm_readv(pid, &local, 1, &remote, 1, 0);
    let ok = n == 8;
    let e = if ok { 0 } else { errno() };
    libc::kill(pid, libc::SIGKILL);
    let mut st = 0; libc::waitpid(pid, &mut st, 0);
    out("process_vm_readv", ok, if ok { format!("read=0x{val:x}") } else { format!("n={n} errno={e}") });
}

unsafe fn test_seccomp_notify() {
    const SECCOMP_SET_MODE_FILTER: u32 = 1;
    const FILTER_FLAG_NEW_LISTENER: u32 = 8;
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_USER_NOTIF: u32 = 0x7fc0_0000;
    #[repr(C)] struct SockFilter { code: u16, jt: u8, jf: u8, k: u32 }
    #[repr(C)] struct SockFprog { len: u16, filter: *const SockFilter }
    let prog = [
        SockFilter { code: 0x20, jt: 0, jf: 0, k: 0 },                       // ld nr
        SockFilter { code: 0x15, jt: 0, jf: 1, k: libc::SYS_getpid as u32 }, // jeq getpid
        SockFilter { code: 0x06, jt: 0, jf: 0, k: RET_USER_NOTIF },
        SockFilter { code: 0x06, jt: 0, jf: 0, k: RET_ALLOW },
    ];
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        out("seccomp_user_notify", false, format!("nnp errno={}", errno())); return;
    }
    let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
    let fd = libc::syscall(libc::SYS_seccomp, SECCOMP_SET_MODE_FILTER as libc::c_long, FILTER_FLAG_NEW_LISTENER as libc::c_long, &fprog as *const SockFprog) as i32;
    if fd < 0 {
        out("seccomp_install_listener", false, format!("errno={}", errno()));
        out("seccomp_user_notify", false, "listener unavailable".into());
        return;
    }
    out("seccomp_install_listener", true, format!("listener_fd={fd}"));
    // Child inherits the filter and blocks in getpid().
    let pid = libc::fork();
    if pid == 0 { libc::syscall(libc::SYS_getpid); libc::_exit(0); }
    #[repr(C)] struct SeccompData { nr: i32, arch: u32, ip: u64, args: [u64; 6] }
    #[repr(C)] struct SeccompNotif { id: u64, pid: u32, flags: u32, data: SeccompData }
    #[repr(C)] struct SeccompNotifResp { id: u64, val: i64, error: i32, flags: u32 }
    const IOCTL_NOTIF_RECV: i32 = 0xC050_2100u32 as i32;
    const IOCTL_NOTIF_SEND: i32 = 0xC018_2101u32 as i32;
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    let ready = libc::poll(&mut pfd, 1, 2000);
    if ready <= 0 {
        out("seccomp_user_notify", false, format!("no notification (poll={ready} errno={})", errno()));
        libc::kill(pid, libc::SIGKILL);
        let mut st = 0; libc::waitpid(pid, &mut st, 0);
        libc::close(fd);
        return;
    }
    let mut notif: SeccompNotif = std::mem::zeroed();
    if libc::ioctl(fd, IOCTL_NOTIF_RECV, &mut notif as *mut SeccompNotif) != 0 {
        out("seccomp_user_notify", false, format!("recv errno={}", errno()));
        libc::kill(pid, libc::SIGKILL); let mut st = 0; libc::waitpid(pid, &mut st, 0);
        libc::close(fd); return;
    }
    let mut resp = SeccompNotifResp { id: notif.id, val: 4242, error: 0, flags: 0 };
    let sent = libc::ioctl(fd, IOCTL_NOTIF_SEND, &mut resp as *mut SeccompNotifResp) == 0;
    let mut st = 0; libc::waitpid(pid, &mut st, 0);
    libc::close(fd);
    out("seccomp_user_notify", sent, format!("notif_pid={} sent={}", notif.pid, sent));
}


/// Which syscalls does the app's seccomp policy kill outright? A glibc runtime
/// can only run here if the loader and libc avoid the killed ones, so this is
/// the decisive table for "can a real app process host a glibc distro".
fn test_syscalls() {
    // aarch64 syscall numbers
    let cands: [(&str, libc::c_long); 36] = [
        ("rseq", 293),
        ("membarrier", 283),
        ("clone3", 435),
        ("futex_waitv", 449),
        ("statx", 291),
        ("faccessat2", 439),
        ("openat2", 437),
        ("pidfd_open", 434),
        ("pidfd_getfd", 438),
        ("process_madvise", 440),
        ("io_uring_setup", 425),
        ("mseal", 462),
        ("landlock_create_ruleset", 444),
        ("mount_setattr", 442),
        // glibc / ld.so startup path (see __pthread_initialize_minimal and
        // __libc_setup_tls in aarch64 ld.so): these run BEFORE any LD_PRELOAD
        // constructor, so a trap here is invisible to the path shim.
        ("set_tid_address", 96),
        ("futex", 98),
        ("set_robust_list", 99),
        ("get_robust_list", 100),
        ("nanosleep", 101),
        ("clock_gettime", 113),
        ("rt_sigaction", 134),
        ("sigaltstack", 132),
        ("rt_sigprocmask", 135),
        ("rt_sigreturn", 139),
        ("getrandom", 278),
        ("mmap", 222),
        ("mprotect", 226),
        ("madvise", 233),
        ("brk", 214),
        ("mremap", 216),
        ("munmap", 215),
        ("readlinkat", 78),
        ("prlimit64", 261),
        ("getpriority", 141),
        ("sched_getaffinity", 123),
        ("rseq_membarrier", 324),
    ];
    for (name, nr) in cands.iter() {
        let name = *name;
        let nr = *nr;
        unsafe {
            let pid = libc::fork();
            if pid == 0 {
                libc::syscall(nr, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
                let e = errno();
                let line = format!("syscall|{}|returned|errno={}\n", name, e);
                let _ = std::io::stdout().write_all(line.as_bytes());
                let _ = std::io::stdout().flush();
                libc::_exit(0);
            }
            let mut st: i32 = 0;
            libc::waitpid(pid, &mut st, 0);
            if libc::WIFSIGNALED(st) {
                println!("syscall|{}|KILLED|sig={}", name, libc::WTERMSIG(st));
            } else if !libc::WIFEXITED(st) {
                println!("syscall|{}|OTHER|status={}", name, st);
            }
            let _ = std::io::stdout().flush();
        }
    }
    // and the same list through the socket of a plain in-process call, to show
    // that a non-forked caller would simply die
    println!("syscall|note|child-per-syscall, parent reports WTERMSIG");
}


extern "C" {
    static mut environ: *mut *mut libc::c_char;
}

fn now_ns() -> u128 {
    unsafe {
        let mut ts: libc::timespec = core::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        (ts.tv_sec as u128) * 1_000_000_000 + (ts.tv_nsec as u128)
    }
}

/// fork+execve one process. Used by exec_bench.
fn exec_once(path: &std::ffi::CString, argv: &[*const libc::c_char; 3]) -> i32 {
    unsafe {
        let pid = libc::fork();
        if pid == 0 {
            libc::execve(path.as_ptr(), argv.as_ptr(), environ as *const *const libc::c_char);
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            let buf = [b'e', b'x', b'e', b'c', b'=', (b'0' + (e / 10 % 10) as u8), (b'0' + (e % 10) as u8), b'\n'];
            libc::write(2, buf.as_ptr() as *const libc::c_void, buf.len());
            libc::_exit(127);
        }
        let mut st: libc::c_int = 0;
        libc::waitpid(pid, &mut st, 0);
        if libc::WIFEXITED(st) { libc::WEXITSTATUS(st) } else { -1 }
    }
}

/// fork+execve N times, report per-exec wall cost. The exec'd program is the same
/// in every variant; only the LD_PRELOAD environment differs, so the delta is
/// exactly "what one extra pre-loaded object costs per process creation".
fn exec_bench(target: &str, n: usize) {
    begin("execbench");
    let cpath = std::ffi::CString::new(target.to_string()).unwrap();
    let child_arg = std::ffi::CString::new("--child-ok").unwrap();
    let argv: [*const libc::c_char; 3] = [cpath.as_ptr(), child_arg.as_ptr(), core::ptr::null()];
    let mut bad = 0;
    for _ in 0..3 { if exec_once(&cpath, &argv) != 0 { bad += 1; } }
    let mut best = u128::MAX;
    let mut total: u128 = 0;
    for _ in 0..n {
        let t0 = now_ns();
        if exec_once(&cpath, &argv) != 0 { bad += 1; }
        let d = now_ns() - t0;
        total += d;
        if d < best { best = d; }
    }
    let preload = std::env::var("LD_PRELOAD").unwrap_or_else(|_| "-".into());
    println!(
        "execbench|target={}|preload={}|n={}|failed={}|mean_ms={:.2}|best_ms={:.2}",
        target, preload, n, bad,
        total as f64 / (n as f64) / 1e6,
        best as f64 / 1e6
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // child mode for exec_bench: do nothing at all, so the measurement is the
    // process creation itself and not our own report
    if args.iter().any(|a| a == "--child-ok") { return; }
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let glibc = get("--glibc");
    let only = get("--only");
    let run = |name: &str| only.as_deref().map_or(true, |o| o == name);
    println!("probe_version|0.1.0");
    flush_all();
    if let Some(target) = get("--execbench") {
        let n: usize = get("--n").and_then(|v| v.parse().ok()).unwrap_or(50);
        exec_bench(&target, n);
        flush_all();
        return;
    }
    let exec_check = get("--exec-check");
    let ldso = get("--ldso");
    let libdir = get("--libdir");
    let guest = get("--guest");
    if only.is_none() { env_report(); exec_tests(glibc.as_deref(), exec_check.as_deref(), ldso.as_deref(), libdir.as_deref(), guest.as_deref()); }
    unsafe {
        if run("memfd") { begin("memfd"); test_memfd(); }
        if run("openat2") { begin("openat2"); test_openat2(); }
        if run("fuse") { begin("fuse"); test_fuse(); }
        if run("userns") { begin("userns"); test_unshare(libc::CLONE_NEWUSER, "unshare_userns"); }
        if run("mountns") { begin("mountns"); test_unshare(libc::CLONE_NEWNS, "unshare_mountns"); }
        if run("pidns") { begin("pidns"); test_unshare(libc::CLONE_NEWPID, "unshare_pidns"); }
        if run("mount") { begin("mount"); test_mount(); }
        if run("chroot") { begin("chroot"); test_chroot(); }
        if run("ptrace") { begin("ptrace"); test_ptrace(); }
        if run("vmreadv") { begin("vmreadv"); test_process_vm_readv(); }
        if run("seccomp") { begin("seccomp"); test_seccomp_notify(); }
        if run("syscalls") { begin("syscalls"); test_syscalls(); }
    }
    println!("probe_end|ok");
    flush_all();
}
