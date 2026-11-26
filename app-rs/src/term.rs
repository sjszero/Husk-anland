//! The PTY and the screen buffer.  One fork, one execve, no interpreter in the
//! middle: the child is the guest loader, so bash runs as a plain Android process.

use crate::sys::*;
use core::ffi::c_int;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};

/// Filled block, drawn as the cursor.  Not a font glyph: the atlas gets it for
/// free so the renderer has one draw path for everything on screen.
pub const BLOCK: u8 = 127;

/// How many scrolled-off lines we keep for scrollback.
const HISTORY: usize = 2000;

/// Wire format sent once by the app to the session guard. The guard is started
/// before the guest so there is no window in which App death leaves no reader
/// for the lease; the leader id is filled in immediately after fork().
#[repr(C)]
#[derive(Clone, Copy)]
struct GuardArm {
    magic: u32,
    version: u32,
    leader_pid: i32,
    _reserved: u32,
    leader_start_ticks: u64,
}

const GUARD_MAGIC: u32 = 0x4853_4b47; // "HSKG"
const GUARD_VERSION: u32 = 1;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

fn gpu_software_requested() -> bool {
    std::env::var("HUSK_GPU")
        .map(|v| v == "llvmpipe")
        .unwrap_or(false)
        || std::env::var("NR_GPU_DEFAULT_OFF")
            .map(|v| {
                !matches!(
                    v.trim(),
                    "" | "0" | "false" | "FALSE" | "False" | "no" | "NO" | "off" | "OFF"
                )
            })
            .unwrap_or(false)
}

fn mali_device_available() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/mali0")
        .is_ok()
}

fn adreno_device_available() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kgsl-3d0")
        .is_ok()
}

fn gpu_env(rootfs: &str) -> Vec<String> {
    gpu_env::build(
        std::path::Path::new(rootfs),
        mali_device_available(),
        adreno_device_available(),
        gpu_software_requested(),
    )
}

fn session_locale(rootfs: &str) -> (String, String) {
    let has_zh_locale = std::path::Path::new(rootfs)
        .join("usr/lib/locale/locale-archive")
        .is_file()
        || std::path::Path::new(rootfs)
            .join("usr/lib/locale/zh_CN.utf8")
            .is_dir();
    session_env::choose_session_locale(
        std::env::var("LANG").ok().as_deref(),
        std::env::var("LANGUAGE").ok().as_deref(),
        has_zh_locale,
    )
}

pub struct Term {
    pub fd: i32,
    pub pid: i32,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<u8>,
    pub cx: usize,
    pub cy: usize,
    pub dead: bool,
    /// Raw mirror of everything the guest wrote, for debugging (fd, -1 = off).
    pub tee: i32,
    /// Human-readable end of session ("exit 7" / "signal 31 (SIGSYS)").
    pub exit_text: String,
    /// Field 22 of /proc/<pid>/stat taken right after fork. The session leader's
    /// pid can be recycled; comparing start ticks is the only cheap way to tell
    /// "our group is still there" from "an unrelated process now owns this pid"
    /// before signalling -pid.
    pub start_ticks: u64,
    /// Write end of the session lease. The guard sees EOF when the App process
    /// dies, even for `am force-stop` where no Android callback is delivered.
    guard_fd: i32,
    guard_pid: i32,
    /// Private marker inherited by every process in this guest session. The
    /// guard uses it to find daemons that called setsid() and escaped the
    /// session leader's process group.
    session_id: String,
    history: VecDeque<Vec<u8>>,
    esc: u8,
    /// CSI with a private marker (`ESC [ ? ... h/l`): the alternate screen that
    /// full-screen programs like `top` and `less` switch to.
    private: bool,
    /// Cells and cursor saved by the alternate screen, restored on exit.
    alt: Option<(Vec<u8>, usize, usize)>,
    params: Vec<u32>,
    cur: u32,
    has_cur: bool,
}

