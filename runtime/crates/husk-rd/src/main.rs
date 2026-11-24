//! husk-rd — drive GNOME/mutter from the guest over its own D-Bus APIs.
//!
//! Mutter 46 exposes `org.gnome.Mutter.RemoteDesktop` (input injection) and
//! `org.gnome.Mutter.ScreenCast` (virtual monitors). Both are what
//! gnome-remote-desktop itself uses, so this is a supported route, not a hack.
//!
//! Prior art: anland does the same thing one layer lower (a Clutter virtual
//! input device inside a patched mutter); we use the stock D-Bus API so that
//! nothing in the guest has to be patched.
//!
//! GIO is reached through dlopen: our loader shim only rewrites paths for
//! libraries the program links against, and linking libgio at build time would
//! need the rootfs .so in the cross sysroot. dlopen("libgio-2.0.so.0") goes
//! through the guest's own loader path and works.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::io::Read;
use std::os::unix::net::UnixStream;

// ---------------------------------------------------------------- GIO via dlfcn

type GV = *mut c_void;
type Conn = *mut c_void;

#[repr(C)]
struct Dl {
    h: *mut c_void,
}

impl Dl {
    fn open() -> Option<Dl> {
        for name in [
            "/usr/lib/aarch64-linux-gnu/libgio-2.0.so.0",
            "libgio-2.0.so.0",
            "/usr/lib/libgio-2.0.so.0",
        ] {
            let c = CString::new(name).ok()?;
            let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
            if !h.is_null() {
                return Some(Dl { h });
            }
        }
        None
    }
    fn sym(&self, name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        unsafe { dlsym(self.h, c.as_ptr()) }
    }
}

const RTLD_NOW: c_int = 2;

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

macro_rules! fp {
    ($dl:expr, $name:literal, $ty:ty) => {{
        let p = $dl.sym($name);
        if p.is_null() {
            eprintln!("husk-rd: missing symbol {}", $name);
            std::process::exit(3);
        }
        unsafe { std::mem::transmute::<*mut c_void, $ty>(p) }
    }};
}

struct Gio {
    bus_get_sync: unsafe extern "C" fn(c_uint, *mut c_void, *mut *mut c_void) -> Conn,
    call_sync: unsafe extern "C" fn(
        Conn,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        GV,
        *const c_char,
        c_uint,
        c_int,
        *mut c_void,
        *mut *mut c_void,
    ) -> GV,
    v_new_uint32: unsafe extern "C" fn(c_uint) -> GV,
    v_new_int32: unsafe extern "C" fn(c_int) -> GV,
    v_new_bool: unsafe extern "C" fn(c_int) -> GV,
    v_new_double: unsafe extern "C" fn(f64) -> GV,
    v_new_string: unsafe extern "C" fn(*const c_char) -> GV,
    v_new_tuple: unsafe extern "C" fn(*const GV, usize) -> GV,
    v_child: unsafe extern "C" fn(GV, usize) -> GV,
    v_new_array: unsafe extern "C" fn(*const c_void, *const GV, usize) -> GV,
    v_new_dict_entry: unsafe extern "C" fn(GV, GV) -> GV,
    v_get_uint32: unsafe extern "C" fn(GV) -> c_uint,
    v_new_variant: unsafe extern "C" fn(GV) -> GV,
    v_type_new: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    sig_subscribe: unsafe extern "C" fn(
        Conn,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        c_uint,
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            GV,
            *mut c_void,
        ),
        *mut c_void,
        *mut c_void,
    ) -> c_uint,
    main_context_default: unsafe extern "C" fn() -> *mut c_void,
    main_context_iteration: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    v_get_string: unsafe extern "C" fn(GV, *mut usize) -> *const c_char,
    v_unref: unsafe extern "C" fn(GV),
    obj_unref: unsafe extern "C" fn(*mut c_void),
    err_free: unsafe extern "C" fn(*mut c_void),
}

