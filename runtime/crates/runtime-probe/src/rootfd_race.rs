use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;

fn errno() -> libc::c_int {
    unsafe { *libc::__errno_location() }
}

fn note_errno(slot: &AtomicI32, value: libc::c_int) {
    let _ = slot.compare_exchange(0, value, Ordering::Relaxed, Ordering::Relaxed);
}

pub fn run() -> i32 {
    let stop = Arc::new(AtomicBool::new(false));
    let stat_ok = Arc::new(AtomicUsize::new(0));
    let stat_fail = Arc::new(AtomicUsize::new(0));
    let access_ok = Arc::new(AtomicUsize::new(0));
    let access_fail = Arc::new(AtomicUsize::new(0));
    let range_ok = Arc::new(AtomicUsize::new(0));
    let range_fail = Arc::new(AtomicUsize::new(0));
    let first_errno = Arc::new(AtomicI32::new(0));

    let mut readers = Vec::with_capacity(8);
    for _ in 0..8 {
        let stop = Arc::clone(&stop);
        let stat_ok = Arc::clone(&stat_ok);
        let stat_fail = Arc::clone(&stat_fail);
        let access_ok = Arc::clone(&access_ok);
        let access_fail = Arc::clone(&access_fail);
        let first_errno = Arc::clone(&first_errno);
        readers.push(std::thread::spawn(move || {
            let os_release = CString::new("/etc/os-release").unwrap();
            let bash = CString::new("/usr/bin/bash").unwrap();
            while !stop.load(Ordering::Relaxed) {
                unsafe {
                    let mut st: libc::stat = std::mem::zeroed();
                    *libc::__errno_location() = 0;
                    if libc::stat(os_release.as_ptr(), &mut st) == 0 {
                        stat_ok.fetch_add(1, Ordering::Relaxed);
                    } else {
                        stat_fail.fetch_add(1, Ordering::Relaxed);
                        note_errno(&first_errno, errno());
                    }
                    *libc::__errno_location() = 0;
                    if libc::access(bash.as_ptr(), libc::X_OK) == 0 {
                        access_ok.fetch_add(1, Ordering::Relaxed);
                    } else {
                        access_fail.fetch_add(1, Ordering::Relaxed);
                        note_errno(&first_errno, errno());
                    }
                }
            }
        }));
    }

    let mut closers = Vec::with_capacity(2);
    for _ in 0..2 {
        let range_ok = Arc::clone(&range_ok);
        let range_fail = Arc::clone(&range_fail);
        let first_errno = Arc::clone(&first_errno);
        closers.push(std::thread::spawn(move || {
            for _ in 0..250 {
                unsafe {
                    *libc::__errno_location() = 0;
                    if libc::close_range(1024, 1024, 0) == 0 {
                        range_ok.fetch_add(1, Ordering::Relaxed);
                    } else {
                        range_fail.fetch_add(1, Ordering::Relaxed);
                        note_errno(&first_errno, errno());
                    }
                }
            }
        }));
    }
    for closer in closers {
        let _ = closer.join();
    }
    stop.store(true, Ordering::Relaxed);
    for reader in readers {
        let _ = reader.join();
    }

    println!(
        "ROOTFD_RACE stat_ok={} stat_fail={} access_ok={} access_fail={} range_ok={} range_fail={} first_errno={}",
        stat_ok.load(Ordering::Relaxed),
        stat_fail.load(Ordering::Relaxed),
        access_ok.load(Ordering::Relaxed),
        access_fail.load(Ordering::Relaxed),
        range_ok.load(Ordering::Relaxed),
        range_fail.load(Ordering::Relaxed),
        first_errno.load(Ordering::Relaxed)
    );
    if stat_fail.load(Ordering::Relaxed) == 0
        && access_fail.load(Ordering::Relaxed) == 0
        && range_fail.load(Ordering::Relaxed) == 0
    {
        0
    } else {
        1
    }
}
