use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

const THREADS: usize = 16;
const DEFAULT_ROUNDS: usize = 4000;

fn valid(path: &str, body: &str) -> bool {
    match path {
        "/proc/mounts" => {
            let lines: Vec<_> = body.lines().collect();
            lines.len() == 6
                && lines[0].contains(" / ")
                && lines[1].contains(" /proc ")
                && lines[2].contains(" /sys ")
                && lines[3].contains(" /dev ")
                && lines[4].contains(" /dev/pts ")
                && lines[5].contains(" /dev/shm ")
                && !body.contains("Name:\t")
                && !body.starts_with("       key")
        }
        "/proc/self/status" => {
            body.contains("Name:\t")
                && body.contains("Uid:\t")
                && body.contains("Gid:\t")
                && body.contains("Threads:\t")
                && !body.contains(" /dev/shm ")
        }
        "/proc/sysvipc/shm" => {
            body.starts_with("       key      shmid perms")
                && !body.contains("Name:\t")
                && !body.lines().any(|line| line.contains(" /dev/shm "))
        }
        "/proc/uptime" => {
            body.ends_with('\n')
                && body.split_whitespace().count() == 2
                && !body.contains("Name:\t")
                && !body.contains(" /dev/shm ")
        }
        _ => false,
    }
}

fn main() {
    let rounds = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ROUNDS);
    let paths = [
        "/proc/mounts",
        "/proc/self/status",
        "/proc/sysvipc/shm",
        "/proc/uptime",
    ];
    let bad = Arc::new(AtomicUsize::new(0));
    let ok = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(THREADS + 1));
    let mut workers = Vec::with_capacity(THREADS);

    for tid in 0..THREADS {
        let bad = Arc::clone(&bad);
        let ok = Arc::clone(&ok);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for round in 0..rounds {
                let path = paths[(round + tid) % paths.len()];
                match fs::read_to_string(path) {
                    Ok(body) if valid(path, &body) => {
                        ok.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(body) => {
                        let n = bad.fetch_add(1, Ordering::Relaxed) + 1;
                        if n <= 4 {
                            let head: String = body.chars().take(120).collect();
                            eprintln!(
                                "BAD path={path} round={round} len={} head={head:?}",
                                body.len()
                            );
                        }
                    }
                    Err(error) => {
                        let n = bad.fetch_add(1, Ordering::Relaxed) + 1;
                        if n <= 4 {
                            eprintln!("ERR path={path} round={round} {error}");
                        }
                    }
                }
            }
        }));
    }

    barrier.wait();
    for worker in workers {
        worker.join().expect("probe worker panicked");
    }
    let bad = bad.load(Ordering::Relaxed);
    let ok = ok.load(Ordering::Relaxed);
    println!(
        "RESULT bad={bad} ok={ok} threads={THREADS} rounds={rounds} opens={}",
        ok + bad
    );
    if bad != 0 {
        std::process::exit(1);
    }
}