/// Start the tiny exec'd Rust guard that survives the App and owns the other
/// end of the session lease. Only async-signal-safe calls happen after fork();
/// every allocation and string is prepared first.
fn spawn_guard(libdir: &str, rootfs: &str, session_id: &str) -> Option<(i32, i32)> {
    let guard_path = format!("{libdir}/libhuskguard.so");
    if !std::path::Path::new(&guard_path).exists() {
        crate::log(&format!(
            "session: guard missing at {guard_path}; refusing leak-prone session"
        ));
        return None;
    }
    let log_path = format!("{rootfs}/../session-guard.log");
    let guard_c = CString::new(guard_path).ok()?;
    let lease_fd_c = CString::new("3").ok()?;
    let ready_fd_c = CString::new("4").ok()?;
    let session_c = CString::new(session_id).ok()?;
    let log_c = CString::new(log_path).ok()?;
    let argv_s: Vec<CString> = vec![
        CString::new("husk-session-guard").ok()?,
        CString::new("--lease-fd").ok()?,
        lease_fd_c,
        CString::new("--ready-fd").ok()?,
        ready_fd_c,
        CString::new("--session-id").ok()?,
        session_c,
        CString::new("--log").ok()?,
        log_c,
    ];
    let env_s: Vec<CString> = vec![
        CString::new("PATH=/system/bin").ok()?,
        CString::new("ANDROID_ROOT=/system").ok()?,
        CString::new("ANDROID_DATA=/data").ok()?,
    ];
    let mut lease = [0 as c_int; 2];
    let mut ready = [0 as c_int; 2];
    if unsafe { pipe(lease.as_mut_ptr()) } != 0 {
        crate::log("session: guard pipe() failed");
        return None;
    }
    if unsafe { pipe(ready.as_mut_ptr()) } != 0 {
        unsafe {
            close(lease[0]);
            close(lease[1]);
        }
        crate::log("session: guard ready pipe() failed");
        return None;
    }
    let guard_pid = unsafe { fork() };
    if guard_pid < 0 {
        unsafe {
            close(lease[0]);
            close(lease[1]);
            close(ready[0]);
            close(ready[1]);
        }
        crate::log("session: guard fork() failed");
        return None;
    }
    if guard_pid == 0 {
        unsafe {
            setsid();
            close(ready[0]);
            if lease[0] != 3 && dup2(lease[0], 3) < 0 {
                _exit(127);
            }
            if ready[1] != 4 && dup2(ready[1], 4) < 0 {
                _exit(127);
            }
            let mut fd = 0;
            while fd < 1024 {
                if fd != 3 && fd != 4 {
                    close(fd);
                }
                fd += 1;
            }
            let mut argv: Vec<*const core::ffi::c_char> =
                argv_s.iter().map(|c| c.as_ptr()).collect();
            argv.push(core::ptr::null());
            let mut envp: Vec<*const core::ffi::c_char> =
                env_s.iter().map(|c| c.as_ptr()).collect();
            envp.push(core::ptr::null());
            execve(guard_c.as_ptr(), argv.as_ptr(), envp.as_ptr());
            _exit(127);
        }
    }
    unsafe {
        close(lease[0]);
        close(ready[1]);
        fcntl(ready[0], F_SETFL, O_NONBLOCK);
    }
    let mut ready_byte = 0u8;
    let mut is_ready = false;
    for _ in 0..100 {
        let n = unsafe { read(ready[0], &mut ready_byte as *mut u8 as *mut _, 1) };
        if n == 1 && ready_byte == b'R' {
            is_ready = true;
            break;
        }
        if n == 0 {
            break;
        }
        let errno = unsafe { *__errno() };
        if errno != EAGAIN && errno != EINTR {
            break;
        }
        unsafe { usleep(10_000) };
    }
    unsafe { close(ready[0]) };
    if !is_ready {
        unsafe { close(lease[1]) };
        crate::log(&format!("session: guard pid={guard_pid} failed to exec"));
        return None;
    }
    crate::log(&format!("session: guard pid={guard_pid} ready"));
    Some((guard_pid, lease[1]))
}

impl Term {
    pub fn blank(cols: usize, rows: usize) -> Term {
        Term {
            fd: -1,
            pid: 0,
            cols,
            rows,
            cells: vec![b' '; cols * rows],
            cx: 0,
            cy: 0,
            dead: true,
            tee: -1,
            exit_text: String::new(),
            start_ticks: 0,
            guard_fd: -1,
            guard_pid: 0,
            session_id: String::new(),
            history: VecDeque::with_capacity(HISTORY),
            esc: 0,
            private: false,
            alt: None,
            params: Vec::new(),
            cur: 0,
            has_cur: false,
        }
    }

