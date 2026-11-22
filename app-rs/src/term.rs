//! The PTY and the screen buffer.  One fork, one execve, no interpreter in the
//! middle: the child is the guest loader, so bash runs as a plain Android process.

use crate::sys::*;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};

/// Filled block, drawn as the cursor.  Not a font glyph: the atlas gets it for
/// free so the renderer has one draw path for everything on screen.
pub const BLOCK: u8 = 127;

/// How many scrolled-off lines we keep for scrollback.
const HISTORY: usize = 2000;

pub struct Term {
    pub fd: i32,
    pub pid: i32,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<u8>,
    pub cx: usize,
    pub cy: usize,
    pub dead: bool,
    /// Human-readable end of session ("exit 7" / "signal 31 (SIGSYS)").
    pub exit_text: String,
    history: VecDeque<Vec<u8>>,
    esc: u8,
    params: Vec<u32>,
    cur: u32,
    has_cur: bool,
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
            exit_text: String::new(),
            history: VecDeque::with_capacity(HISTORY),
            esc: 0,
            params: Vec::new(),
            cur: 0,
            has_cur: false,
        }
    }

    pub fn spawn(loader: &str, libdir: &str, rootfs: &str, cols: usize, rows: usize) -> Option<Term> {
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
        let env_s: Vec<CString> = [
            format!("NR_ROOTFS={rootfs}"),
            "GLIBC_TUNABLES=glibc.pthread.rseq=0".to_string(),
            "TERM=xterm-256color".to_string(),
            "SHELL=/usr/bin/bash".to_string(),
            format!("HOME={rootfs}/root"),
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "LANG=C.UTF-8".to_string(),
            "USER=root".to_string(),
            // diagnostics for the seccomp policy of the ROM under test
            "NR_SIGSYS=log".to_string(),
            format!("NR_SIGSYS_LOG={rootfs}/../sigsys.log"),
        ]
        .iter()
        .filter_map(|s| CString::new(s.as_str()).ok())
        .collect();

        let pid = unsafe { fork() };
        if pid < 0 {
            unsafe { close(mfd) };
            return None;
        }
        if pid == 0 {
            unsafe {
                setsid();
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
                let _ = chdir(root_c.as_ptr());
                let mut argv: Vec<*const core::ffi::c_char> = argv_s.iter().map(|c| c.as_ptr()).collect();
                argv.push(core::ptr::null());
                let mut envp: Vec<*const core::ffi::c_char> = env_s.iter().map(|c| c.as_ptr()).collect();
                envp.push(core::ptr::null());
                execve(loader_c.as_ptr(), argv.as_ptr(), envp.as_ptr());
                _exit(127);
            }
        }
        unsafe {
            fcntl(mfd, F_SETFL, O_NONBLOCK);
        }
        crate::log(&format!("spawned pid={pid} fd={mfd}"));
        let mut t = Term::blank(cols, rows);
        t.fd = mfd;
        t.pid = pid;
        t.dead = false;
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
            let ws = Winsize { rows: rows as u16, cols: cols as u16, xpixel: 0, ypixel: 0 };
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
                    unsafe { close(self.fd) };
                    self.fd = -1;
                }
            }
            break;
        }
        got
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
        self.banner(&format!("\r\n[husk] session ended: {text}\r\n[husk] tap to start again\r\n"));
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
                        self.cur = 0;
                        self.has_cur = false;
                    }
                    b']' => self.esc = 3,
                    _ => self.esc = 0,
                },
                2 => match b {
                    b'0'..=b'9' => {
                        self.cur = self.cur.saturating_mul(10).saturating_add((b - b'0') as u32);
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
                _ => {
                    if b == 0x07 {
                        self.esc = 0;
                    }
                }
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
