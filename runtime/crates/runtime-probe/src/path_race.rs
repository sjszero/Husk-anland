use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const TOGGLE_ROUNDS: usize = 60_000;
const WORKER_ROUNDS: usize = 30_000;
const WORKERS: usize = 4;

#[derive(Clone)]
struct Paths {
    base: CString,
    race_link: CString,
    guest_target: CString,
    next0: CString,
    next1: CString,
    host_target: CString,
    guest_link: CString,
}

fn cpath(value: impl Into<Vec<u8>>) -> CString {
    CString::new(value).expect("path has no NUL")
}

fn errno() -> libc::c_int {
    unsafe { *libc::__errno_location() }
}

fn toggler(paths: Arc<Paths>, stop: Arc<AtomicBool>, toggles: Arc<AtomicUsize>) {
    for i in 0..TOGGLE_ROUNDS {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let temp = if i & 1 == 0 {
            &paths.next0
        } else {
            &paths.next1
        };
        let target: *const c_char = if i & 1 == 0 {
            paths.guest_link.as_ptr()
        } else {
            paths.host_target.as_ptr()
        };
        unsafe {
            libc::unlink(temp.as_ptr());
            if libc::symlink(target, temp.as_ptr()) == 0
                && libc::rename(temp.as_ptr(), paths.race_link.as_ptr()) == 0
            {
                toggles.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn worker(
    paths: Arc<Paths>,
    seed: usize,
    operations: Arc<AtomicUsize>,
    successes: Arc<AtomicUsize>,
) {
    for i in 0..WORKER_ROUNDS {
        let times = [
            libc::timespec {
                tv_sec: 1_600_000_000 + (i & 31) as i64,
                tv_nsec: ((i % 1000) * 1_000_000) as i64,
            },
            libc::timespec {
                tv_sec: 1_600_000_000 + ((i + 1) & 31) as i64,
                tv_nsec: (((i + 1) % 1000) * 1_000_000) as i64,
            },
        ];
        unsafe {
            if libc::chmod(
                paths.race_link.as_ptr(),
                if i & 1 == 0 { 0o644 } else { 0o600 },
            ) == 0
            {
                successes.fetch_add(1, Ordering::Relaxed);
            }
            operations.fetch_add(1, Ordering::Relaxed);

            if (i + seed) % 3 == 0 {
                if libc::utimensat(libc::AT_FDCWD, paths.race_link.as_ptr(), times.as_ptr(), 0) == 0
                {
                    successes.fetch_add(1, Ordering::Relaxed);
                }
                operations.fetch_add(1, Ordering::Relaxed);
            }

            if (i + seed) % 5 == 0 {
                let fd = libc::open(
                    paths.race_link.as_ptr(),
                    libc::O_WRONLY | libc::O_TRUNC | libc::O_CLOEXEC,
                );
                if fd >= 0 {
                    successes.fetch_add(1, Ordering::Relaxed);
                    libc::close(fd);
                }
                operations.fetch_add(1, Ordering::Relaxed);
            }

            if (i + seed) % 7 == 0 {
                let mut st: libc::stat = std::mem::zeroed();
                if libc::lstat(paths.race_link.as_ptr(), &mut st) == 0 {
                    successes.fetch_add(1, Ordering::Relaxed);
                }
                operations.fetch_add(1, Ordering::Relaxed);
            }

            if (i + seed) % 11 == 0 {
                if libc::chown(paths.race_link.as_ptr(), !0u32, !0u32) == 0 {
                    successes.fetch_add(1, Ordering::Relaxed);
                }
                operations.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub fn run(host_sentinel: &str) -> i32 {
    let paths = Arc::new(Paths {
        base: cpath("/tmp/husk-path-race"),
        race_link: cpath("/tmp/husk-path-race/race-link"),
        guest_target: cpath("/tmp/husk-path-race/guest-victim"),
        next0: cpath("/tmp/husk-path-race/.next-0"),
        next1: cpath("/tmp/husk-path-race/.next-1"),
        host_target: cpath(host_sentinel.as_bytes().to_vec()),
        guest_link: cpath("guest-victim"),
    });

    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        *libc::__errno_location() = 0;
        if libc::lstat(paths.host_target.as_ptr(), &mut st) == 0 {
            println!(
                "FAIL host sentinel is visible inside guest: {}",
                host_sentinel
            );
            return 65;
        }
        println!("PASS host-sentinel-not-visible errno={}", errno());

        libc::unlink(paths.race_link.as_ptr());
        libc::unlink(paths.guest_target.as_ptr());
        libc::rmdir(paths.base.as_ptr());
        if libc::mkdir(paths.base.as_ptr(), 0o700) != 0 && errno() != libc::EEXIST {
            return 66;
        }
        let fd = libc::open(
            paths.guest_target.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o644,
        );
        if fd < 0 {
            return 67;
        }
        if libc::write(fd, b"guest\n".as_ptr().cast(), 6) != 6 {
            libc::close(fd);
            return 68;
        }
        libc::close(fd);
        if libc::symlink(paths.guest_link.as_ptr(), paths.race_link.as_ptr()) != 0 {
            return 69;
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let toggles = Arc::new(AtomicUsize::new(0));
    let operations = Arc::new(AtomicUsize::new(0));
    let successes = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        {
            let paths = Arc::clone(&paths);
            let stop = Arc::clone(&stop);
            let toggles = Arc::clone(&toggles);
            scope.spawn(move || toggler(paths, stop, toggles));
        }
        for id in 0..WORKERS {
            let paths = Arc::clone(&paths);
            let operations = Arc::clone(&operations);
            let successes = Arc::clone(&successes);
            scope.spawn(move || worker(paths, id + 1, operations, successes));
        }
    });
    stop.store(true, Ordering::Relaxed);

    unsafe {
        libc::unlink(paths.next0.as_ptr());
        libc::unlink(paths.next1.as_ptr());
        libc::unlink(paths.race_link.as_ptr());
        libc::unlink(paths.guest_target.as_ptr());
        libc::rmdir(paths.base.as_ptr());
    }

    let n_toggles = toggles.load(Ordering::Relaxed);
    let n_ops = operations.load(Ordering::Relaxed);
    let n_ok = successes.load(Ordering::Relaxed);
    println!(
        "STATS toggles={} operations={} successes={}",
        n_toggles, n_ops, n_ok
    );
    if n_toggles < 1000 {
        println!("FAIL insufficient toggle interleavings");
        return 72;
    }
    if n_ops < 1000 {
        println!("FAIL insufficient metadata operations");
        return 73;
    }
    println!("PATH_RACE_DONE");
    0
}