impl Gio {
    fn load() -> Gio {
        let dl = match Dl::open() {
            Some(d) => d,
            None => {
                eprintln!("husk-rd: cannot dlopen libgio: {:?}", unsafe {
                    CStr::from_ptr(dlerror())
                });
                std::process::exit(3);
            }
        };
        let gio = Gio {
            bus_get_sync: fp!(
                dl,
                "g_bus_get_sync",
                unsafe extern "C" fn(c_uint, *mut c_void, *mut *mut c_void) -> Conn
            ),
            call_sync: fp!(
                dl,
                "g_dbus_connection_call_sync",
                unsafe extern "C" fn(
                    Conn,
                    *const c_char,
                    *const c_char,
                    *const c_char,
                    *const c_char,
                    GV,
                    *const c_char,
                    c_uint,
                    c_int,
                    *mut c_void,
                    *mut *mut c_void,
                ) -> GV
            ),
            v_new_uint32: fp!(
                dl,
                "g_variant_new_uint32",
                unsafe extern "C" fn(c_uint) -> GV
            ),
            v_new_int32: fp!(dl, "g_variant_new_int32", unsafe extern "C" fn(c_int) -> GV),
            v_new_bool: fp!(
                dl,
                "g_variant_new_boolean",
                unsafe extern "C" fn(c_int) -> GV
            ),
            v_new_double: fp!(dl, "g_variant_new_double", unsafe extern "C" fn(f64) -> GV),
            v_new_string: fp!(
                dl,
                "g_variant_new_string",
                unsafe extern "C" fn(*const c_char) -> GV
            ),
            v_new_tuple: fp!(
                dl,
                "g_variant_new_tuple",
                unsafe extern "C" fn(*const GV, usize) -> GV
            ),
            v_child: fp!(
                dl,
                "g_variant_get_child_value",
                unsafe extern "C" fn(GV, usize) -> GV
            ),
            v_new_array: fp!(
                dl,
                "g_variant_new_array",
                unsafe extern "C" fn(*const c_void, *const GV, usize) -> GV
            ),
            v_new_dict_entry: fp!(
                dl,
                "g_variant_new_dict_entry",
                unsafe extern "C" fn(GV, GV) -> GV
            ),
            v_get_uint32: fp!(
                dl,
                "g_variant_get_uint32",
                unsafe extern "C" fn(GV) -> c_uint
            ),
            v_new_variant: fp!(dl, "g_variant_new_variant", unsafe extern "C" fn(GV) -> GV),
            v_type_new: fp!(
                dl,
                "g_variant_type_new",
                unsafe extern "C" fn(*const c_char) -> *mut c_void
            ),
            sig_subscribe: fp!(
                dl,
                "g_dbus_connection_signal_subscribe",
                unsafe extern "C" fn(
                    Conn,
                    *const c_char,
                    *const c_char,
                    *const c_char,
                    *const c_char,
                    *const c_char,
                    c_uint,
                    unsafe extern "C" fn(
                        *mut c_void,
                        *const c_char,
                        *const c_char,
                        *const c_char,
                        *const c_char,
                        GV,
                        *mut c_void,
                    ),
                    *mut c_void,
                    *mut c_void,
                ) -> c_uint
            ),
            main_context_default: fp!(
                dl,
                "g_main_context_default",
                unsafe extern "C" fn() -> *mut c_void
            ),
            main_context_iteration: fp!(
                dl,
                "g_main_context_iteration",
                unsafe extern "C" fn(*mut c_void, c_int) -> c_int
            ),
            v_get_string: fp!(
                dl,
                "g_variant_get_string",
                unsafe extern "C" fn(GV, *mut usize) -> *const c_char
            ),
            v_unref: fp!(dl, "g_variant_unref", unsafe extern "C" fn(GV)),
            obj_unref: fp!(dl, "g_object_unref", unsafe extern "C" fn(*mut c_void)),
            err_free: fp!(dl, "g_error_free", unsafe extern "C" fn(*mut c_void)),
        };
        let _ = FNS.set(Fns {
            v_child: gio.v_child,
            v_get_uint32: gio.v_get_uint32,
            v_unref: gio.v_unref,
        });
        gio
    }