    pub fn spawn(
        loader: &str,
        libdir: &str,
        rootfs: &str,
        cols: usize,
        rows: usize,
    ) -> Option<Term> {
        let mfd = unsafe { posix_openpt(O_RDWR | O_NOCTTY) };
        if mfd < 0 {
            return None;
        }
        unsafe {
            if grantpt(mfd) != 0 || unlockpt(mfd) != 0 {
                close(mfd);
                return None;
            }
        }
        let mut name = [0 as core::ffi::c_char; 128];
        if unsafe { ptsname_r(mfd, name.as_mut_ptr(), name.len()) } != 0 {
            unsafe { close(mfd) };
            return None;
        }

        provision_identity(rootfs);

        let session_id = format!(
            "husk-{}-{}",
            unsafe { getpid() },
            NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
        );

        // Everything the child needs is built before fork(): no allocation and no
        // logging between fork and execve.
        let slave = unsafe { CStr::from_ptr(name.as_ptr()) }.to_owned();
        let loader_c = CString::new(loader).ok()?;
        let bash_c = CString::new(format!("{rootfs}/usr/bin/bash")).ok()?;
        let root_c = CString::new(rootfs).ok()?;

        let argv_s: Vec<CString> = vec![
            CString::new("libld.so").ok()?,
            CString::new("--library-path").ok()?,
            CString::new(libdir).ok()?,
            bash_c.clone(),
            CString::new("-i").ok()?,
        ];
        let (session_lang, session_language) = session_locale(rootfs);
        let mut env = vec![
            format!("NR_ROOTFS={rootfs}"),
            "GLIBC_TUNABLES=glibc.pthread.rseq=0".to_string(),
            "TERM=xterm-256color".to_string(),
            "SHELL=/usr/bin/bash".to_string(),
            format!("HOME={rootfs}/root"),
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            format!("LANG={session_lang}"),
            format!("LANGUAGE={session_language}"),
            "USER=root".to_string(),
            format!("NR_SESSION_ID={session_id}"),
            // diagnostics for the seccomp policy of the ROM under test
            "NR_SIGSYS=enosys".to_string(),
            format!("NR_LD={loader}"),
            format!("NR_SIGSYS_LOG={rootfs}/../sigsys.log"),
        ];
        env.extend(gpu_env(rootfs));
        let env_s: Vec<CString> = env
            .iter()
            .filter_map(|s| CString::new(s.as_str()).ok())
            .collect();
        // fork() duplicates a multi-threaded process. Only async-signal-safe
        // calls are legal until execve(), so the argv/envp pointer arrays must
        // be allocated here, before the fork. Building them in the child could
        // deadlock on the allocator lock held by another App thread.
        let mut argv_ptrs: Vec<*const core::ffi::c_char> =
            argv_s.iter().map(|c| c.as_ptr()).collect();
        argv_ptrs.push(core::ptr::null());
        let mut env_ptrs: Vec<*const core::ffi::c_char> =
            env_s.iter().map(|c| c.as_ptr()).collect();
        env_ptrs.push(core::ptr::null());

        // The lease belongs to the App, not to the guest. Starting the guard
        // before the guest closes the only bad ordering: App death after fork()
        // but before a guard exists would again leave setsid() daemons behind.
        let Some(native_libdir) = crate::native_lib_dir() else {
            unsafe { close(mfd) };
            return None;
        };
        let Some((guard_pid, guard_fd)) = spawn_guard(&native_libdir, rootfs, &session_id) else {
            unsafe { close(mfd) };
            return None;
        };

        let pid = unsafe { fork() };
        if pid < 0 {
            unsafe {
                close(guard_fd);
                close(mfd);
            }
            return None;
        }
        if pid == 0 {
            // Between fork() and execve() only async-signal-safe calls are legal:
            // write(2) is, log()/format!() are not. So the failure paths below
            // write a fixed byte string straight to fd 2 instead of logging.
            const SETSID_FAILED: &[u8] = b"husk: setsid failed\n";
            unsafe {
                // The parent and the guard keep their own lease descriptors.
                close(guard_fd);
                if setsid() < 0 {
                    write(2, SETSID_FAILED.as_ptr() as *const _, SETSID_FAILED.len());
                    _exit(126);
                }
                // A child of a multi-threaded process inherits the parent's signal
                // mask. glibc's loader wants a clean one; anything blocked here
                // would silently change the guest's semantics.
                let empty = [0u64; 2];
                sigprocmask(
                    SIG_SETMASK,
                    empty.as_ptr() as *const _,
                    core::ptr::null_mut(),
                );
                let sfd = open(slave.as_ptr(), O_RDWR);
                if sfd >= 0 {
                    ioctl(sfd, TIOCSCTTY, 0usize);
                    dup2(sfd, 0);
                    dup2(sfd, 1);
                    dup2(sfd, 2);
                    if sfd > 2 {
                        close(sfd);
                    }
                }
                close(mfd);
                // Nothing above fd 2 belongs to the guest. Leaving EGL/looper/log
                // fds open would hand the guest descriptors it can neither name
                // nor close, and would keep surfaces alive past teardown.
                let mut fd = 3;
                while fd < 1024 {
                    close(fd);
                    fd += 1;
                }
                let _ = chdir(root_c.as_ptr());
                execve(loader_c.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                _exit(127);
            }
        }
        unsafe {
            fcntl(mfd, F_SETFL, O_NONBLOCK);
        }
        let leader_start_ticks = start_ticks(pid);
        let arm = GuardArm {
            magic: GUARD_MAGIC,
            version: GUARD_VERSION,
            leader_pid: pid,
            _reserved: 0,
            leader_start_ticks,
        };
        let armed = unsafe {
            write(
                guard_fd,
                &arm as *const GuardArm as *const _,
                core::mem::size_of::<GuardArm>(),
            )
        } == core::mem::size_of::<GuardArm>() as isize;
        let tee = {
            let p = CString::new(format!("{rootfs}/../pty.log")).ok();
            match p {
                Some(p) => unsafe { open(p.as_ptr(), 0o1 | 0o100 | 0o1000, 0o644u32) },
                None => -1,
            }
        };
        crate::log(&format!(
            "spawned pid={pid} fd={mfd} tee={tee} guard_pid={guard_pid} guard_fd={guard_fd} armed={armed}"
        ));
        let mut t = Term::blank(cols, rows);
        t.start_ticks = leader_start_ticks;
        t.fd = mfd;
        t.tee = tee;
        t.pid = pid;
        t.dead = false;
        t.guard_fd = guard_fd;
        t.guard_pid = guard_pid;
        t.session_id = session_id;
        t.resize(cols, rows);
        Some(t)
    }

    /// Keeps the top-left corner of what was already on screen: a rotation or an
    /// IME show/hide should not wipe the session.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        let (oc, orows) = (self.cols, self.rows);
        if (cols, rows) != (oc, orows) && self.pid > 0 {
            crate::log(&format!(
                "session: resize {oc}x{orows} -> {cols}x{rows} (TIOCSWINSZ, pid={})",
                self.pid
            ));
        }
        let mut cells = vec![b' '; cols * rows];
        for r in 0..rows.min(orows) {
            for c in 0..cols.min(oc) {
                cells[r * cols + c] = self.cells[r * oc + c];
            }
        }
        self.cells = cells;
        self.cols = cols;
        self.rows = rows;
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
        if self.fd >= 0 {
            let ws = Winsize {
                rows: rows as u16,
                cols: cols as u16,
                xpixel: 0,
                ypixel: 0,
            };
            unsafe {
                ioctl(self.fd, TIOCSWINSZ, &ws as *const Winsize);
            }
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.fd < 0 {
            return;
        }
        unsafe {
            write(self.fd, bytes.as_ptr() as *const _, bytes.len());
        }
    }

    pub fn read_available(&mut self) -> bool {
        // Once the master has seen EIO/EOF it stays dead: reading it again just
        // spams EIO for every frame. Close it and let the reaper finish the story.
        if self.fd < 0 || self.dead {
            return false;
        }
        let mut buf = [0u8; 8192];
        let mut got = false;
        loop {
            let n = unsafe { read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n > 0 {
                got = true;
                if self.tee >= 0 {
                    unsafe { write(self.tee, buf.as_ptr() as *const _, n as usize) };
                }
                self.feed(&buf[..n as usize]);
                continue;
            }
            if n == 0 {
                self.dead = true;
            } else {
                let e = unsafe { *__errno() };
                // EIO/EOF on a pty master means every slave is closed: the child
                // exited (or exec'd away from us). waitpid belongs to the reaper
                // thread alone -- a second waiter here is exactly what leaked
                // 700 zombies on 2026-11-22.
                if e != EAGAIN && e != EINTR {
                    crate::log(&format!("pty read errno={e}; session closed"));
                    self.dead = true;
                    self.close_master();
                }
            }
            break;
        }
        got
    }

    /// Close the PTY master the only correct way: deregister from the looper,
    /// then close (lifecycle L7).
    ///
    /// A closed fd that is still registered with the looper is worse than a leak:
    /// the number is reused, and the looper then reports readiness for whatever
    /// took it, under this session's identity.
    pub fn close_master(&mut self) {
        if self.fd >= 0 {
            let l = crate::LOOPER.load(core::sync::atomic::Ordering::SeqCst);
            if !l.is_null() {
                unsafe { ALooper_removeFd(l, self.fd) };
            }
            unsafe { close(self.fd) };
            self.fd = -1;
            if self.tee >= 0 {
                unsafe { close(self.tee) };
                self.tee = -1;
            }
        }
        self.close_guard();
    }

    /// Closing the lease is the out-of-band teardown channel: the guard gets
    /// EOF even when Android force-stops us before any callback. It then kills
    /// every process carrying this session's private marker, including daemons
    /// that escaped the leader's process group with setsid().
    fn close_guard(&mut self) {
        if self.guard_fd < 0 {
            return;
        }
        unsafe { close(self.guard_fd) };
        self.guard_fd = -1;
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Give scrollback memory back. Called from onLowMemory; the visible grid and
    /// the live PTY are untouched, so the shell does not notice.
    pub fn trim_history(&mut self, keep: usize) {
        while self.history.len() > keep {
            self.history.pop_front();
        }
    }

    /// The reaper thread saw this session's pid exit. Records the real reason and
    /// prints it on screen, so a dead guest explains itself instead of blinking.
    pub fn note_exit(&mut self, status: i32) {
        let text = if wifexited(status) {
            format!("exit {}", wexitstatus(status))
        } else if wifsignaled(status) {
            let sig = wtermsig(status);
            format!("signal {sig} ({})", sig_name(sig))
        } else {
            format!("status {status}")
        };
        crate::log(&format!("session pid={} ended: {text}", self.pid));
        self.exit_text = text.clone();
        self.dead = true;
        self.close_guard();
        self.banner(&format!(
            "\r\n[husk] session ended: {text}\r\n[husk] tap to start again\r\n"
        ));
    }

    pub fn banner(&mut self, text: &str) {
        for b in text.bytes() {
            self.feed(&[b]);
        }
    }

    fn put(&mut self, c: u8) {
        if self.cx >= self.cols {
            self.cx = 0;
            self.newline();
        }
        let i = self.cy * self.cols + self.cx;
        if i < self.cells.len() {
            self.cells[i] = c;
        }
        self.cx += 1;
    }

    fn newline(&mut self) {
        self.cy += 1;
        if self.cy >= self.rows {
            let (cols, rows) = (self.cols, self.rows);
            // the line about to scroll off becomes scrollback
            let top: Vec<u8> = self.cells[..cols].to_vec();
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(top);
            self.cells.copy_within(cols.., 0);
            for i in 0..cols {
                self.cells[(rows - 1) * cols + i] = b' ';
            }
            self.cy = rows - 1;
        }
    }

    fn clear_screen(&mut self) {
        for c in self.cells.iter_mut() {
            *c = b' ';
        }
        self.cx = 0;
        self.cy = 0;
    }

    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            match self.esc {
                0 => match b {
                    0x1b => self.esc = 1,
                    0x0d => self.cx = 0,
                    0x0a => self.newline(),
                    0x08 => {
                        if self.cx > 0 {
                            self.cx -= 1;
                        }
                    }
                    0x09 => {
                        let next = (self.cx / 8 + 1) * 8;
                        while self.cx < next.min(self.cols) {
                            self.put(b' ');
                        }
                    }
                    0x07 => {}
                    0x00..=0x1f => {}
                    0x7f => {}
                    _ => self.put(b),
                },
                1 => match b {
                    b'[' => {
                        self.esc = 2;
                        self.params.clear();
                        self.private = false;
                        self.cur = 0;
                        self.has_cur = false;
                    }
                    b']' => self.esc = 3,
                    // ESC ( B / ESC ) 0 / ESC * ... : designate character set.
                    // Three bytes, and the last one is a printable letter that
                    // must not reach the screen -- `top` sends ESC ( B on every
                    // full repaint, which is where the stray "B" came from.
                    b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.esc = 4,
                    // ESC 7/8 (save/restore cursor), ESC = / > (keypad), ESC c
                    // (reset): complete sequences with no operands.
                    b'7' | b'8' | b'=' | b'>' | b'c' | b'M' => self.esc = 0,
                    // ESC followed by anything else is a two-byte sequence.
                    _ => self.esc = 0,
                },
                3 => {
                    // OSC: terminated by BEL or by ESC \ (string terminator).
                    if b == 0x07 {
                        self.esc = 0;
                    } else if b == 0x1b {
                        self.esc = 5;
                    }
                }
                4 => self.esc = 0,
                5 => self.esc = if b == b'\\' { 0 } else { 3 },
                2 => match b {
                    b'?' => self.private = true,
                    b'0'..=b'9' => {
                        self.cur = self
                            .cur
                            .saturating_mul(10)
                            .saturating_add((b - b'0') as u32);
                        self.has_cur = true;
                    }
                    b';' => {
                        self.params.push(if self.has_cur { self.cur } else { 0 });
                        self.cur = 0;
                        self.has_cur = false;
                    }
                    0x40..=0x7e => {
                        if self.has_cur {
                            self.params.push(self.cur);
                        }
                        self.csi(b);
                        self.esc = 0;
                        self.cur = 0;
                        self.has_cur = false;
                    }
                    _ => {}
                },
                _ => self.esc = 0,
            }
        }
    }

    fn p(&self, i: usize, dflt: u32) -> u32 {
        match self.params.get(i) {
            Some(0) | None => dflt,
            Some(v) => *v,
        }
    }

    fn csi(&mut self, final_byte: u8) {
        // Alternate screen: top/less/vim switch to it and expect their own
        // drawing to disappear when they exit. Without this the program's screen
        // stays behind and the shell prompt lands on the last row.
        if self.private && (final_byte == b'h' || final_byte == b'l') {
            let mode = self.params.first().copied().unwrap_or(0);
            if mode == 1049 || mode == 1047 || mode == 47 {
                if final_byte == b'h' {
                    if self.alt.is_none() {
                        self.alt = Some((self.cells.clone(), self.cx, self.cy));
                        self.clear_screen();
                        self.cx = 0;
                        self.cy = 0;
                    }
                } else if let Some((cells, cx, cy)) = self.alt.take() {
                    if cells.len() == self.cells.len() {
                        self.cells = cells;
                        self.cx = cx.min(self.cols - 1);
                        self.cy = cy.min(self.rows - 1);
                    } else {
                        self.clear_screen();
                    }
                }
            }
            return;
        }
        match final_byte {
            b'H' | b'f' => {
                let row = self.p(0, 1) as usize;
                let col = self.p(1, 1) as usize;
                self.cy = row.saturating_sub(1).min(self.rows - 1);
                self.cx = col.saturating_sub(1).min(self.cols - 1);
            }
            b'A' => self.cy = self.cy.saturating_sub(self.p(0, 1) as usize),
            b'B' => self.cy = (self.cy + self.p(0, 1) as usize).min(self.rows - 1),
            b'C' => self.cx = (self.cx + self.p(0, 1) as usize).min(self.cols - 1),
            b'D' => self.cx = self.cx.saturating_sub(self.p(0, 1) as usize),
            b'J' => {
                let mode = self.p(0, 0);
                if mode == 2 || mode == 3 {
                    self.clear_screen();
                } else if mode == 0 {
                    let i = self.cy * self.cols + self.cx;
                    for c in self.cells.iter_mut().skip(i) {
                        *c = b' ';
                    }
                }
            }
            b'K' => {
                let mode = self.p(0, 0);
                let start = self.cy * self.cols;
                match mode {
                    1 => {
                        for c in self.cells.iter_mut().take(start + self.cx + 1) {
                            *c = b' ';
                        }
                    }
                    2 => {
                        for c in self.cells.iter_mut().skip(start).take(self.cols) {
                            *c = b' ';
                        }
                    }
                    _ => {
                        let i = start + self.cx;
                        for c in self.cells.iter_mut().skip(i) {
                            *c = b' ';
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Rows for the current view: `back` lines of scrollback above the grid.
    pub fn snapshot(&self, back: usize) -> Vec<u8> {
        let back = back.min(self.history.len());
        if back == 0 {
            return self.cells.clone();
        }
        let (cols, rows) = (self.cols, self.rows);
        let mut out = vec![b' '; cols * rows];
        let from_hist = back.min(rows);
        let mut r = 0;
        for line in self.history.iter().skip(self.history.len() - from_hist) {
            out[r * cols..r * cols + cols].copy_from_slice(&line[..cols]);
            r += 1;
        }
        for g in 0..(rows - from_hist) {
            let src = g * cols;
            out[r * cols..r * cols + cols].copy_from_slice(&self.cells[src..src + cols]);
            r += 1;
        }
        out
    }

    pub fn max_scroll(&self) -> usize {
        self.history.len().min(self.rows.saturating_sub(1))
    }

    #[cfg(test)]
    fn row(&self, r: usize) -> String {
        let s: Vec<u8> = self.cells[r * self.cols..(r + 1) * self.cols].to_vec();
        String::from_utf8_lossy(&s).trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::current_dir()
            .unwrap()
            .join(format!("husk-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn writes_and_wraps() {
        let mut t = Term::blank(4, 2);
        t.feed(b"abcdef");
        assert_eq!(t.row(0), "abcd");
        assert_eq!(t.row(1), "ef");
    }

    #[test]
    fn carriage_return_overwrites() {
        let mut t = Term::blank(8, 2);
        t.feed(b"abc\rXY");
        assert_eq!(t.row(0), "XYc");
    }

    #[test]
    fn newline_scrolls() {
        let mut t = Term::blank(4, 2);
        t.feed(b"one\r\ntwo\r\nthree");
        // LF keeps the column (like a real terminal); output uses CRLF.
        // "three" is 5 cells in a 4-wide screen: the fifth wraps and scrolls again.
        assert_eq!(t.row(0), "thre");
        assert_eq!(t.row(1), "e");
    }

    #[test]
    fn csi_cursor_position() {
        let mut t = Term::blank(6, 3);
        t.feed(b"\x1b[2;3HX");
        assert_eq!(t.row(1), "  X");
        assert_eq!(t.cy, 1);
        assert_eq!(t.cx, 3);
    }

    #[test]
    fn csi_erase_to_end_of_line() {
        let mut t = Term::blank(6, 1);
        t.feed(b"abcdef\x1b[1;3H\x1b[K");
        assert_eq!(t.row(0), "ab");
    }

    #[test]
    fn csi_clear_screen() {
        let mut t = Term::blank(4, 2);
        t.feed(b"aa\nbb\x1b[2J");
        assert_eq!(t.row(0), "");
        assert_eq!(t.row(1), "");
        assert_eq!((t.cx, t.cy), (0, 0));
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = Term::blank(6, 2);
        t.feed(b"hello");
        t.resize(4, 2);
        assert_eq!(t.cols, 4);
        assert_eq!(t.row(0), "hell");
    }

    #[test]
    fn ignores_osc_and_colour() {
        let mut t = Term::blank(8, 1);
        t.feed(b"\x1b]0;title\x07a\x1b[38;5;9mb");
        assert_eq!(t.row(0), "ab");
    }

    #[test]
    fn scrollback_holds_lines_that_left_the_screen() {
        let mut t = Term::blank(4, 2);
        t.feed(b"a\r\nb\r\nc\r\n");
        // "a" and "b" scrolled off; the grid shows "c" and an empty line.
        assert_eq!(t.row(0), "c");
        assert!(t.max_scroll() >= 1);
        let view = t.snapshot(2);
        assert_eq!(String::from_utf8_lossy(&view[0..4]).trim_end(), "a");
        assert_eq!(String::from_utf8_lossy(&view[4..8]).trim_end(), "b");
    }

    #[test]
    fn snapshot_is_the_grid_when_not_scrolled() {
        let mut t = Term::blank(4, 2);
        t.feed(b"xy");
        assert_eq!(t.snapshot(0), t.cells);
    }

    #[test]
    fn tab_moves_to_next_stop() {
        let mut t = Term::blank(16, 1);
        t.feed(b"a\tb");
        assert_eq!(t.row(0), "a       b");
    }
}

// --------------------------------------------------------- provisioning ---

/// Mirror the Android process identity into the rootfs' passwd/group files.
///
/// Inside the guest we *are* uid 10302 (`u0_a302`) plus a fistful of Android
/// supplementary groups. glibc's getpwuid/getgrgid only know what /etc/passwd
/// and /etc/group say, so without this bash prints `I have no name!` and
/// `groups` prints `cannot find name for group ID 20302`. The numbers differ per
/// install and per device, so the entries are generated here on every start and
/// only appended when missing. Nothing else in the image is touched.
fn provision_identity(rootfs: &str) {
    let uid = unsafe { getuid() };
    let gid = unsafe { getgid() };
    let mut gids = [0u32; 32];
    let n = unsafe { getgroups(gids.len() as c_int, gids.as_mut_ptr()) };
    let extra: Vec<u32> = if n > 0 {
        gids[..(n as usize).min(gids.len())].to_vec()
    } else {
        Vec::new()
    };

    // /etc/passwd: name the uid, give it a home and a shell.
    let passwd = format!("{rootfs}/etc/passwd");
    let mut add = String::new();
    if !has_id(&passwd, 2, uid) {
        add.push_str(&format!("husk:x:{uid}:{gid}:Husk:/:/bin/bash\n"));
    }
    if !add.is_empty() {
        let _ = append(&passwd, &add);
    }

    // /etc/group: name the gid and every supplementary group the Android
    // process carries, otherwise every `groups`/`ls -l` shows a raw number.
    let group = format!("{rootfs}/etc/group");
    let mut add = String::new();
    let mut all = extra.clone();
    if !all.contains(&gid) {
        all.push(gid);
    }
    for g in all {
        if !has_id(&group, 2, g) {
            add.push_str(&format!("husk{g}:x:{g}:\n"));
        }
    }
    if !add.is_empty() {
        let _ = append(&group, &add);
    }

    provision_prompt(rootfs);
}

/// Android's gethostname() is `localhost` for every app, so the distro prompt
/// (`\u@\h:\w\$`) would name every container `localhost`. `\h` reads the
/// kernel, not `$HOSTNAME`, and changing the hostname needs CAP_SYS_ADMIN -
/// so the prompt itself is pinned once, with a marker so it is not appended
/// twice.
fn provision_prompt(rootfs: &str) {
    const MARK: &str = "# husk: stable prompt";
    let p = format!("{rootfs}/etc/bash.bashrc");
    let existing = std::fs::read_to_string(&p).unwrap_or_default();
    if existing.contains(MARK) {
        return;
    }
    let _ = append(
        &p,
        "\n# husk: stable prompt\n# \\h is gethostname(), which Android pins to 'localhost' for apps.\n# \\u resolves once provision_identity() has written our uid into /etc/passwd.\nPS1='\\u@husk:\\w\\$ '\n",
    );
}

/// True when `path` has a line whose `idx`-th (0-based) colon field equals `id`.
fn has_id(path: &str, idx: usize, id: u32) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let want = id.to_string();
    text.lines()
        .any(|l| l.split(':').nth(idx).map(|f| f == want).unwrap_or(false))
}

fn append(path: &str, extra: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    f.write_all(extra.as_bytes())
}

/// Field 22 (starttime, in clock ticks since boot) of /proc/<pid>/stat.
/// 0 means "could not read it", which callers must treat as "unverified".
pub fn start_ticks(pid: i32) -> u64 {
    let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0;
    };
    let Some(i) = s.rfind(')') else { return 0 };
    // After the comm field the fields start again at #3 (state), so #22 is #19.
    s[i + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|t| t.parse().ok())
        .unwrap_or(0)
}

/// Dropping a Term must hang up its PTY, not just forget the numbers.
///
/// Found on device 2026-11-22: the reaper reports the child's exit and the shell
/// is dropped before the master has seen EIO, which left both the ptmx fd and
/// the pty.log tee behind on every restart (fds 98 -> 154 -> 159 over two
/// sessions). close_master() deregisters from the looper first, so this is also
/// what stops the "phantom input" class of bug.
impl Drop for Term {
    fn drop(&mut self) {
        self.close_master();
    }
}
