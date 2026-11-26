use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

const READERS: usize = 2;
const WRITERS: usize = 1;
const ITERS: usize = 20_000;
const MAX_GROUPS: usize = 32;
const CANARY: u32 = 0x51a7_c0de;

static FAILURES: AtomicUsize = AtomicUsize::new(0);
static SIGNAL_CALLS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_alarm(_sig: libc::c_int) {
    let mut groups = [0u32; MAX_GROUPS];
    unsafe {
        let count = libc::getgroups(0, std::ptr::null_mut());
        if count > 0 && count as usize <= MAX_GROUPS {
            libc::getgroups(count, groups.as_mut_ptr());
        }
    }
    SIGNAL_CALLS.fetch_add(1, Ordering::Relaxed);
}

fn fail(what: &str, rc: libc::c_int, err: libc::c_int) {
    FAILURES.fetch_add(1, Ordering::Relaxed);
    eprintln!("GROUPS_RACE_FAIL what={} rc={} errno={}", what, rc, err);
}

#[repr(C)]
struct GuardedSlot {
    before: u32,
    values: [u32; 1],
    after: u32,
}

fn reader(barrier: &Barrier) {
    barrier.wait();
    for _ in 0..ITERS {
        let mut slot = GuardedSlot {
            before: CANARY,
            values: [0],
            after: CANARY,
        };
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
        unsafe {
            *libc::__errno_location() = 0;
            let rc = libc::getgroups(1, slot.values.as_mut_ptr());
            let err = *libc::__errno_location();
            std::sync::atomic::compiler_fence(Ordering::SeqCst);
            if slot.before != CANARY || slot.after != CANARY {
                fail("buffer-overrun", rc, err);
                return;
            }
            if rc < 0 && err != libc::EINVAL {
                fail("unexpected-errno", rc, err);
                return;
            }
            if rc > 1 {
                fail("impossible-count", rc, err);
                return;
            }
        }
    }
}

fn writer(barrier: &Barrier, id: usize) {
    barrier.wait();
    let one = [1500u32];
    let many: Vec<u32> = (0..MAX_GROUPS).map(|i| 2000 + i as u32).collect();
    for i in 0..ITERS {
        let groups: &[u32] = if (i + id) & 1 == 0 { &one } else { &many };
        let rc = unsafe { libc::setgroups(groups.len(), groups.as_ptr()) };
        if rc != 0 {
            fail("setgroups", rc, unsafe { *libc::__errno_location() });
            return;
        }
    }
}

pub fn run() -> i32 {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_alarm as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut()) != 0 {
            return 2;
        }
        let timer = libc::itimerval {
            it_interval: libc::timeval {
                tv_sec: 0,
                tv_usec: 1000,
            },
            it_value: libc::timeval {
                tv_sec: 0,
                tv_usec: 1000,
            },
        };
        if libc::setitimer(libc::ITIMER_REAL, &timer, std::ptr::null_mut()) != 0 {
            return 2;
        }
    }

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        unsafe {
            let msg = b"GROUPS_RACE_FAIL watchdog-timeout\n";
            libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(124);
        }
    });

    let barrier = Arc::new(Barrier::new(READERS + WRITERS));
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || reader(&barrier));
        }
        for id in 0..WRITERS {
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || writer(&barrier, id));
        }
    });

    unsafe {
        let zero = libc::itimerval {
            it_interval: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            it_value: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
        };
        libc::setitimer(libc::ITIMER_REAL, &zero, std::ptr::null_mut());
        let one = [1500u32];
        libc::setgroups(1, one.as_ptr());
    }

    let failures = FAILURES.load(Ordering::Relaxed);
    let signals = SIGNAL_CALLS.load(Ordering::Relaxed);
    println!(
        "GROUPS_RACE failures={} signals={} expected=0",
        failures, signals
    );
    if failures == 0 && signals != 0 {
        0
    } else {
        1
    }
}
