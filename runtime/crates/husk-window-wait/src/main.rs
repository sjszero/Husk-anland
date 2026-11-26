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
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_ulong, c_void};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Display = c_void;
type Window = c_ulong;
type Atom = c_ulong;
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
    intern_atom: unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> Atom,
    get_property: unsafe extern "C" fn(
        *mut Display,
        Window,
        Atom,
        c_ulong,
        c_ulong,
        c_int,
        Atom,
        *mut Atom,
        *mut c_int,
        *mut c_ulong,
        *mut c_ulong,
        *mut *mut c_uchar,
    ) -> c_int,
    fetch_name: unsafe extern "C" fn(*mut Display, Window, *mut *mut c_char) -> c_int,
    display_width: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    display_height: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
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
            let intern_atom = std::mem::transmute(sym(handle, "XInternAtom")?);
            let get_property = std::mem::transmute(sym(handle, "XGetWindowProperty")?);
            let fetch_name = std::mem::transmute(sym(handle, "XFetchName")?);
            let display_width = std::mem::transmute(sym(handle, "XDisplayWidth")?);
            let display_height = std::mem::transmute(sym(handle, "XDisplayHeight")?);
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
                intern_atom,
                get_property,
                fetch_name,
                display_width,
                display_height,
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

    fn root_property(&self, name: &str) -> Option<c_ulong> {
        let atom_name = CString::new(name).ok()?;
        let atom = unsafe { (self.intern_atom)(self.display, atom_name.as_ptr(), 0) };
        if atom == 0 {
            return None;
        }
        let mut actual_type: Atom = 0;
        let mut actual_format: c_int = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut data: *mut c_uchar = std::ptr::null_mut();
        let rc = unsafe {
            (self.get_property)(
                self.display,
                self.root,
                atom,
                0,
                1,
                0,
                0,
                &mut actual_type,
                &mut actual_format,
                &mut nitems,
                &mut bytes_after,
                &mut data,
            )
        };
        let value = if rc == 0 && actual_format == 32 && nitems == 1 && !data.is_null() {
            Some(unsafe { *(data as *const c_long_t) as c_ulong })
        } else {
            None
        };
        if !data.is_null() {
            unsafe { (self.free)(data as *mut c_void) };
        }
        value
    }

    fn window_name(&self, window: Window) -> Option<String> {
        let mut name: *mut c_char = std::ptr::null_mut();
        let ok = unsafe { (self.fetch_name)(self.display, window, &mut name) };
        if ok == 0 || name.is_null() {
            return None;
        }
        let text = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.free)(name as *mut c_void) };
        Some(text)
    }

    fn viewable_named(&self) -> Vec<(Window, String)> {
        self.viewable()
            .into_iter()
            .filter_map(|window| self.window_name(window).map(|name| (window, name)))
            .collect()
    }

    fn geometry(&self) -> (c_int, c_int) {
        unsafe {
            (
                (self.display_width)(self.display, 0),
                (self.display_height)(self.display, 0),
            )
        }
    }

    fn prime_input(&self) -> bool {
        let handle = match dlopen("libXtst.so.6") {
            Some(handle) => handle,
            None => return false,
        };
        let motion: unsafe extern "C" fn(*mut Display, c_int, c_int, c_int, c_ulong) -> c_int =
            match sym(handle, "XTestFakeMotionEvent") {
                Some(symbol) => unsafe { std::mem::transmute(symbol) },
                None => return false,
            };
        let button: unsafe extern "C" fn(*mut Display, c_uint, c_int, c_ulong) -> c_int =
            match sym(handle, "XTestFakeButtonEvent") {
                Some(symbol) => unsafe { std::mem::transmute(symbol) },
                None => return false,
            };
        let flush: unsafe extern "C" fn(*mut Display) -> c_int = match sym(handle, "XFlush") {
            Some(symbol) => unsafe { std::mem::transmute(symbol) },
            None => return false,
        };
        let (width, height) = self.geometry();
        if width <= 16 || height <= 16 {
            return false;
        }
        let x = width / 2;
        let y = height / 4 * 3;
        unsafe {
            if (motion)(self.display, 0, x, y, 0) == 0 {
                return false;
            }
            let _ = (button)(self.display, 1, 1, 0);
            let _ = (motion)(
                self.display,
                0,
                (x + 8).min(width - 1),
                (y + 8).min(height - 1),
                0,
            );
            let _ = (button)(self.display, 1, 0, 0);
            let _ = (flush)(self.display);
        }
        true
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

/// GNOME/DING append a numeric instance suffix to the X11 title, e.g.
/// `Desktop Icons 1`. Accept only that exact shape so `Desktop` does not match
/// the unrelated `Desktop Icons` window.
fn window_title_matches(title: &str, name: &str) -> bool {
    title == name
        || title.strip_prefix(name).is_some_and(|rest| {
            rest.strip_prefix(' ').is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
            })
        })
}

fn open_display_or_skip() -> Option<X11> {
    if std::env::var_os("DISPLAY").is_none() {
        println!("window_wait\tskip\tDISPLAY unset");
        return None;
    }
    match X11::open() {
        Some(x) => Some(x),
        None => {
            println!(
                "window_wait\tskip\tcannot open {}",
                std::env::var("DISPLAY").unwrap_or_default()
            );
            None
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--help") {
        println!("usage: husk-window-wait [APP]");
        println!("       husk-window-wait --probe-wm");
        println!("       husk-window-wait --wait-window NAME [TIMEOUT_MS]");
        println!("       husk-window-wait --geometry");
        println!("       husk-window-wait --prime-input");
        println!("       husk-window-wait --list");
        return;
    }
    let x11 = match open_display_or_skip() {
        Some(x) => x,
        None => return,
    };
    match args.first().map(String::as_str) {
        Some("--probe-wm") => {
            let have = x11.root_property("_NET_SUPPORTING_WM_CHECK").is_some();
            println!("wm={}", if have { "yes" } else { "no" });
            return;
        }
        Some("--wait-window") => {
            let name = args.get(1).map(String::as_str).unwrap_or("Desktop Icons");
            let timeout_ms = args
                .get(2)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(30_000);
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            let found = loop {
                if x11
                    .viewable_named()
                    .iter()
                    .any(|(_, title)| window_title_matches(title, name))
                {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            println!("window={}", if found { "yes" } else { "no" });
            return;
        }
        Some("--geometry") => {
            let (width, height) = x11.geometry();
            println!("{width}x{height}");
            return;
        }
        Some("--prime-input") => {
            println!(
                "input={}",
                if x11.prime_input() { "primed" } else { "skip" }
            );
            return;
        }
        Some("--list") => {
            for (window, title) in x11.viewable_named() {
                println!("0x{window:x}\t{title}");
            }
            return;
        }
        _ => {}
    }
    let app = args
        .first()
        .cloned()
        .unwrap_or_else(|| "gnome-control-center".to_string());
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
}

#[cfg(test)]
mod tests {
    use super::window_title_matches;

    #[test]
    fn window_title_accepts_instance_suffix() {
        assert!(window_title_matches("Desktop Icons 1", "Desktop Icons"));
        assert!(window_title_matches("Desktop Icons", "Desktop Icons"));
    }

    #[test]
    fn window_title_rejects_unrelated_prefix() {
        assert!(!window_title_matches("Desktop Icons 1", "Desktop"));
        assert!(!window_title_matches("Desktop", "Desktop Icons"));
    }
}
