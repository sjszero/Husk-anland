use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

const READERS: usize = 15;

static SIGNAL_CALLS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_alarm(_sig: libc::c_int) {
    unsafe {
        let mut value = libc::getuid() ^ libc::getgid();
        value = value.wrapping_add(1);
        std::hint::black_box(value);
    }
    SIGNAL_CALLS.fetch_add(1, Ordering::Relaxed);
}

fn reader(barrier: &Barrier) {
    barrier.wait();
    let mut value = 0u32;
    for _ in 0..20_000 {
        unsafe {
            value ^= libc::getuid();
            value ^= libc::getgid();
        }
    }
    std::hint::black_box(value);
}

fn writer(barrier: &Barrier) {
    barrier.wait();
    unsafe {
        if libc::setgid(1000) != 0 || libc::setuid(1000) != 0 {
            libc::_exit(2);
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
                tv_usec: 100,
            },
            it_value: libc::timeval {
                tv_sec: 0,
                tv_usec: 100,
            },
        };
        if libc::setitimer(libc::ITIMER_REAL, &timer, std::ptr::null_mut()) != 0 {
            return 2;
        }
    }

    let barrier = Arc::new(Barrier::new(READERS + 1));
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || reader(&barrier));
        }
        let barrier = Arc::clone(&barrier);
        scope.spawn(move || writer(&barrier));
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
        let uid = libc::getuid();
        let gid = libc::getgid();
        let signals = SIGNAL_CALLS.load(Ordering::Relaxed);
        println!(
            "IDENTITY_RACE final={}:{} expected=1000:1000 signals={}",
            uid, gid, signals
        );
        if uid == 1000 && gid == 1000 && signals != 0 {
            0
        } else {
            1
        }
    }
}
