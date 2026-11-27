//! Mount-semantics checks that used to live in Python heredocs.
//!
//! The shell test still creates the fake bind/tmpfs mounts. This module performs
//! the checks that need direct libc entry points: read-only write attempts,
//! directory-fd operations, xattr round-trips, and stat under mount churn.

use std::ffi::{c_int, c_void, CString};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const LINUX_EROFS: c_int = 30;

fn path(value: &str) -> CString {
    CString::new(value).expect("path has no NUL")
}

fn errno() -> c_int {
    unsafe { *libc::__errno_location() }
}

fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

fn report_failure(label: &str, rc: c_int, error: c_int) {
    eprintln!("FAIL {label} rc={rc} errno={error}");
}

fn report_erofs(label: &str, rc: c_int, error: c_int, failed: &mut bool) {
    if rc == -1 && error == LINUX_EROFS {
        println!("PASS {label} EROFS");
    } else {
        report_failure(label, rc, error);
        *failed = true;
    }
}

fn mount_bind_ro() -> c_int {
    let source = path("/mnt/rosrc");
    let target = path("/mnt/ro");
    unsafe {
        if libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND,
            std::ptr::null(),
        ) != 0
        {
            report_failure("mount bind", -1, errno());
            return 1;
        }
        clear_errno();
        if libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
            std::ptr::null(),
        ) != 0
        {
            report_failure("mount remount ro", -1, errno());
            return 1;
        }
    }
    println!("PASS mount bind read-only");
    0
}

fn mount_write() -> c_int {
    let dir_path = path("/mnt/ro");
    let file = path("f");
    let new_dir = path("nd");
    let absolute_file = path("/mnt/ro/f");
    let attr_name = path("user.husk");
    let attr_value = b"x";
    let mut failed = false;

    let dir = unsafe { libc::open(dir_path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) };
    if dir < 0 {
        report_failure("open /mnt/ro", -1, errno());
        return 1;
    }

    clear_errno();
    let fd = unsafe {
        libc::openat(
            dir,
            file.as_ptr(),
            libc::O_WRONLY | libc::O_TRUNC | libc::O_CLOEXEC,
        )
    };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        report_failure("openat write", fd, 0);
        failed = true;
    } else {
        report_erofs("openat write", fd, errno(), &mut failed);
    }

    clear_errno();
    let rc = unsafe { libc::mkdirat(dir, new_dir.as_ptr(), 0o755) };
    report_erofs("mkdirat", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::unlinkat(dir, file.as_ptr(), 0) };
    report_erofs("unlinkat", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::fchmodat(dir, file.as_ptr(), 0o600, 0) };
    report_erofs("fchmodat", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::fchownat(dir, file.as_ptr(), 0, 0, 0) };
    report_erofs("fchownat", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::utimensat(dir, file.as_ptr(), std::ptr::null(), 0) };
    report_erofs("utimensat", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::chmod(absolute_file.as_ptr(), 0o600) };
    report_erofs("chmod", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::chown(absolute_file.as_ptr(), 0, 0) };
    report_erofs("chown", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe { libc::truncate(absolute_file.as_ptr(), 0) };
    report_erofs("truncate", rc, errno(), &mut failed);

    clear_errno();
    let rc = unsafe {
        libc::setxattr(
            absolute_file.as_ptr(),
            attr_name.as_ptr(),
            attr_value.as_ptr().cast::<c_void>(),
            attr_value.len(),
            0,
        )
    };
    report_erofs("setxattr", rc, errno(), &mut failed);

    unsafe { libc::close(dir) };
    i32::from(failed)
}

fn xattr_roundtrip(target: &str) -> c_int {
    let target = path(target);
    let name = path("user.husk");
    let value = b"hello";
    let mut readback = [0_u8; 16];

    clear_errno();
    let set = unsafe {
        libc::setxattr(
            target.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast::<c_void>(),
            value.len(),
            0,
        )
    };
    if set != 0 {
        report_failure("setxattr", set, errno());
        return 1;
    }

    clear_errno();
    let got = unsafe {
        libc::getxattr(
            target.as_ptr(),
            name.as_ptr(),
            readback.as_mut_ptr().cast::<c_void>(),
            readback.len(),
        )
    };
    if got != value.len() as isize || &readback[..got as usize] != value {
        report_failure("getxattr", got as c_int, errno());
        return 1;
    }

    clear_errno();
    let removed = unsafe { libc::removexattr(target.as_ptr(), name.as_ptr()) };
    if removed != 0 {
        report_failure("removexattr", removed, errno());
        return 1;
    }
    println!("PASS xattr set/get/remove");
    0
}

fn mount_race() -> c_int {
    if let Err(error) = std::env::set_current_dir("/") {
        eprintln!("FAIL chdir /: {error}");
        return 1;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let churn_stop = Arc::clone(&stop);
    let churn = thread::spawn(move || {
        let mut i = 0_u64;
        while !churn_stop.load(Ordering::Relaxed) {
            let dir = format!("/mnt/sc{}", i % 3);
            let _ = Command::new("mount")
                .args(["-t", "tmpfs", "tmpfs", &dir])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("umount")
                .arg(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            i += 1;
        }
    });

    let mut walkers = Vec::new();
    for _ in 0..6 {
        let walker_errors = Arc::clone(&errors);
        walkers.push(thread::spawn(move || {
            for _ in 0..3000 {
                for name in [
                    "/usr/bin/bash",
                    "usr/bin/bash",
                    "/usr/lib",
                    "/proc/self/cmdline",
                ] {
                    let name_c = match CString::new(name) {
                        Ok(name) => name,
                        Err(_) => continue,
                    };
                    let mut st: libc::stat = unsafe { std::mem::zeroed() };
                    clear_errno();
                    if unsafe { libc::stat(name_c.as_ptr(), &mut st) } != 0 {
                        let message = format!("{name}: errno={}", errno());
                        if let Ok(mut list) = walker_errors.lock() {
                            list.push(message);
                        }
                    }
                }
            }
        }));
    }

    for walker in walkers {
        let _ = walker.join();
    }
    stop.store(true, Ordering::Relaxed);
    let _ = churn.join();
    for i in 0..3 {
        let _ = Command::new("umount")
            .arg(format!("/mnt/sc{i}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let errors = errors.lock().expect("race error list poisoned");
    if errors.is_empty() {
        println!("PASS mount churn: no resolution errors");
        0
    } else {
        eprintln!(
            "FAIL mount churn: {} resolution errors; first={}",
            errors.len(),
            errors[0]
        );
        1
    }
}

pub fn run(args: &[String]) -> c_int {
    match args.get(2).map(String::as_str) {
        Some("mount-bind-ro") => mount_bind_ro(),
        Some("mount-write") => mount_write(),
        Some("mount-race") => mount_race(),
        Some("xattr") if args.len() == 4 => xattr_roundtrip(&args[3]),
        _ => {
            eprintln!(
                "usage: runtime-probe mount-audit <mount-bind-ro|mount-write|mount-race|xattr PATH>"
            );
            64
        }
    }
}
