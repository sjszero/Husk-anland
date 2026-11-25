//! Out-of-process session guard for the Husk Android shell.
//!
//! The App holds the write end of a lease pipe and gives this process the read
//! end. Android can kill the App without delivering onDestroy; in that case the
//! kernel closes the write end and this process wakes on EOF. It then finds
//! every process carrying the session's private `NR_SESSION_ID`, including
//! daemons that escaped with setsid(), and escalates SIGTERM -> SIGKILL.

use core::ffi::{c_int, c_void};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

extern "C" {
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn setsid() -> c_int;
    fn __errno() -> *mut c_int;
}

const SIGKILL: c_int = 9;
const SIGTERM: c_int = 15;
const EINTR: c_int = 4;
const GUARD_MAGIC: u32 = 0x4853_4b47; // "HSKG"
const GUARD_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GuardArm {
    magic: u32,
    version: u32,
    leader_pid: i32,
    _reserved: u32,
    leader_start_ticks: u64,
}

struct Logger {
    file: Option<std::fs::File>,
}

impl Logger {
    fn open(path: Option<&str>) -> Logger {
        let file = path.and_then(|p| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(p))
                .ok()
        });
        Logger { file }
    }

    fn line(&mut self, text: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(
            file,
            "{stamp} guard[{pid}]: {text}",
            pid = std::process::id()
        );
        let _ = file.flush();
    }
}

fn main() {
    let mut lease_fd = -1;
    let mut ready_fd = -1;
    let mut session_id = String::new();
    let mut log_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lease-fd" => {
                lease_fd = args.next().and_then(|v| v.parse().ok()).unwrap_or(-1);
            }
            "--ready-fd" => {
                ready_fd = args.next().and_then(|v| v.parse().ok()).unwrap_or(-1);
            }
            "--session-id" => session_id = args.next().unwrap_or_default(),
            "--log" => log_path = args.next(),
            _ => {}
        }
    }
    let mut log = Logger::open(log_path.as_deref());
    if lease_fd < 3 {
        log.line("bad --lease-fd");
        std::process::exit(64);
    }
    if session_id.is_empty() {
        log.line("empty --session-id");
        std::process::exit(64);
    }
    unsafe {
        setsid();
    }
    log.line(&format!("started lease_fd={lease_fd} session={session_id}"));
    if ready_fd >= 0 {
        let ready = b'R';
        unsafe {
            write(ready_fd, &ready as *const u8 as *const c_void, 1);
            close(ready_fd);
        }
    }

    let arm = read_arm(lease_fd, &mut log);
    if arm.magic == GUARD_MAGIC && arm.version == GUARD_VERSION {
        log.line(&format!(
            "armed leader={} start_ticks={}",
            arm.leader_pid, arm.leader_start_ticks
        ));
    } else {
        log.line("lease closed before arm; using marker-only cleanup");
    }

    // Any byte means the App deliberately closed the session; EOF means the App
    // process died. Both take the same cleanup path.
    let mut byte = 0u8;
    loop {
        let n = unsafe { read(lease_fd, &mut byte as *mut u8 as *mut c_void, 1) };
        if n > 0 {
            log.line("lease command received");
            break;
        }
        if n == 0 {
            log.line("lease EOF: App process is gone");
            break;
        }
        let errno = unsafe { *__errno() };
        if errno == EINTR {
            continue;
        }
        log.line(&format!("lease read failed errno={errno}"));
        break;
    }

    terminate(&session_id, &arm, &mut log);
}

fn read_arm(fd: c_int, log: &mut Logger) -> GuardArm {
    let mut arm = GuardArm::default();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(
            &mut arm as *mut GuardArm as *mut u8,
            core::mem::size_of::<GuardArm>(),
        )
    };
    let mut off = 0;
    while off < bytes.len() {
        let n = unsafe {
            read(
                fd,
                bytes[off..].as_mut_ptr() as *mut c_void,
                bytes.len() - off,
            )
        };
        if n > 0 {
            off += n as usize;
            continue;
        }
        if n == 0 {
            return GuardArm::default();
        }
        let errno = unsafe { *__errno() };
        if errno == EINTR {
            continue;
        }
        log.line(&format!("arm read failed errno={errno}"));
        return GuardArm::default();
    }
    arm
}

fn marker_pids(session_id: &str) -> Vec<i32> {
    let needle = format!("NR_SESSION_ID={session_id}");
    let self_pid = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        if pid <= 0 || pid == self_pid {
            continue;
        }
        let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
            continue;
        };
        if env.split(|b| *b == 0).any(|item| item == needle.as_bytes()) {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn start_ticks(pid: i32) -> u64 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0;
    };
    let Some(end) = stat.rfind(')') else { return 0 };
    stat[end + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn group_alive(pid: i32, expected_ticks: u64) -> bool {
    if pid <= 0 {
        return false;
    }
    let now = start_ticks(pid);
    if now != 0 && expected_ticks != 0 && now != expected_ticks {
        return false;
    }
    unsafe { kill(-pid, 0) == 0 }
}

fn signal_pids(pids: &[i32], sig: c_int, log: &mut Logger) {
    for &pid in pids {
        let rc = unsafe { kill(pid, sig) };
        if rc == 0 {
            log.line(&format!("sent signal={sig} pid={pid}"));
        }
    }
}

fn terminate(session_id: &str, arm: &GuardArm, log: &mut Logger) {
    let mut pids = marker_pids(session_id);
    log.line(&format!("cleanup marker_pids={pids:?}"));
    if group_alive(arm.leader_pid, arm.leader_start_ticks) {
        unsafe { kill(-arm.leader_pid, SIGTERM) };
        log.line(&format!("sent signal={SIGTERM} group={}", arm.leader_pid));
    }
    signal_pids(&pids, SIGTERM, log);

    std::thread::sleep(Duration::from_millis(300));

    pids = marker_pids(session_id);
    if group_alive(arm.leader_pid, arm.leader_start_ticks) {
        unsafe { kill(-arm.leader_pid, SIGKILL) };
        log.line(&format!("sent signal={SIGKILL} group={}", arm.leader_pid));
    }
    signal_pids(&pids, SIGKILL, log);
    std::thread::sleep(Duration::from_millis(100));
    log.line(&format!("done remaining={:?}", marker_pids(session_id)));
}
