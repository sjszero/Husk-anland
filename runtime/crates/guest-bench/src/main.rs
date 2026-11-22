//! Micro-benchmarks for the runtime, built for the *guest* (aarch64 glibc) and
//! run through the modified loader, so every measurement is syscall + resolver
//! cost only: the shell is not in the timed path.
//!
//!   bench syscall     N x getpid(): the per-syscall floor in this process.
//!                     Any trap-and-forward design pays this plus a round trip.
//!   bench stat-real   stat("$NR_ROOTFS/etc/os-release")   passthrough
//!   bench stat-guest  stat("/etc/os-release")             resolver + syscall
//!   bench stat-deep   stat("/usr/lib/aarch64-linux-gnu/libc.so.6")
//!   bench open-real   open+close("$NR_ROOTFS/etc/os-release")
//!   bench open-guest  open+close("/etc/os-release")
//!   bench exec        fork+execve+waitpid("/usr/bin/true"), 100 children
//!
//! execve, not execl: glibc's execl() builds argv and calls its own internal
//! execve, which a GOT-level shim cannot see, so an execl() here would measure
//! the unshimmed libc path.

use core::ffi::c_char;
use std::ffi::CString;
use std::time::Instant;

extern "C" {
    fn fork() -> i32;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> i32;
    fn waitpid(pid: i32, status: *mut i32, opts: i32) -> i32;
    fn _exit(code: i32) -> !;
    fn getpid() -> i32;
    static mut environ: *const *const c_char;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");
    if mode.is_empty() {
        eprintln!("usage: bench <mode>");
        std::process::exit(2);
    }
    let root = match std::env::var("NR_ROOTFS") {
        Ok(r) => r,
        Err(_) => {
            eprintln!("NR_ROOTFS unset");
            std::process::exit(2);
        }
    };

    match mode {
        "exec" => {
            let n = 100;
            let path = CString::new("/usr/bin/true").unwrap();
            let arg0 = CString::new("true").unwrap();
            let t0 = Instant::now();
            let mut ok = 0;
            for _ in 0..n {
                let pid = unsafe { fork() };
                if pid == 0 {
                    let argv = [arg0.as_ptr(), core::ptr::null()];
                    unsafe {
                        execve(path.as_ptr(), argv.as_ptr(), environ);
                        _exit(127);
                    }
                }
                if pid > 0 {
                    let mut st: i32 = 0;
                    if unsafe { waitpid(pid, &mut st, 0) } == pid && (st & 0x7f) == 0 && (st >> 8) == 0 {
                        ok += 1;
                    }
                }
            }
            let ns = t0.elapsed().as_nanos() as f64 / n as f64;
            println!("{:<28} {:8.0} ns/op  ({}/{} ok)", "fork+execve+wait", ns, ok, n);
            std::process::exit(if ok == n { 0 } else { 1 });
        }
        "syscall" => {
            let n = 200_000;
            let t0 = Instant::now();
            let mut sink = 0;
            for _ in 0..n {
                sink = unsafe { getpid() };
            }
            let ns = t0.elapsed().as_nanos() as f64 / n as f64;
            println!("{:<28} {:8.1} ns/op  ({} calls, sink={})", "getpid", ns, n, sink);
        }
        m if m.starts_with("stat") => {
            let (path, label) = match m {
                "stat-real" => (format!("{root}/etc/os-release"), "stat-real"),
                "stat-deep" => ("/usr/lib/aarch64-linux-gnu/libc.so.6".to_string(), "stat-deep"),
                _ => ("/etc/os-release".to_string(), "stat-guest"),
            };
            let n = 20_000;
            let t0 = Instant::now();
            let mut ok = 0;
            for _ in 0..n {
                if std::fs::metadata(&path).is_ok() {
                    ok += 1;
                }
            }
            let ns = t0.elapsed().as_nanos() as f64 / n as f64;
            println!("{:<28} {:8.1} ns/op  ({}/{} ok)", label, ns, ok, n);
            std::process::exit(if ok == n { 0 } else { 1 });
        }
        m if m.starts_with("open") => {
            let (path, label) = match m {
                "open-real" => (format!("{root}/etc/os-release"), "open-real"),
                _ => ("/etc/os-release".to_string(), "open-guest"),
            };
            let n = 20_000;
            let t0 = Instant::now();
            let mut ok = 0;
            for _ in 0..n {
                if std::fs::File::open(&path).is_ok() {
                    ok += 1;
                }
            }
            let ns = t0.elapsed().as_nanos() as f64 / n as f64;
            println!("{:<28} {:8.1} ns/op  ({}/{} ok)", label, ns, ok, n);
            std::process::exit(if ok == n { 0 } else { 1 });
        }
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }
}
