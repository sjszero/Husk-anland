//! Husk app shell, in Rust only.
//!
//! There is no Java and no C in this process: the APK declares
//! `android.app.NativeActivity`, the platform calls `ANativeActivity_onCreate`
//! in this library, and from there everything - PTY, guest loader, GL surface,
//! text, lifecycle - is done here.
//!
//! Lifecycle contract (what this file guarantees):
//!   * the guest session lives as long as the process, not the window;
//!   * a destroyed surface only takes the GL objects with it;
//!   * pause/stop keep the PTY reading, so a backgrounded session keeps its
//!     output instead of silently dropping it;
//!   * a dead shell (child exited) is restarted by the next tap.
#![allow(non_snake_case)]

mod gl;
mod sys;
mod term;

use core::ffi::{c_char, c_int, c_long, c_void};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use sys::*;
use term::Term;
mod trace;

static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static INPUTQ: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static ACTIVITY: AtomicPtr<ANativeActivity> = AtomicPtr::new(core::ptr::null_mut());
static DESTROYED: AtomicBool = AtomicBool::new(false);
static FOCUSED: AtomicBool = AtomicBool::new(false);
static LOOPER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static APP: Mutex<Option<Box<App>>> = Mutex::new(None);
/// (pid, wait status) collected by the single reaper thread. Nothing else in the
/// process is allowed to call waitpid(): two waiters is how zombies get leaked.
static EXITS: Mutex<Vec<(i32, i32)>> = Mutex::new(Vec::new());
static REAPER: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

pub fn log(msg: &str) {
    if let Ok(c) = std::ffi::CString::new(msg) {
        unsafe {
            __android_log_write(4, b"Husk\0".as_ptr() as *const c_char, c.as_ptr());
        }
    }
}

// The pointers inside are only touched from the loop thread; the platform
// callbacks publish raw handles through atomics and never touch this.
unsafe impl Send for App {}

struct App {
    term: Option<Term>,
    gfx: Option<gl::Gfx>,
    attached_queue: usize,
    win_ptr: usize,
    dirty: bool,
    /// Screen size the terminal was last laid out for, so a resize is a diff and
    /// not a repaint storm.
    laid_out: (usize, usize),
    /// Scrollback offset in lines, 0 = bottom.
    scroll: usize,
    drag: i32,
    /// The session is started exactly once per process. A dead guest stays dead
    /// until the user taps -- auto-respawn turns any broken rootfs into a fork
    /// storm (700+ zombies on 2026-11-22).
    started: bool,
    last_spawn: Option<std::time::Instant>,
    no_window_logged: bool,
}

impl App {
    fn new() -> App {
        App {
            term: None,
            gfx: None,
            attached_queue: 0,
            win_ptr: 0,
            dirty: true,
            laid_out: (0, 0),
            scroll: 0,
            drag: 0,
            started: false,
            last_spawn: None,
            no_window_logged: false,
        }
    }

    fn paths(&self) -> Option<(String, String, String)> {
        let act = ACTIVITY.load(Ordering::Relaxed);
        if act.is_null() {
            return None;
        }
        let act = unsafe { &*act };
        if act.internalDataPath.is_null() {
            return None;
        }
        let files = unsafe { std::ffi::CStr::from_ptr(act.internalDataPath) }.to_string_lossy().to_string();
        let rootfs = format!("{files}/rootfs");
        let libdir = native_lib_dir()?;
        let loader = format!("{libdir}/libld.so");
        Some((rootfs, libdir, loader))
    }