    fn bus(&self) -> Conn {
        let mut err: *mut c_void = std::ptr::null_mut();
        // G_BUS_TYPE_SESSION = 2
        let c = unsafe { (self.bus_get_sync)(2, std::ptr::null_mut(), &mut err) };
        if c.is_null() {
            let msg = self.take_err(err);
            eprintln!("husk-rd: no session bus: {msg}");
            std::process::exit(4);
        }
        c
    }

    fn take_err(&self, err: *mut c_void) -> String {
        if err.is_null() {
            return "unknown".into();
        }
        #[repr(C)]
        struct GError {
            domain: u32,
            code: i32,
            message: *mut c_char,
        }
        let s = unsafe { CStr::from_ptr((*(err as *const GError)).message) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.err_free)(err) };
        s
    }

    /// One blocking D-Bus call. `args` are GVariant floats (already built).
    fn call(
        &self,
        conn: Conn,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: &[GV],
    ) -> Result<GV, String> {
        let (cd, cp, ci, cm) = (
            CString::new(dest).unwrap(),
            CString::new(path).unwrap(),
            CString::new(iface).unwrap(),
            CString::new(method).unwrap(),
        );
        let params = if args.is_empty() {
            std::ptr::null_mut()
        } else {
            unsafe { (self.v_new_tuple)(args.as_ptr(), args.len()) }
        };
        let mut err: *mut c_void = std::ptr::null_mut();
        let r = unsafe {
            (self.call_sync)(
                conn,
                cd.as_ptr(),
                cp.as_ptr(),
                ci.as_ptr(),
                cm.as_ptr(),
                params,
                std::ptr::null(), // let GIO infer the reply type
                0,                // no flags
                -1,               // default timeout
                std::ptr::null_mut(),
                &mut err,
            )
        };
        if r.is_null() {
            return Err(self.take_err(err));
        }
        Ok(r)
    }

    /// Reply is "(o)" -> object path string.
    fn reply_path(&self, reply: GV) -> Option<String> {
        let child = unsafe { (self.v_child)(reply, 0) };
        let mut len: usize = 0;
        let p = unsafe { (self.v_get_string)(child, &mut len) };
        let s = if p.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        };
        unsafe { (self.v_unref)(child) };
        s
    }
}

const RD: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const RD_IFACE: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_SESSION_IFACE: &str = "org.gnome.Mutter.RemoteDesktop.Session";

const SC: &str = "org.gnome.Mutter.ScreenCast";
const SC_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SC_IFACE: &str = "org.gnome.Mutter.ScreenCast";
const SC_SESSION_IFACE: &str = "org.gnome.Mutter.ScreenCast.Session";
const SC_STREAM_IFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: husk-rd <selftest|inject <kind> <a> <b>|rd <socket> [--count N]>");
        std::process::exit(2);
    }
    let gio = Gio::load();
    match args[1].as_str() {
        "selftest" => selftest(&gio),
        "inject" => inject(&gio, &args[2..]),
        "probe" => probe(&gio),
        "sc" => screencast(&gio, &args[2..]),
        "rd" => bridge(&gio, &args[2..]),
        other => {
            eprintln!("husk-rd: unknown command {other}");
            std::process::exit(2);
        }
    }
}

fn selftest(gio: &Gio) {
    let bus = gio.bus();
    match gio.call(bus, RD, RD_PATH, RD_IFACE, "CreateSession", &[]) {
        Ok(r) => {
            let p = gio.reply_path(r).unwrap_or_default();
            unsafe { (gio.v_unref)(r) };
            println!("husk-rd|rd|session={p}");
            // A second call on the same path proves the object exists while this
            // connection is alive.
            if let Err(e) = gio.call(bus, RD, &p, RD_SESSION_IFACE, "Start", &[]) {
                println!("husk-rd|rd|start-failed|{e}");
            } else {
                println!("husk-rd|rd|start=ok");
            }
        }
        Err(e) => println!("husk-rd|rd|create-failed|{e}"),
    }
    unsafe { (gio.obj_unref)(bus) };
}

