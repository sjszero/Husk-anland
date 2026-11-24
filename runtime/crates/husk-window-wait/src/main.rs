//! `husk-window-wait [app]` — time from spawning a GTK app to the first *new*
//! viewable top-level window on `$DISPLAY`.
//!
//! Dimension 5 of the benchmark needs "app start -> window really on screen",
//! not "app start -> process alive". The same binary runs under every runner,
//! and it only talks to the X server, so the number is the runner's cost and
//! nothing else.
//!
//! Used to be `tools/bench/guest/window_wait.py`, which needed python3-xlib in
//! the guest. This is a static-ish Rust binary that dlopens `libX11.so.6`, so
//! the guest needs nothing but the X client library it already has.
//!
//! Prints `window_wait<TAB><ms>` (or `-1` on timeout, `skip` when there is no
//! display) and exits 0: the benchmark harness parses the line, it does not read
//! the exit status.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Display = c_void;
type Window = c_ulong;
type Visual = c_void;
type Screen = c_void;

/// The prefix of Xlib's `XWindowAttributes` up to `map_state`, laid out by the C
/// ABI rules Rust follows for `#[repr(C)]`. Copied field for field from
/// `<X11/Xlib.h>`; only `map_state` is read.
#[repr(C)]
struct XWindowAttributes {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    depth: c_int,
    visual: *mut Visual,
    root: Window,
    class: c_int,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    colormap: c_ulong,
    map_installed: c_int,
    map_state: c_int,
    all_event_masks: c_long_t,
    your_event_mask: c_long_t,
    do_not_propagate_mask: c_long_t,
    override_redirect: c_int,
    screen: *mut Screen,
}

#[allow(non_camel_case_types)]
type c_long_t = std::os::raw::c_long;

const IS_VIEWABLE: c_int = 2;

fn dlopen(name: &str) -> Option<*mut c_void> {
    let path = CString::new(name).ok()?;
    unsafe {
        let handle = libc_dlopen(path.as_ptr(), 2 /* RTLD_NOW */);
        if handle.is_null() {
            None
        } else {
            Some(handle)
        }
    }
}

extern "C" {
    #[link_name = "dlopen"]
    fn libc_dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    #[link_name = "dlsym"]
    fn libc_dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

fn sym(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
    let c = CString::new(name).ok()?;
    let p = unsafe { libc_dlsym(handle, c.as_ptr()) };
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

struct X11 {
    display: *mut Display,
    root: Window,
    query_tree: unsafe extern "C" fn(
        *mut Display,
        Window,
        *mut Window,
        *mut Window,
        *mut *mut Window,
        *mut c_uint,
    ) -> c_int,
    get_attributes: unsafe extern "C" fn(*mut Display, Window, *mut XWindowAttributes) -> c_int,
    free: unsafe extern "C" fn(*mut c_void) -> c_int,
    close_display: unsafe extern "C" fn(*mut Display) -> c_int,
}

impl X11 {
    fn open() -> Option<X11> {
        let handle = dlopen("libX11.so.6")?;
        unsafe {
            let open: unsafe extern "C" fn(*const c_char) -> *mut Display =
                std::mem::transmute(sym(handle, "XOpenDisplay")?);
            let root_of: unsafe extern "C" fn(*mut Display) -> Window =
                std::mem::transmute(sym(handle, "XDefaultRootWindow")?);
            let query_tree = std::mem::transmute(sym(handle, "XQueryTree")?);
            let get_attributes = std::mem::transmute(sym(handle, "XGetWindowAttributes")?);
            let free = std::mem::transmute(sym(handle, "XFree")?);
            let close_display = std::mem::transmute(sym(handle, "XCloseDisplay")?);
            let display = open(std::ptr::null());
            if display.is_null() {
                return None;
            }
            Some(X11 {
                display,
                root: root_of(display),
                query_tree,
                get_attributes,
                free,
                close_display,
            })
        }
    }

    /// Window ids that are currently mapped (`IsViewable`) below the root.
    fn viewable(&self) -> Vec<Window> {
        let mut root_return: Window = 0;
        let mut parent_return: Window = 0;
        let mut children: *mut Window = std::ptr::null_mut();
        let mut count: c_uint = 0;
        let mut out = Vec::new();
        let ok = unsafe {
            (self.query_tree)(
                self.display,
                self.root,
                &mut root_return,
                &mut parent_return,
                &mut children,
                &mut count,
            )
        };
        if ok == 0 || children.is_null() {
            return out;
        }
        for i in 0..count as usize {
            let window = unsafe { *children.add(i) };
            let mut attrs = std::mem::MaybeUninit::<XWindowAttributes>::zeroed();
            let got = unsafe { (self.get_attributes)(self.display, window, attrs.as_mut_ptr()) };
            if got != 0 && unsafe { attrs.assume_init_ref() }.map_state == IS_VIEWABLE {
                out.push(window);
            }
        }
        unsafe { (self.free)(children as *mut c_void) };
        out
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        if !self.display.is_null() {
            unsafe { (self.close_display)(self.display) };
        }
    }
}

fn spawn(app: &str) -> Option<Child> {
    Command::new(app)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn main() {
    let app = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gnome-control-center".to_string());
    if std::env::var_os("DISPLAY").is_none() {
        println!("window_wait\tskip\tDISPLAY unset");
        return;
    }
    let x11 = match X11::open() {
        Some(x) => x,
        None => {
            println!(
                "window_wait\tskip\tcannot open {}",
                std::env::var("DISPLAY").unwrap_or_default()
            );
            return;
        }
    };
    let before: Vec<Window> = x11.viewable();
    let started = Instant::now();
    let mut child = match spawn(&app) {
        Some(c) => c,
        None => {
            println!("window_wait\tskip\tcannot spawn {app}");
            return;
        }
    };
    let deadline = Duration::from_secs(60);
    let mut elapsed: Option<Duration> = None;
    while started.elapsed() < deadline {
        let now = x11.viewable();
        if now.iter().any(|w| !before.contains(w)) {
            elapsed = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    match elapsed {
        Some(d) => println!("window_wait\t{}", d.as_millis()),
        None => println!("window_wait\t-1"),
    }
    let _ = child.kill();
    let _ = child.wait();
    // Keep the CStr import used on every path that opened a display.
    let _ = CStr::from_bytes_with_nul(b"\0");
}