    /// Start the session once, as soon as we know the screen size.
    fn maybe_start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        // `files/trace` switches this launch into the ptrace snitch: one traced
        // loader run whose syscall numbers land in logcat.
        if let Some((rootfs, _apklib, loader)) = self.paths() {
            if std::path::Path::new(&format!("{rootfs}/../trace")).exists() {
                let guestlib = format!("{rootfs}/usr/lib/aarch64-linux-gnu");
                std::thread::spawn(move || trace::spy(&loader, &guestlib, &rootfs));
                return;
            }
        }
        self.ensure_term();
    }

    /// User-driven restart (tap on a dead session). Rate limited so a stray
    /// double tap cannot fork twice.
    fn restart_session(&mut self) {
        const MIN_GAP: core::time::Duration = core::time::Duration::from_millis(400);
        if let Some(t) = self.last_spawn {
            if t.elapsed() < MIN_GAP {
                return;
            }
        }
        self.term = None;
        self.ensure_term();
    }

    /// Drain what the reaper thread saw and give each status to its session.
    fn poll_children(&mut self) {
        let done: Vec<(i32, i32)> = match EXITS.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        for (pid, st) in done {
            let mine = self.term.as_ref().map(|t| t.pid == pid).unwrap_or(false);
            if mine {
                if let Some(t) = self.term.as_mut() {
                    t.note_exit(st);
                }
                self.dirty = true;
            } else {
                // A grandchild (bash's own children) reparented to us by
                // PR_SET_CHILD_SUBREAPER. Reaped, nothing else to do.
                crate::log(&format!("reaped orphan pid={pid} status={st}"));
            }
        }
    }

    fn ensure_term(&mut self) {
        if let Some(t) = &self.term {
            if !t.dead {
                return;
            }
        }
        let Some((rootfs, _libdir, loader)) = self.paths() else { return };
        if !std::path::Path::new(&format!("{rootfs}/usr/bin/bash")).exists() {
            log(&format!("no rootfs at {rootfs}"));
            return;
        }
        let libpath = format!("{rootfs}/usr/lib/aarch64-linux-gnu");
        let (cols, rows) = self.laid_out;
        let (cols, rows) = if cols > 0 { (cols, rows) } else { (80, 24) };
        match Term::spawn(&loader, &libpath, &rootfs, cols, rows) {
            Some(t) => {
                self.last_spawn = Some(std::time::Instant::now());
                log("guest session started");
                if let Some(old) = self.term.take() {
                    if old.fd >= 0 {
                        unsafe { close(old.fd) };
                    }
                }
                let fd = t.fd;
                self.term = Some(t);
                let l = LOOPER.load(Ordering::Relaxed);
                if !l.is_null() && fd >= 0 {
                    unsafe {
                        ALooper_addFd(l, fd, 1000, ALOOPER_EVENT_INPUT, core::ptr::null_mut(), core::ptr::null_mut());
                    }
                }
                self.dirty = true;
            }
            None => log("PTY spawn failed"),
        }
    }

    /// Rebuilds the GL side when the platform hands us a new surface, and lays
    /// the terminal out for the current pixel size.
    fn sync_surface(&mut self) {
        let win = WINDOW.load(Ordering::SeqCst) as usize;
        if win == 0 {
            if !self.no_window_logged {
                self.no_window_logged = true;
                log("sync: waiting for onNativeWindowCreated");
            }
            if self.gfx.is_some() {
                self.gfx = None; // Drop tears down EGL; the session keeps running.
            }
            self.win_ptr = 0;
            return;
        }
        if self.gfx.is_none() || win != self.win_ptr {
            self.gfx = gl::Gfx::new(win as *mut c_void);
            self.win_ptr = win;
            self.dirty = true;
            if self.gfx.is_none() {
                log(&format!("EGL surface failed for win={win:#x}"));
                return;
            }
        }
        if let Some(g) = &self.gfx {
            let (w, h) = g.size();
            let (cw, ch) = g.cell();
            if w > 0 && h > 0 && cw > 1.0 && ch > 1.0 {
                let cols = ((w as f32 / cw).floor() as usize).max(1);
                let rows = ((h as f32 / ch).floor() as usize).max(1);
                if (cols, rows) != self.laid_out {
                    self.laid_out = (cols, rows);
                    match &mut self.term {
                        Some(t) => t.resize(cols, rows),
                        None => self.term = Some(Term::blank(cols, rows)),
                    }
                    self.dirty = true;
                }
            }
        }
    }
}

fn native_lib_dir() -> Option<String> {
    let addr = native_lib_dir as *const c_void;
    let mut info = DlInfo {
        dli_fname: core::ptr::null(),
        dli_fbase: core::ptr::null_mut(),
        dli_sname: core::ptr::null(),
        dli_saddr: core::ptr::null_mut(),
    };
    let ok = unsafe { dladdr(addr, &mut info) };
    if ok == 0 || info.dli_fname.is_null() {
        return None;
    }
    let p = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }.to_string_lossy().to_string();
    let idx = p.rfind('/')?;
    Some(p[..idx].to_string())
}

