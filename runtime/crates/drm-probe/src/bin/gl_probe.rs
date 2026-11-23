//! gl_probe — 让 **真的 Mesa** 走我们的假 DRM 节点。
//!
//! 纯 Rust：dlopen libEGL.so.1 / libGLESv2.so.2，走 EGL surfaceless + GLES2，
//! 把 EGL 的 VENDOR / VERSION 和 GL 的 VENDOR / RENDERER / VERSION 打出来。
//!
//! 判据：RENDERER 里出现 `Mali`（说明 panfrost 真的在 /dev/mali0 上起来了），
//! 而不是 `llvmpipe` / `softpipe`。

use std::ffi::CString;

type CInt = i32;
type Ptr = *mut core::ffi::c_void;

extern "C" {
    fn dlopen(filename: *const u8, flag: CInt) -> Ptr;
    fn dlsym(handle: Ptr, symbol: *const u8) -> Ptr;
    fn dlerror() -> *const u8;
}

fn line(n: &str, v: &str, d: impl AsRef<str>) {
    println!("gl|{}|{}|{}", n, v, d.as_ref());
}

unsafe fn load(name: &str) -> Option<Ptr> {
    let c = CString::new(name).ok()?;
    let h = dlopen(c.as_ptr(), 2); // RTLD_NOW
    if h.is_null() {
        let e = dlerror();
        let msg = if e.is_null() {
            "?".to_string()
        } else {
            std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
        };
        line("dlopen", "FAIL", format!("{name}: {msg}"));
        return None;
    }
    line("dlopen", "PASS", name);
    Some(h)
}

unsafe fn sym(h: Ptr, name: &str) -> Option<Ptr> {
    let c = CString::new(name).ok()?;
    let p = dlsym(h, c.as_ptr());
    if p.is_null() {
        line("dlsym", "FAIL", name);
        None
    } else {
        Some(p)
    }
}

macro_rules! get_fn {
    ($h:expr, $name:expr, $ty:ty) => {{
        match unsafe { sym($h, $name) } {
            Some(p) => Some(unsafe { core::mem::transmute::<Ptr, $ty>(p) }),
            None => None,
        }
    }};
}

const EGL_DEFAULT_DISPLAY: Ptr = core::ptr::null_mut();
const EGL_NO_CONTEXT: Ptr = core::ptr::null_mut();
const EGL_NO_SURFACE: Ptr = core::ptr::null_mut();
const EGL_TRUE: CInt = 1;
const EGL_NONE: CInt = 0x3038;
const EGL_VENDOR: CInt = 0x3053;
const EGL_VERSION: CInt = 0x3054;
const EGL_EXTENSIONS: CInt = 0x3055;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_SURFACE_TYPE: CInt = 0x3033;
const EGL_PBUFFER_BIT: CInt = 0x0001;
const EGL_RENDERABLE_TYPE: CInt = 0x3040;
const EGL_OPENGL_ES2_BIT: CInt = 0x0004;
const EGL_CONTEXT_CLIENT_VERSION: CInt = 0x3098;
const GL_VENDOR: u32 = 0x1F00;
const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;

