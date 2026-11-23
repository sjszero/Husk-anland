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

const RING: usize = 64;
const PTRACE_O_EXITKILL: c_long = 0x0010_0000;

/// Find which mapped file an address belongs to: "path+offset".
fn owner_of(pid: i32, addr: u64) -> String {
    let Ok(maps) = std::fs::read_to_string(format!("/proc/{pid}/maps")) else {
        return "?".into();
    };
    for line in maps.lines() {
        let mut it = line.split_whitespace();
        let Some(range) = it.next() else { continue };
        let Some(perms) = it.next() else { continue };
        if !perms.starts_with("r-x") {
            continue;
        }
        let Some((a, b)) = range.split_once('-') else {
            continue;
        };
        let (Ok(lo), Ok(hi)) = (u64::from_str_radix(a, 16), u64::from_str_radix(b, 16)) else {
            continue;
        };
        if addr >= lo && addr < hi {
            // try: perms, offset, dev, inode, path
            let _offset = it.next();
            let path = it.nth(2).unwrap_or("[anon]");
            let foff =
                u64::from_str_radix(line.split_whitespace().nth(2).unwrap_or("0"), 16).unwrap_or(0);
            return format!("{path}+{:#x}", foff + (addr - lo));
        }
    }
    "?".into()
}

pub fn spy(loader: &str, libdir: &str, rootfs: &str) {
    crate::log("spy: start");
    let Ok(loader_c) = CString::new(loader) else {
        return;
    };
    // The guest command comes from `files/trace` (first line, whitespace
    // separated) so the snitch can replay the exact argv that fails without
    // rebuilding the APK. Empty file = the safe default.
    let want = std::fs::read_to_string(format!("{rootfs}/../trace")).unwrap_or_default();
    let words: Vec<String> = if want.split_whitespace().count() == 0 {
        vec!["-c".to_string(), "echo TRACED_OK".to_string()]
    } else {
        want.split_whitespace().map(|s| s.to_string()).collect()
    };
    let mut argv_s: Vec<CString> = vec![
        "libld.so".to_string(),
        "--library-path".to_string(),
        libdir.to_string(),
        format!("{rootfs}/usr/bin/bash"),
    ]
    .iter()
    .filter_map(|s| CString::new(s.as_str()).ok())
    .collect();
    for w in &words {
        if w == "nopty" {
            continue;
        }
        if let Ok(c) = CString::new(w.as_str()) {
            argv_s.push(c);
        }
    }
    crate::log(&format!("spy: argv tail = {words:?}"));
    let env_s: Vec<CString> = [
        format!("NR_ROOTFS={rootfs}"),
        "GLIBC_TUNABLES=glibc.pthread.rseq=0".to_string(),
        "NR_SIGSYS=enosys".to_string(),
        "NR_DEBUG=1".to_string(),
        format!("NR_LD={loader}"),
        format!("NR_SIGSYS_LOG={rootfs}/../sigsys.log"),
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
    // Diagnostic: `files/trace` may contain the word "nopty" to run the guest
    // with no controlling terminal at all. That separates "the domain kills us"
    // from "job control on a PTY kills us".
    let want_nopty = want.contains("nopty");
    let mfd = if want_nopty {
        -1
    } else {
        unsafe { posix_openpt(0o2 | 0o400) }
    }; // O_RDWR | O_NOCTTY
    let mut ptname = [0 as c_char; 128];
    if !want_nopty
        && (mfd < 0
            || unsafe { grantpt(mfd) } != 0
            || unsafe { unlockpt(mfd) } != 0
            || unsafe { ptsname_r(mfd, ptname.as_mut_ptr(), ptname.len()) } != 0)
    {
        crate::log("spy: pty setup failed");
        return;
    }
    let slave = unsafe { CStr::from_ptr(ptname.as_ptr()) }.to_owned();
    let pid = unsafe { fork() };
    if pid == 0 {
        unsafe {
            setsid();
            let sfd = if want_nopty {
                -1
            } else {
                open(slave.as_ptr(), 0o2)
            };
            if sfd >= 0 {
                ioctl(sfd, 0x540E /* TIOCSCTTY */, 0usize);
                dup2(sfd, 0);
                dup2(sfd, 1);
                dup2(sfd, 2);
                if sfd > 2 {
                    close(sfd);
                }
            }
            if let Some(r) = &root_c {
                chdir(r.as_ptr());
            }
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
        let mut fails = 0u32;

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
                let mut regs: PtRegs = core::mem::zeroed();
                let mut iov = IoVec {
                    base: &mut regs as *mut _ as *mut c_void,
                    len: core::mem::size_of::<PtRegs>(),
                };
                let have = ptrace(
                    PTRACE_GETREGSET,
                    pid as c_long,
                    NT_PRSTATUS as c_long,
                    &mut iov as *mut _ as c_long,
                ) == 0;
                if have {
                    crate::log(&format!(
                        "spy: death regs x8={} args=({},{},{}) pc={:#x}",
                        regs.regs[8] as i64,
                        regs.regs[0] as i64,
                        regs.regs[1] as i64,
                        regs.regs[2] as i64,
                        regs.pc
                    ));
                }
                crate::log(&format!(
                    "spy: tracee {} after {total} syscalls; last pending entry = {pending}",
                    if wifexited(st) {
                        format!("exit {}", wexitstatus(st))
                    } else {
                        format!(
                            "killed by signal {} ({})",
                            wtermsig(st),
                            sig_name(wtermsig(st))
                        )
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
                        // syscall-exit stop: this is where x0 carries the result.
                        // (The failure log used to live in the branch below, which
                        // this `continue` made unreachable - hence "0 failures"
                        // for a run that certainly had ENOENTs.)
                        let ret = regs.regs[0] as i64;
                        if ret < 0 && ret > -4096 && fails < 64 {
                            fails += 1;
                            crate::log(&format!(
                                "spy: syscall {pending} -> {ret} at {}",
                                owner_of(pid, regs.pc)
                            ));
                        }
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
                        if total > 100000 {
                            crate::log(&format!("spy: tail nr={nr} at {}", owner_of(pid, regs.pc)));
                        }
                        if nr == 94 {
                            crate::log(&format!(
                                "spy: exit_group caller(lr) = {}",
                                owner_of(pid, regs.regs[30])
                            ));
                            // Crude backtrace: scan the stack for words that land
                            // inside a mapped code page. glibc is built without
                            // frame pointers, so this is all we get.
                            let mut found = 0;
                            for i in 0..128usize {
                                let mut word: c_long = 0;
                                let r = ptrace(
                                    PTRACE_PEEKDATA,
                                    pid as c_long,
                                    (regs.sp as usize + i * 8) as c_long,
                                    &mut word as *mut c_long as c_long,
                                );
                                if r != 0 {
                                    break;
                                }
                                let v = word as u64;
                                if v < 0x10000 {
                                    continue;
                                }
                                let who = owner_of(pid, v);
                                if who == "?" || who.contains("libld.so") {
                                    continue;
                                }
                                crate::log(&format!("spy: bt[{i}] {who}"));
                                found += 1;
                                if found >= 10 {
                                    break;
                                }
                            }
                        }
                        if nr == 129 {
                            // Walk the stack of the stopped tracee and print the
                            // words that land in a mapped file: that is the call
                            // chain, which neither frame pointers nor lr give us.
                            use std::io::{Read, Seek, SeekFrom};
                            if let Ok(mut m) = std::fs::File::open(format!("/proc/{pid}/mem")) {
                                let _ = m.seek(SeekFrom::Start(regs.sp));
                                let mut buf = [0u8; 512];
                                if m.read(&mut buf).is_ok() {
                                    let mut shown = 0;
                                    for i in 0..64usize {
                                        let v = u64::from_le_bytes(
                                            buf[i * 8..i * 8 + 8].try_into().unwrap(),
                                        );
                                        if v < 0x10000 {
                                            continue;
                                        }
                                        let who = owner_of(pid, v);
                                        if who == "?" || who.contains("libld.so") {
                                            continue;
                                        }
                                        crate::log(&format!("spy: st[{i}] {who}"));
                                        shown += 1;
                                        if shown >= 6 {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if nr == 129 || nr == 131 || nr == 130 || nr == 94 || nr == 231 {
                            crate::log(&format!(
                                "spy: entry nr={nr} args=({},{},{}) call_site={} lr={}",
                                regs.regs[0] as i64,
                                regs.regs[1] as i64,
                                regs.regs[2] as i64,
                                owner_of(pid, regs.pc),
                                owner_of(pid, regs.regs[30])
                            ));
                        }
                    } else {
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
        crate::log(&format!("spy: failed syscalls logged = {fails}"));
        // Drain the pty: the guest's own error messages are the second half of
        // the answer (e.g. which exec got EACCES).
        let mut buf = [0u8; 8192];
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(format!("{rootfs}/../trace.err"))
            .ok();
        loop {
            let n = read(mfd, buf.as_mut_ptr() as *mut c_void, buf.len());
            if n <= 0 {
                break;
            }
            if let Some(f) = out.as_mut() {
                use std::io::Write;
                let _ = f.write_all(&buf[..n as usize]);
            }
        }
        close(mfd);
    }
}