// ------------------------------------------------------------- callbacks ---

static CALLBACKS: ANativeActivityCallbacks = ANativeActivityCallbacks {
    onStart: Some(on_start),
    onResume: Some(on_resume),
    onSaveInstanceState: None,
    onPause: Some(on_pause),
    onStop: None,
    onDestroy: Some(on_destroy),
    onWindowFocusChanged: Some(on_focus),
    onNativeWindowCreated: Some(on_window),
    onNativeWindowResized: Some(on_window),
    onNativeWindowRedrawNeeded: Some(on_window),
    onNativeWindowDestroyed: Some(on_window_gone),
    onInputQueueCreated: Some(on_input_queue),
    onInputQueueDestroyed: Some(on_input_queue_gone),
    onContentRectChanged: None,
    onConfigurationChanged: Some(on_config),
    onLowMemory: None,
};

unsafe extern "C" fn on_start(_a: *mut ANativeActivity) { log("cb: onStart"); }
unsafe extern "C" fn on_resume(_a: *mut ANativeActivity) { log("cb: onResume"); }
unsafe extern "C" fn on_pause(_a: *mut ANativeActivity) { log("cb: onPause"); }
unsafe extern "C" fn on_config(_a: *mut ANativeActivity) {
    if let Ok(mut g) = APP.lock() {
        if let Some(app) = g.as_mut() {
            app.dirty = true;
        }
    }
}

unsafe extern "C" fn on_focus(_a: *mut ANativeActivity, focused: c_int) {
    FOCUSED.store(focused != 0, Ordering::SeqCst);
    if let Ok(mut g) = APP.lock() {
        if let Some(app) = g.as_mut() {
            app.dirty = true;
        }
    }
}

unsafe extern "C" fn on_window(_a: *mut ANativeActivity, w: *mut c_void) {
    // Width/height straight from the window, so the log says whether the
    // surface the platform handed us is a real one or a 0x0 stub.
    let (mut ww, mut wh) = (0i32, 0i32);
    if !w.is_null() {
        ww = ANativeWindow_getWidth(w as *mut _);
        wh = ANativeWindow_getHeight(w as *mut _);
    }
    log(&format!("cb: onNativeWindow win={:p} {ww}x{wh}", w));
    WINDOW.store(w, Ordering::SeqCst);
}

unsafe extern "C" fn on_window_gone(_a: *mut ANativeActivity, w: *mut c_void) {
    log(&format!("cb: onNativeWindowDestroyed win={:p}", w));
    WINDOW.store(core::ptr::null_mut(), Ordering::SeqCst);
}

unsafe extern "C" fn on_input_queue(_a: *mut ANativeActivity, q: *mut c_void) {
    log(&format!("cb: onInputQueueCreated q={:p}", q));
    INPUTQ.store(q, Ordering::SeqCst);
    let looper = LOOPER.load(Ordering::SeqCst);
    if !looper.is_null() {
        AInputQueue_attachLooper(q, looper, 2000, core::ptr::null_mut(), core::ptr::null_mut());
    }
}

unsafe extern "C" fn on_input_queue_gone(_a: *mut ANativeActivity, _q: *mut c_void) {
    INPUTQ.store(core::ptr::null_mut(), Ordering::SeqCst);
}

unsafe extern "C" fn on_destroy(_a: *mut ANativeActivity) {
    log("cb: onDestroy");
    DESTROYED.store(true, Ordering::SeqCst);
    let l = LOOPER.load(Ordering::SeqCst);
    if !l.is_null() {
        ALooper_wake(l);
    }
}

// ---------------------------------------------------------------- the app ---

