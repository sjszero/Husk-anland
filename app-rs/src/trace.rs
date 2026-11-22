//! ptrace "syscall snitch".
//!
//! Why: an Android app-domain seccomp trap can kill a process with SIGSYS in a
//! way that no in-process handler ever observes (and this ROM logs nothing to
//! logcat or dmesg). A tracer lives in a *different* process, so it survives the
//! kill and can still read the tracee's registers.
//!
//! Kernel order for a syscall is: ptrace entry stop -> seccomp check. So the
//! tracer normally sees the offending syscall number at its entry stop. If the
//! filter kills before that stop, the number is still in x8 at the signal stop.
//! Either way we end up with a fact, not a guess.

use crate::sys::*;
use core::ffi::{c_char, c_int, c_long, c_void};
use std::ffi::{CStr, CString};

const RING: usize = 12;
const PTRACE_O_EXITKILL: c_long = 0x0010_0000;

pub fn spy(loader: &str, libdir: &str, rootfs: &str) {
    crate::log("spy: start");
    let Ok(loader_c) = CString::new(loader) else { return };
    // The guest command comes from `files/trace` (first line, whitespace
    // separated) so the snitch can replay the exact argv that fails without
    // rebuilding the APK. Empty file = the safe default.
    let want = std::fs::read_to_string(format!("{rootfs}/../trace")).unwrap_or_default();
    let words: Vec<String> = if want.split_whitespace().count() == 0 {
        vec!["-c".to_string(), "echo TRACED_OK".to_string()]
    } else {
        want.split_whitespace().map(|s| s.to_string()).collect()
    };
    let mut argv_s: Vec<CString> = vec!["libld.so".to_string(), "--library-path".to_string(), libdir.to_string(), format!("{rootfs}/usr/bin/bash")]
        .iter()
        .filter_map(|s| CString::new(s.as_str()).ok())
        .collect();
    for w in &words {
        if let Ok(c) = CString::new(w.as_str()) { argv_s.push(c); }
    }
    crate::log(&format!("spy: argv tail = {words:?}"));
    let env_s: Vec<CString> = [
        format!("NR_ROOTFS={rootfs}"),
        "GLIBC_TUNABLES=glibc.pthread.rseq=0".to_string(),
        format!("HOME={rootfs}/root"),
        "TERM=dumb".to_string(),
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
    ]
    .iter()
    .filter_map(|s| CString::new(s.as_str()).ok())
    .collect();

    let errpath = CString::new(format!("{rootfs}/../trace.err")).ok();
    let root_c = CString::new(rootfs).ok();
    // Same pty shape as the product spawn: this is the last variable to control.
    let mfd = unsafe { posix_openpt(0o2 | 0o400) }; // O_RDWR | O_NOCTTY
    let mut ptname = [0 as c_char; 128];
    if mfd < 0 || unsafe { grantpt(mfd) } != 0 || unsafe { unlockpt(mfd) } != 0
        || unsafe { ptsname_r(mfd, ptname.as_mut_ptr(), ptname.len()) } != 0
    {
        crate::log("spy: pty setup failed");
        return;
    }
    let slave = unsafe { CStr::from_ptr(ptname.as_ptr()) }.to_owned();
    let pid = unsafe { fork() };
    if pid == 0 {
        unsafe {
            setsid();
            let sfd = open(slave.as_ptr(), 0o2);
            if sfd >= 0 {
                ioctl(sfd, 0x540E /* TIOCSCTTY */, 0usize);
                dup2(sfd, 0);
                dup2(sfd, 1);
                dup2(sfd, 2);
                if sfd > 2 { close(sfd); }
            }
            if let Some(r) = &root_c { chdir(r.as_ptr()); }
            ptrace(PTRACE_TRACEME, 0 as c_long, 0 as c_long, 0 as c_long);
            let mut argv: Vec<*const c_char> = argv_s.iter().map(|c| c.as_ptr()).collect();
            argv.push(core::ptr::null());
            let mut envp: Vec<*const c_char> = env_s.iter().map(|c| c.as_ptr()).collect();
            envp.push(core::ptr::null());
            execve(loader_c.as_ptr(), argv.as_ptr(), envp.as_ptr());
            _exit(127);
        }
    }
    if pid < 0 {
        crate::log("spy: fork failed");
        return;
    }

    unsafe {
        let mut st: c_int = 0;
        if waitpid(pid, &mut st, 0) < 0 {
            crate::log("spy: no initial stop");
            return;
        }
        ptrace(
            PTRACE_SETOPTIONS,
            pid as c_long,
            0 as c_long,
            PTRACE_O_TRACESYSGOOD | PTRACE_O_EXITKILL,
        );

        let mut ring = [0i64; RING];
        let mut seen = 0usize;
        let mut total = 0u64;
        let mut pending: i64 = -1; // syscall we entered but never saw exit for
        let mut deliver: c_long = 0;

        loop {
            if ptrace(PTRACE_SYSCALL, pid as c_long, 0 as c_long, deliver) < 0 {
                crate::log("spy: resume failed");
                break;
            }
            deliver = 0;
            if waitpid(pid, &mut st, 0) < 0 {
                break;
            }
            if wifexited(st) || wifsignaled(st) {
                crate::log(&format!(
                    "spy: tracee {} after {total} syscalls; last pending entry = {pending}",
                    if wifexited(st) {
                        format!("exit {}", wexitstatus(st))
                    } else {
                        format!("killed by signal {} ({})", wtermsig(st), sig_name(wtermsig(st)))
                    }
                ));
                break;
            }
            let sig = wstopsig(st);
            if sig == (SIGTRAP | 0x80) {
                let mut regs: PtRegs = core::mem::zeroed();
                let mut iov = IoVec {
                    base: &mut regs as *mut _ as *mut c_void,
                    len: core::mem::size_of::<PtRegs>(),
                };
                if ptrace(
                    PTRACE_GETREGSET,
                    pid as c_long,
                    NT_PRSTATUS as c_long,
                    &mut iov as *mut _ as c_long,
                ) == 0
                {
                    let nr = regs.regs[8] as i64;
                    // A successful execve/execveat() has no syscall-exit stop, so
                    // parity flips there. Treat the next stop as an entry again.
                    if pending >= 0 && pending != 221 && pending != 281 {
                        pending = -1;
                        deliver = 0;
                        continue;
                    }
                    if pending < 0 {
                        // an entry stop
                        pending = nr;
                        ring[seen % RING] = nr;
                        seen += 1;
                        total += 1;
                    } else {
                        // an exit stop
                        pending = -1;
                    }
                }
            } else {
                // A real signal stop; forward it untouched on the next resume.
                deliver = sig as c_long;
            }
        }

        let n = core::cmp::min(seen, RING);
        let mut list = String::new();
        for k in 0..n {
            let idx = (seen + RING - n + k) % RING;
            list.push_str(&format!("{} ", ring[idx]));
        }
        crate::log(&format!("spy: last enters (old -> new): {list}"));
        // Drain the pty: the guest's own error messages are the second half of
        // the answer (e.g. which exec got EACCES).
        let mut buf = [0u8; 8192];
        let mut out = std::fs::OpenOptions::new().create(true).truncate(true).write(true)
            .open(format!("{rootfs}/../trace.err")).ok();
        loop {
            let n = read(mfd, buf.as_mut_ptr() as *mut c_void, buf.len());
            if n <= 0 { break; }
            if let Some(f) = out.as_mut() {
                use std::io::Write;
                let _ = f.write_all(&buf[..n as usize]);
            }
        }
        close(mfd);
    }
}