fn inject(gio: &Gio, a: &[String]) {
    if a.len() < 3 {
        eprintln!("usage: husk-rd inject <motion|button|key|axis> <a> <b>");
        std::process::exit(2);
    }
    let bus = gio.bus();
    let r = match gio.call(bus, RD, RD_PATH, RD_IFACE, "CreateSession", &[]) {
        Ok(r) => r,
        Err(e) => {
            println!("husk-rd|inject|create-failed|{e}");
            return;
        }
    };
    let path = gio.reply_path(r).unwrap_or_default();
    unsafe { (gio.v_unref)(r) };
    if let Err(e) = gio.call(bus, RD, &path, RD_SESSION_IFACE, "Start", &[]) {
        println!("husk-rd|inject|start-failed|{e}");
        return;
    }
    let (kind, x, y) = (
        a[0].as_str(),
        a[1].parse::<f64>().unwrap_or(0.0),
        a[2].parse::<i32>().unwrap_or(0),
    );
    let res = match kind {
        "motion" => gio.call(
            bus,
            RD,
            &path,
            RD_SESSION_IFACE,
            "NotifyPointerMotionRelative",
            &[unsafe { (gio.v_new_double)(x) }, unsafe {
                (gio.v_new_double)(y as f64)
            }],
        ),
        "button" => gio.call(
            bus,
            RD,
            &path,
            RD_SESSION_IFACE,
            "NotifyPointerButton",
            &[unsafe { (gio.v_new_int32)(x as i32) }, unsafe {
                (gio.v_new_bool)(y as i32)
            }],
        ),
        "key" => gio.call(
            bus,
            RD,
            &path,
            RD_SESSION_IFACE,
            "NotifyKeyboardKeycode",
            &[unsafe { (gio.v_new_uint32)(x as u32) }, unsafe {
                (gio.v_new_bool)(y as i32)
            }],
        ),
        "axis" => gio.call(
            bus,
            RD,
            &path,
            RD_SESSION_IFACE,
            "NotifyPointerAxisDiscrete",
            &[unsafe { (gio.v_new_uint32)(x as u32) }, unsafe {
                (gio.v_new_int32)(y as i32)
            }],
        ),
        other => {
            eprintln!("unknown kind {other}");
            std::process::exit(2);
        }
    };
    match res {
        Ok(r) => {
            unsafe { (gio.v_unref)(r) };
            println!("husk-rd|inject|{kind}|ok");
        }
        Err(e) => println!("husk-rd|inject|{kind}|failed|{e}"),
    }
    unsafe { (gio.obj_unref)(bus) };
}

struct Fns {
    v_child: unsafe extern "C" fn(GV, usize) -> GV,
    v_get_uint32: unsafe extern "C" fn(GV) -> c_uint,
    v_unref: unsafe extern "C" fn(GV),
}
static FNS: std::sync::OnceLock<Fns> = std::sync::OnceLock::new();

unsafe extern "C" fn on_signal(
    _conn: *mut c_void,
    _sender: *const c_char,
    object_path: *const c_char,
    _iface: *const c_char,
    member: *const c_char,
    params: GV,
    _ud: *mut c_void,
) {
    let path = if object_path.is_null() {
        String::new()
    } else {
        CStr::from_ptr(object_path).to_string_lossy().into_owned()
    };
    let mem = if member.is_null() {
        String::new()
    } else {
        CStr::from_ptr(member).to_string_lossy().into_owned()
    };
    // PipeWireStreamAdded(u node_id): read child 0 as uint32 through the
    // function table is not available in this plain fn, so decode via the
    // statically stored fn pointers captured at load time.
    let node = match FNS.get() {
        Some(f) => unsafe {
            let c = (f.v_child)(params, 0);
            let n = (f.v_get_uint32)(c);
            (f.v_unref)(c);
            Some(n)
        },
        None => None,
    };
    println!("husk-rd|signal|{mem}|{path}|node={node:?}");
    std::io::Write::flush(&mut std::io::stdout()).ok();
}