#[no_mangle]
pub unsafe extern "C" fn ANativeActivity_onCreate(activity: *mut ANativeActivity, _state: *mut c_void, _size: usize) {
    log(&format!("onCreate activity={:p}", activity));
    if activity.is_null() {
        return;
    }
    ACTIVITY.store(activity, Ordering::SeqCst);
    // The platform hands us a callbacks struct of its own. Copying into it keeps
    // working whether the framework re-reads `activity->callbacks` on every
    // event (NDK doc model) or keeps its static struct pointer and dereferences
    // that. Replacing the pointer only works for the former; on this ROM the
    // pointer swap produced *no* lifecycle or window callback at all, which is
    // what a stale static pointer looks like.
    let fw_cb = (*activity).callbacks;
    if !fw_cb.is_null() {
        let p = fw_cb as usize;
        let aligned = p % core::mem::align_of::<ANativeActivityCallbacks>() == 0;
        if aligned {
            let bytes = core::mem::size_of::<ANativeActivityCallbacks>();
            log(&format!("onCreate callbacks={fw_cb:p} -> in-place copy ({bytes} bytes)"));
            core::ptr::copy_nonoverlapping(
                &CALLBACKS as *const ANativeActivityCallbacks as *const u8,
                fw_cb as *mut u8,
                bytes,
            );
        } else {
            log(&format!("onCreate callbacks={fw_cb:p} misaligned"));
            (*activity).callbacks = &CALLBACKS as *const _ as *mut ANativeActivityCallbacks;
        }
    } else {
        log("onCreate callbacks=null -> installing ours");
        (*activity).callbacks = &CALLBACKS as *const _ as *mut ANativeActivityCallbacks;
    }
    (*activity).instance = core::ptr::null_mut();
    if let Ok(mut g) = APP.lock() {
        *g = Some(Box::new(App::new()));
    }
    probe_sigsys_semantics();
    // In snitch mode the tracee's ptrace stops are reaped by the spy thread.
    // A second waitpid(-1) in the same process would steal them and hang the run.
    if !trace_requested() {
        start_reaper();
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(run);
    });
}


/// One-shot experiment: does this ROM's app-domain seccomp use TRAP (catchable)
/// or KILL (the process is gone, no handler ever runs)?
///
/// Method: the child installs a SIGSYS handler, then calls `openat2` (437), which
/// this ROM is known to disallow for apps. Handler ran -> child exits 42 (TRAP).
/// Child died of SIGSYS -> KILL, and no in-process emulation is possible.
/// Child got an errno back -> 43 (allowed after all).
unsafe extern "C" fn probe_sig_handler(_s: c_int, _i: *mut c_void, _u: *mut c_void) {
    _exit(42);
}

fn probe_sigsys_semantics() {
    unsafe {
        let pid = fork();
        if pid == 0 {
            let mut act = [0usize; 4];
            act[0] = probe_sig_handler as usize;
            act[1] = 0x1004; // SA_SIGINFO | SA_RESTART
            let r = syscall(134, SIGSYS as c_long, act.as_ptr() as c_long, 0 as c_long, 8 as c_long);
            crate::log(&format!("probe: rt_sigaction ret={r}"));
            let code = syscall(437, 0 as c_long, 0 as c_long, 0 as c_long, 0 as c_long);
            _exit(if code < 0 { 43 } else { 44 });
        }
        let mut st: c_int = 0;
        waitpid(pid, &mut st, 0);
        let verdict = if wifexited(st) {
            match wexitstatus(st) {
                42 => "TRAP (handler ran: in-process emulation possible)".to_string(),
                43 => "ALLOWED (syscall returned errno)".to_string(),
                44 => "ALLOWED (syscall succeeded)".to_string(),
                c => format!("exit {c}"),
            }
        } else {
            format!("signal {} ({}) -- KILL, nothing can catch it", wtermsig(st), sig_name(wtermsig(st)))
        };
        crate::log(&format!("probe: openat2 semantics = {verdict}"));
    }
}

/// True when `files/trace` exists (see trace.rs).
fn trace_requested() -> bool {
    unsafe {
        let act = ACTIVITY.load(Ordering::Relaxed);
        if act.is_null() {
            return false;
        }
        let a = &*act;
        if a.internalDataPath.is_null() {
            return false;
        }
        let files = CStr::from_ptr(a.internalDataPath).to_string_lossy().to_string();
        std::path::Path::new(&format!("{files}/trace")).exists()
    }
}