fn cstr(p: *const u8) -> String {
    if p.is_null() {
        return "<null>".into();
    }
    unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn main() {
    unsafe {
        // 优先直接用 Mesa 的 libEGL（glvnd 的 libEGL.so.1 只是个分发壳，要 ICD json）
        let egl = match load("libEGL.so.1").or_else(|| load("libEGL_mesa.so.0")) {
            Some(h) => h,
            None => {
                line("verdict", "FAIL", "没有 libEGL");
                return;
            }
        };

        type GetDisplay = unsafe extern "C" fn(Ptr) -> Ptr;
        type Initialize = unsafe extern "C" fn(Ptr, *mut CInt, *mut CInt) -> CInt;
        type QueryString = unsafe extern "C" fn(Ptr, CInt) -> *const u8;
        type BindApi = unsafe extern "C" fn(u32) -> CInt;
        type ChooseConfig =
            unsafe extern "C" fn(Ptr, *const CInt, *mut Ptr, CInt, *mut CInt) -> CInt;
        type CreateContext = unsafe extern "C" fn(Ptr, Ptr, Ptr, *const CInt) -> Ptr;
        type MakeCurrent = unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> CInt;
        type GetError = unsafe extern "C" fn() -> CInt;

        let get_display = match get_fn!(egl, "eglGetDisplay", GetDisplay) {
            Some(f) => f,
            None => return,
        };
        let initialize = match get_fn!(egl, "eglInitialize", Initialize) {
            Some(f) => f,
            None => return,
        };
        let query_string = match get_fn!(egl, "eglQueryString", QueryString) {
            Some(f) => f,
            None => return,
        };
        let get_error = get_fn!(egl, "eglGetError", GetError);

        // 优先走 eglGetPlatformDisplayEXT(SURFACELESS_MESA)：不需要 X/Wayland。
        type GetProcAddr = unsafe extern "C" fn(*const u8) -> Ptr;
        type GetPlatformDisplay = unsafe extern "C" fn(u32, Ptr, *const CInt) -> Ptr;
        const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
        let mut dpy: Ptr = core::ptr::null_mut();
        if let Some(gpa) = get_fn!(egl, "eglGetProcAddress", GetProcAddr) {
            let name = CString::new("eglGetPlatformDisplayEXT").unwrap();
            let f = gpa(name.as_ptr());
            if !f.is_null() {
                let gpd: GetPlatformDisplay = core::mem::transmute(f);
                let pd = gpd(
                    EGL_PLATFORM_SURFACELESS_MESA,
                    core::ptr::null_mut(),
                    core::ptr::null(),
                );
                line(
                    "get_platform_display",
                    if pd.is_null() { "FAIL" } else { "PASS" },
                    format!("surfaceless {pd:p}"),
                );
                dpy = pd;
            }
        }
        // 路线 A（首选）：gbm_create_device(真 fd) → EGL_PLATFORM_GBM。
        // 这是 Linux/Android 上所有 GPU 程序的标准入口，Mesa 会用它拿 drm fd 选驱动。
        const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
        let no_gbm = std::env::var_os("GL_PROBE_NO_GBM").is_some();
        if !no_gbm {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/dri/renderD128");
            match f {
                Ok(file) => {
                    use std::os::fd::AsRawFd;
                    let fd = file.as_raw_fd();
                    line("open_render", "PASS", format!("fd={fd}"));
                    if let Some(gbm) = load("libgbm.so.1") {
                        type GbmCreate = unsafe extern "C" fn(CInt) -> Ptr;
                        if let Some(create) = get_fn!(gbm, "gbm_create_device", GbmCreate) {
                            let dev = create(fd);
                            line(
                                "gbm_create_device",
                                if dev.is_null() { "FAIL" } else { "PASS" },
                                format!("{dev:p}"),
                            );
                            if !dev.is_null() {
                                if let Some(gpa) = get_fn!(egl, "eglGetProcAddress", GetProcAddr) {
                                    let dn = CString::new("eglGetPlatformDisplayEXT").unwrap();
                                    let dp = gpa(dn.as_ptr());
                                    if !dp.is_null() {
                                        let gpd: GetPlatformDisplay = core::mem::transmute(dp);
                                        let pd = gpd(EGL_PLATFORM_GBM_KHR, dev, core::ptr::null());
                                        line(
                                            "gbm_display",
                                            if pd.is_null() { "FAIL" } else { "PASS" },
                                            format!("{pd:p}"),
                                        );
                                        dpy = pd;
                                    }
                                }
                            }
                        }
                    }
                    // 让 fd 活着到进程结束
                    core::mem::forget(file);
                }
                Err(e) => line(
                    "open_render",
                    "FAIL",
                    format!("errno={}", e.raw_os_error().unwrap_or(0)),
                ),
            }
        }

        // 路线 B（备选）：EGL_EXT_device 枚举
        const EGL_PLATFORM_DEVICE_EXT: u32 = 0x313F;
        if let Some(gpa) = get_fn!(egl, "eglGetProcAddress", GetProcAddr) {
            type QueryDevices = unsafe extern "C" fn(CInt, *mut Ptr, *mut CInt) -> CInt;
            let qn = CString::new("eglQueryDevicesEXT").unwrap();
            let qp = gpa(qn.as_ptr());
            if !qp.is_null() {
                let qd: QueryDevices = core::mem::transmute(qp);
                let mut devs = [core::ptr::null_mut(); 8];
                let mut cnt: CInt = 0;
                let ok = qd(8, devs.as_mut_ptr(), &mut cnt);
                line(
                    "query_devices",
                    if ok == EGL_TRUE && cnt > 0 {
                        "PASS"
                    } else {
                        "FAIL"
                    },
                    format!("n={cnt}"),
                );
                if ok == EGL_TRUE && cnt > 0 {
                    let dn = CString::new("eglGetPlatformDisplayEXT").unwrap();
                    let dp = gpa(dn.as_ptr());
                    if !dp.is_null() {
                        let gpd: GetPlatformDisplay = core::mem::transmute(dp);
                        let pd = gpd(EGL_PLATFORM_DEVICE_EXT, devs[0], core::ptr::null());
                        line(
                            "device_display",
                            if pd.is_null() { "FAIL" } else { "PASS" },
                            format!("{pd:p}"),
                        );
                        dpy = pd;
                    }
                }
            }
        }
        if dpy.is_null() {
            dpy = get_display(EGL_DEFAULT_DISPLAY);
            line(
                "get_display",
                if dpy.is_null() { "FAIL" } else { "PASS" },
                format!("{dpy:p}"),
            );
        }
        if dpy.is_null() {
            if let Some(ge) = get_error {
                line("egl_error", "INFO", format!("{:#x}", ge()));
            }
            return;
        }

        let (mut maj, mut min) = (0 as CInt, 0 as CInt);
        let ok = initialize(dpy, &mut maj, &mut min);
        line(
            "initialize",
            if ok == EGL_TRUE { "PASS" } else { "FAIL" },
            format!("EGL {maj}.{min}"),
        );
        if ok != EGL_TRUE {
            if let Some(ge) = get_error {
                line("egl_error", "INFO", format!("{:#x}", ge()));
            }
            return;
        }

        for (name, q) in [("EGL_VENDOR", EGL_VENDOR), ("EGL_VERSION", EGL_VERSION)] {
            let s = query_string(dpy, q);
            line(name, "INFO", cstr(s));
        }
        let ext = cstr(query_string(dpy, EGL_EXTENSIONS));
        line(
            "EGL_EXTENSIONS",
            "INFO",
            format!("{} 项", ext.split(' ').count()),
        );

        // GLES2 上下文，不需要 surface（surfaceless）
        if let Some(bind) = get_fn!(egl, "eglBindAPI", BindApi) {
            line(
                "bind_api",
                if bind(EGL_OPENGL_ES_API) == EGL_TRUE {
                    "PASS"
                } else {
                    "FAIL"
                },
                "ES",
            );
        }
        let choose = match get_fn!(egl, "eglChooseConfig", ChooseConfig) {
            Some(f) => f,
            None => return,
        };
        let create_ctx = match get_fn!(egl, "eglCreateContext", CreateContext) {
            Some(f) => f,
            None => return,
        };
        let make_current = match get_fn!(egl, "eglMakeCurrent", MakeCurrent) {
            Some(f) => f,
            None => return,
        };

        let attribs: [CInt; 7] = [
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_NONE,
            0,
            0,
        ];
        let mut cfg: Ptr = core::ptr::null_mut();
        let mut n: CInt = 0;
        let ok = choose(dpy, attribs.as_ptr(), &mut cfg, 1, &mut n);
        line(
            "choose_config",
            if ok == EGL_TRUE && n > 0 {
                "PASS"
            } else {
                "FAIL"
            },
            format!("n={n}"),
        );
        if ok != EGL_TRUE || n == 0 {
            if let Some(ge) = get_error {
                line("egl_error", "INFO", format!("{:#x}", ge()));
            }
            return;
        }

        let ctx_attribs: [CInt; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let ctx = create_ctx(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
        line(
            "create_context",
            if ctx.is_null() { "FAIL" } else { "PASS" },
            format!("{ctx:p}"),
        );
        if ctx.is_null() {
            if let Some(ge) = get_error {
                line("egl_error", "INFO", format!("{:#x}", ge()));
            }
            return;
        }
        let ok = make_current(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx);
        line(
            "make_current",
            if ok == EGL_TRUE { "PASS" } else { "FAIL" },
            "surfaceless",
        );
        if ok != EGL_TRUE {
            if let Some(ge) = get_error {
                line("egl_error", "INFO", format!("{:#x}", ge()));
            }
            return;
        }

        // GL 的 renderer
        let gl = match load("libGLESv2.so.2") {
            Some(h) => h,
            None => return,
        };
        type GetString = unsafe extern "C" fn(u32) -> *const u8;
        if let Some(gs) = get_fn!(gl, "glGetString", GetString) {
            let vendor = cstr(gs(GL_VENDOR));
            let renderer = cstr(gs(GL_RENDERER));
            let version = cstr(gs(GL_VERSION));
            line("GL_VENDOR", "INFO", vendor.clone());
            line("GL_RENDERER", "INFO", renderer.clone());
            line("GL_VERSION", "INFO", version);
            let verdict = if renderer.to_lowercase().contains("mali") {
                "PASS"
            } else if renderer.to_lowercase().contains("llvmpipe")
                || renderer.to_lowercase().contains("softpipe")
                || renderer.to_lowercase().contains("swrast")
            {
                "SOFTWARE"
            } else {
                "UNKNOWN"
            };
            line("verdict", verdict, format!("renderer={renderer}"));
        }
    }
}