/// Start a virtual monitor stream and keep the session alive.
fn screencast(gio: &Gio, a: &[String]) {
    let secs: u64 = a.first().and_then(|v| v.parse().ok()).unwrap_or(30);
    let bus = gio.bus();

    // CreateSession with disable-animations = true
    let key = unsafe { (gio.v_new_string)(CString::new("disable-animations").unwrap().as_ptr()) };
    let val = unsafe { (gio.v_new_variant)((gio.v_new_bool)(1)) };
    let entry = unsafe { (gio.v_new_dict_entry)(key, val) };
    // g_variant_new_array takes the *child* type: an entry type, not a{sv}.
    let at = unsafe { (gio.v_type_new)(CString::new("{sv}").unwrap().as_ptr()) };
    let dict = unsafe { (gio.v_new_array)(at, &entry, 1) };
    let r = match gio.call(bus, SC, SC_PATH, SC_IFACE, "CreateSession", &[dict]) {
        Ok(r) => r,
        Err(e) => {
            println!("husk-rd|sc|create-session-failed|{e}");
            return;
        }
    };
    let ses = gio.reply_path(r).unwrap_or_default();
    unsafe { (gio.v_unref)(r) };
    println!("husk-rd|sc|session|{ses}");

    // RecordVirtual: cursor-mode=2 (metadata), is-platform=true
    let k1 = unsafe { (gio.v_new_string)(CString::new("cursor-mode").unwrap().as_ptr()) };
    let v1 = unsafe { (gio.v_new_variant)((gio.v_new_uint32)(2)) };
    let e1 = unsafe { (gio.v_new_dict_entry)(k1, v1) };
    let k2 = unsafe { (gio.v_new_string)(CString::new("is-platform").unwrap().as_ptr()) };
    let v2 = unsafe { (gio.v_new_variant)((gio.v_new_bool)(1)) };
    let e2 = unsafe { (gio.v_new_dict_entry)(k2, v2) };
    let dict2 = unsafe { (gio.v_new_array)(at, [e1, e2].as_ptr(), 2) };
    let r = match gio.call(bus, SC, &ses, SC_SESSION_IFACE, "RecordVirtual", &[dict2]) {
        Ok(r) => r,
        Err(e) => {
            println!("husk-rd|sc|record-virtual-failed|{e}");
            return;
        }
    };
    let stream = gio.reply_path(r).unwrap_or_default();
    unsafe { (gio.v_unref)(r) };
    println!("husk-rd|sc|stream|{stream}");

    // subscribe before Start so we cannot miss the signal
    let iface = CString::new("org.gnome.Mutter.ScreenCast.Stream").unwrap();
    let member = CString::new("PipeWireStreamAdded").unwrap();
    let opath = CString::new(stream.clone()).unwrap();
    unsafe {
        (gio.sig_subscribe)(
            bus,
            std::ptr::null(),
            iface.as_ptr(),
            member.as_ptr(),
            opath.as_ptr(),
            std::ptr::null(),
            0,
            on_signal,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }

    // Session.Start() also starts its streams, so the PipeWire signal arrives
    // during this call -- subscribe before it. Stream.Start() after that only
    // returns "Stream already started".
    if let Err(e) = gio.call(bus, SC, &ses, SC_SESSION_IFACE, "Start", &[]) {
        println!("husk-rd|sc|session-start-failed|{e}");
        return;
    }
    println!("husk-rd|sc|started");

    // hold the session; dispatch D-Bus signals in the default main context
    let ctx = unsafe { (gio.main_context_default)() };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        unsafe { (gio.main_context_iteration)(ctx, 0) };
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = gio.call(bus, SC, &ses, SC_SESSION_IFACE, "Stop", &[]);
    println!("husk-rd|sc|stopped");
    unsafe { (gio.obj_unref)(bus) };
}

/// One connection, one session, a whole press/release cycle. This is the only
/// way to see the state checks: a release in a *new* session is rejected as an
/// "Invalid key event" because that session never saw the press.
fn probe(gio: &Gio) {
    let bus = gio.bus();
    let r = match gio.call(bus, RD, RD_PATH, RD_IFACE, "CreateSession", &[]) {
        Ok(r) => r,
        Err(e) => {
            println!("husk-rd|probe|create-failed|{e}");
            return;
        }
    };
    let path = gio.reply_path(r).unwrap_or_default();
    unsafe { (gio.v_unref)(r) };
    if let Err(e) = gio.call(bus, RD, &path, RD_SESSION_IFACE, "Start", &[]) {
        println!("husk-rd|probe|start-failed|{e}");
        return;
    }
    println!("husk-rd|probe|session|{path}");
    let call = |m: &str, args: &[GV]| -> String {
        match gio.call(bus, RD, &path, RD_SESSION_IFACE, m, args) {
            Ok(r) => {
                unsafe { (gio.v_unref)(r) };
                "ok".into()
            }
            Err(e) => e,
        }
    };
    unsafe {
        println!(
            "motion+  {}",
            call(
                "NotifyPointerMotionRelative",
                &[(gio.v_new_double)(20.0), (gio.v_new_double)(10.0)]
            )
        );
        println!(
            "motion-  {}",
            call(
                "NotifyPointerMotionRelative",
                &[(gio.v_new_double)(-5.0), (gio.v_new_double)(-4.0)]
            )
        );
        println!(
            "btn-down {}",
            call(
                "NotifyPointerButton",
                &[(gio.v_new_int32)(0x110), (gio.v_new_bool)(1)]
            )
        );
        println!(
            "btn-up   {}",
            call(
                "NotifyPointerButton",
                &[(gio.v_new_int32)(0x110), (gio.v_new_bool)(0)]
            )
        );
        println!(
            "key-down {}",
            call(
                "NotifyKeyboardKeycode",
                &[(gio.v_new_uint32)(30), (gio.v_new_bool)(1)]
            )
        );
        println!(
            "key-up   {}",
            call(
                "NotifyKeyboardKeycode",
                &[(gio.v_new_uint32)(30), (gio.v_new_bool)(0)]
            )
        );
        println!(
            "axis     {}",
            call(
                "NotifyPointerAxisDiscrete",
                &[(gio.v_new_uint32)(0), (gio.v_new_int32)(1)]
            )
        );
    }
    let _ = gio.call(bus, RD, &path, RD_SESSION_IFACE, "Stop", &[]);
    unsafe { (gio.obj_unref)(bus) };
}

/// Consume the app's evdev-record stream and inject it into mutter. One session
/// per connection, kept alive for as long as the socket is.
fn bridge(gio: &Gio, a: &[String]) {
    if a.is_empty() {
        eprintln!("usage: husk-rd rd <socket> [--count N]");
        std::process::exit(2);
    }
    let socket = &a[0];
    let stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("husk-rd|FAIL|connect {socket}: {e}");
            std::process::exit(1);
        }
    };
    let bus = gio.bus();
    let r = match gio.call(bus, RD, RD_PATH, RD_IFACE, "CreateSession", &[]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("husk-rd|FAIL|create-session: {e}");
            std::process::exit(1);
        }
    };
    let path = gio.reply_path(r).unwrap_or_default();
    unsafe { (gio.v_unref)(r) };
    if let Err(e) = gio.call(bus, RD, &path, RD_SESSION_IFACE, "Start", &[]) {
        eprintln!("husk-rd|FAIL|start: {e}");
        std::process::exit(1);
    }
    println!("husk-rd|session|{path}");

    // header line, then 24-byte evdev records (same wire format as husk-input)
    let mut tmp = [0u8; 4096];
    let mut s = stream;
    // read header byte by byte (cheap, once)
    let mut header = Vec::new();
    loop {
        let mut b = [0u8; 1];
        match s.read(&mut b) {
            Ok(1) => {
                if b[0] == b'\n' {
                    break;
                }
                header.push(b[0]);
            }
            _ => {
                eprintln!("husk-rd|FAIL|header");
                std::process::exit(1);
            }
        }
    }
    println!("husk-rd|header|{}", String::from_utf8_lossy(&header));

    let mut left: i32 = 1; // last ABS_X
    let mut top: i32 = 1; // last ABS_Y
    let mut have = false;
    let mut pending = Vec::<u8>::new();
    let mut events = 0u64;
    loop {
        let n = match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        pending.extend_from_slice(&tmp[..n]);
        while pending.len() >= 24 {
            let rec: Vec<u8> = pending.drain(..24).collect();
            let ty = u16::from_ne_bytes([rec[16], rec[17]]);
            let code = u16::from_ne_bytes([rec[18], rec[19]]);
            let val = i32::from_ne_bytes([rec[20], rec[21], rec[22], rec[23]]);
            if ty == 3 && (code == 0 || code == 1) {
                // ABS_X / ABS_Y in 0..32767 -> relative pixels on a 1080x2400 surface
                let (span, last) = if code == 0 {
                    (1080i64, &mut left)
                } else {
                    (2400i64, &mut top)
                };
                let px = (val as i64 * span / 32767) as i32;
                let d = px - *last;
                *last = px;
                if have && d != 0 {
                    let dbl = if code == 0 {
                        (d as f64, 0.0)
                    } else {
                        (0.0, d as f64)
                    };
                    let _ = gio.call(
                        bus,
                        RD,
                        &path,
                        RD_SESSION_IFACE,
                        "NotifyPointerMotionRelative",
                        &[unsafe { (gio.v_new_double)(dbl.0) }, unsafe {
                            (gio.v_new_double)(dbl.1)
                        }],
                    );
                    events += 1;
                }
                if code == 1 {
                    have = true;
                }
            } else if ty == 1 {
                // EV_KEY: BTN_LEFT/RIGHT/MIDDLE or a keyboard keycode
                // mutter wants *evdev* button codes (BTN_LEFT=0x110), not X11
                // numbers: meta_evdev_button_to_clutter asserts button > BTN_LEFT.
                let btn = match code {
                    0x110 | 0x111 | 0x112 | 0x113 | 0x114 | 0x115 => Some(code),
                    _ => None,
                };
                let res = match btn {
                    Some(b) => gio.call(
                        bus,
                        RD,
                        &path,
                        RD_SESSION_IFACE,
                        "NotifyPointerButton",
                        &[unsafe { (gio.v_new_int32)(b as i32) }, unsafe {
                            (gio.v_new_bool)(val)
                        }],
                    ),
                    None => gio.call(
                        bus,
                        RD,
                        &path,
                        RD_SESSION_IFACE,
                        "NotifyKeyboardKeycode",
                        &[unsafe { (gio.v_new_uint32)(code as u32) }, unsafe {
                            (gio.v_new_bool)(val)
                        }],
                    ),
                };
                if res.is_ok() {
                    events += 1;
                } else if let Err(e) = res {
                    println!("husk-rd|inject|failed|{e}");
                }
            } else if ty == 2 && code == 8 {
                // REL_WHEEL -> discrete axis, axis 0 = vertical
                let _ = gio.call(
                    bus,
                    RD,
                    &path,
                    RD_SESSION_IFACE,
                    "NotifyPointerAxisDiscrete",
                    &[unsafe { (gio.v_new_uint32)(0) }, unsafe {
                        (gio.v_new_int32)(-val)
                    }],
                );
                events += 1;
            }
        }
    }
    println!("husk-rd|done|{events}");
    let _ = gio.call(bus, RD, &path, RD_SESSION_IFACE, "Stop", &[]);
    unsafe { (gio.obj_unref)(bus) };
}