/// One thread, one waitpid(-1), for the whole process.
///
/// `PR_SET_CHILD_SUBREAPER` makes orphaned grandchildren (bash's children, vim's
/// children) reparent to us instead of init, so they are reaped here too. This is
/// what proot does with its tracee list, and what tini/runc's PID 1 does with
/// SIGCHLD: exactly one owner of child status.
fn start_reaper() {
    if REAPER.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        unsafe {
            prctl(PR_SET_CHILD_SUBREAPER, 1 as c_long, 0 as c_long, 0 as c_long, 0 as c_long);
        }
        loop {
            let mut st: c_int = 0;
            let pid = unsafe { waitpid(-1, &mut st, 0) };
            if pid > 0 {
                if let Ok(mut q) = EXITS.lock() {
                    q.push((pid, st));
                }
                continue;
            }
            let e = unsafe { *__errno() };
            if e == EINTR {
                continue;
            }
            // ECHILD: nothing to wait for right now. Idle instead of spinning.
            unsafe { usleep(50_000) };
        }
    });
}

fn run() {
    log("run: thread up");
    let looper = unsafe { ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
    if looper.is_null() {
        log("no looper");
        return;
    }
    LOOPER.store(looper, Ordering::SeqCst);

    let q = INPUTQ.load(Ordering::SeqCst);
    if !q.is_null() {
        unsafe { AInputQueue_attachLooper(q, looper, 2000, core::ptr::null_mut(), core::ptr::null_mut()) };
    }

    while !DESTROYED.load(Ordering::SeqCst) {
        if let Ok(mut guard) = APP.lock() {
            if let Some(app) = guard.as_mut() {
                app.sync_surface();
                app.poll_children();
                app.maybe_start();
                let q = INPUTQ.load(Ordering::SeqCst) as usize;
                if q != 0 && app.attached_queue != q {
                    unsafe {
                        AInputQueue_attachLooper(q as *mut c_void, looper, 2000, core::ptr::null_mut(), core::ptr::null_mut());
                    }
                    app.attached_queue = q;
                }
            }
        }

        let mut ident: c_int = 0;
        let mut events: c_int = 0;
        let mut data: *mut c_void = core::ptr::null_mut();
        let id = unsafe { ALooper_pollOnce(32, &mut ident, &mut events, &mut data) };

        if let Ok(mut guard) = APP.lock() {
            if let Some(app) = guard.as_mut() {
                if id == 1000 {
                    if let Some(t) = app.term.as_mut() {
                        if t.read_available() {
                            app.dirty = true;
                        }
                    }
                } else if id == 2000 {
                    let q = INPUTQ.load(Ordering::SeqCst);
                    if !q.is_null() && drain_input(q, app) {
                        app.dirty = true;
                    }
                }
                if app.dirty {
                    let focused = FOCUSED.load(Ordering::SeqCst);
                    if let (Some(gfx), Some(t)) = (app.gfx.as_mut(), app.term.as_ref()) {
                        let back = app.scroll.min(t.max_scroll());
                        let view = t.snapshot(back);
                        let cursor = if back > 0 { (t.cols, t.rows) } else { (t.cx, t.cy) };
                        gfx.draw(&view, t.cols, t.rows, cursor.0, cursor.1, focused && back == 0);
                    } else {
                        app.dirty = false;
                        continue;
                    }
                    app.dirty = false;
                }
            }
        }
    }
    log("loop exit");
    if let Ok(mut g) = APP.lock() {
        *g = None;
    }
}

fn drain_input(q: *mut c_void, app: &mut App) -> bool {
    let mut ev: *mut c_void = core::ptr::null_mut();
    let mut handled_any = false;
    loop {
        let rc = unsafe { AInputQueue_getEvent(q, &mut ev) };
        if rc < 0 || ev.is_null() {
            break;
        }
        let mut handled = 0;
        unsafe {
            if AInputQueue_preDispatchEvent(q, ev) != 0 {
                handled = 1;
            } else {
                let ty = AInputEvent_getType(ev);
                if ty == AINPUT_EVENT_TYPE_KEY {
                    if AKeyEvent_getAction(ev) == AKEY_EVENT_ACTION_DOWN {
                        let meta = AKeyEvent_getMetaState(ev);
                        let code = AKeyEvent_getKeyCode(ev);
                        app.scroll = 0;
                        if let Some(t) = app.term.as_mut() {
                            match code {
                                AKEYCODE_DEL => t.send(&[0x7f]),
                                AKEYCODE_ENTER => t.send(b"\r"),
                                AKEYCODE_TAB => t.send(b"\t"),
                                AKEYCODE_DPAD_UP => t.send(b"\x1b[A"),
                                AKEYCODE_DPAD_DOWN => t.send(b"\x1b[B"),
                                AKEYCODE_DPAD_RIGHT => t.send(b"\x1b[C"),
                                AKEYCODE_DPAD_LEFT => t.send(b"\x1b[D"),
                                _ => {
                                    if let Some(ch) = key_text(code, meta) {
                                        let mut b = [0u8; 4];
                                        let s = ch.encode_utf8(&mut b);
                                        t.send(s.as_bytes());
                                    }
                                }
                            }
                        }
                        handled = 1;
                    }
                } else if ty == AINPUT_EVENT_TYPE_MOTION {
                    let action = AMotionEvent_getAction(ev) & AMOTION_EVENT_ACTION_MASK;
                    match action {
                        AMOTION_EVENT_ACTION_DOWN => {
                            app.drag = AMotionEvent_getY(ev, 0) as i32;
                            if app.term.as_ref().map(|t| t.dead).unwrap_or(false) {
                                app.restart_session();
                            }
                            let act = ACTIVITY.load(Ordering::SeqCst);
                            if !act.is_null() {
                                ANativeActivity_showSoftInput(act, 0);
                            }
                            handled = 1;
                        }
                        AMOTION_EVENT_ACTION_MOVE => {
                            let y = AMotionEvent_getY(ev, 0) as i32;
                            let dy = y - app.drag;
                            // ~40 px per line: finger down = scroll back in time
                            if dy.abs() >= 40 {
                                let step = (dy / 40) as usize;
                                let max = app.term.as_ref().map(|t| t.max_scroll()).unwrap_or(0);
                                if dy > 0 {
                                    app.scroll = (app.scroll + step).min(max);
                                } else {
                                    app.scroll = app.scroll.saturating_sub(step);
                                }
                                app.drag = y;
                                handled = 1;
                            }
                        }
                        AMOTION_EVENT_ACTION_UP => {
                            app.drag = 0;
                        }
                        _ => {}
                    }
                }
                AInputQueue_finishEvent(q, ev, handled);
            }
        }
        if handled == 1 {
            handled_any = true;
        }
    }
    handled_any
}

/// US-layout keycode -> text.  The NDK has no "unicode char" accessor (that
/// symbol does not exist in libandroid, verified against a device's libandroid.so),
/// so the mapping lives here.  This is what hardware keyboards and
/// `adb shell input text` deliver.
fn key_text(code: i32, meta: i32) -> Option<char> {
    let shift = meta & AMETA_SHIFT_ON != 0;
    let c = match code {
        AKEYCODE_SPACE => ' ',
        AKEYCODE_COMMA => if shift { '<' } else { ',' },
        AKEYCODE_PERIOD => if shift { '>' } else { '.' },
        AKEYCODE_SLASH => if shift { '?' } else { '/' },
        AKEYCODE_SEMICOLON => if shift { ':' } else { ';' },
        AKEYCODE_APOSTROPHE => if shift { '"' } else { '\'' },
        AKEYCODE_GRAVE => if shift { '~' } else { '`' },
        AKEYCODE_MINUS => if shift { '_' } else { '-' },
        AKEYCODE_EQUALS => if shift { '+' } else { '=' },
        AKEYCODE_LEFT_BRACKET => if shift { '{' } else { '[' },
        AKEYCODE_RIGHT_BRACKET => if shift { '}' } else { ']' },
        AKEYCODE_BACKSLASH => if shift { '|' } else { '\\' },
        AKEYCODE_ESCAPE => '\x1b',
        c if c >= AKEYCODE_0 && c <= AKEYCODE_9 => {
            let d = b'0' + (c - AKEYCODE_0) as u8;
            if shift {
                match c - AKEYCODE_0 {
                    0 => ')',
                    1 => '!',
                    2 => '@',
                    3 => '#',
                    4 => '$',
                    5 => '%',
                    6 => '^',
                    7 => '&',
                    8 => '*',
                    _ => '(',
                }
            } else {
                d as char
            }
        }
        c if c >= AKEYCODE_A && c <= AKEYCODE_Z => {
            let d = (b'a' + (c - AKEYCODE_A) as u8) as char;
            if shift { d.to_ascii_uppercase() } else { d }
        }
        _ => return None,
    };
    Some(c)
}
