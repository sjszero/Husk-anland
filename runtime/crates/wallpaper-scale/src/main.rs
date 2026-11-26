//! Zoom-fill a wallpaper to exactly the guest screen size through GdkPixbuf.
//!
//! This replaces the former GJS helper. It dlopens the guest's GdkPixbuf and
//! GObject libraries, so no host-side GTK dependency is introduced and the
//! binary remains a normal dynamically-linked Linux guest program.
//!
//! Exit codes match the old helper: 0 = wrote output, 3 = already exact size,
//! other = failure.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

struct GdkPixbufApi {
    _gdk: *mut c_void,
    _gobject: *mut c_void,
    new_from_file: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_void,
    get_width: unsafe extern "C" fn(*const c_void) -> c_int,
    get_height: unsafe extern "C" fn(*const c_void) -> c_int,
    scale_simple: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> *mut c_void,
    new_subpixbuf: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int) -> *mut c_void,
    copy: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    savev: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *mut *mut c_char,
        *mut *mut c_char,
        *mut *mut GError,
    ) -> c_int,
    object_unref: unsafe extern "C" fn(*mut c_void),
    error_free: unsafe extern "C" fn(*mut GError),
}

unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &[u8]) -> Option<T> {
    let address = libc::dlsym(handle, name.as_ptr().cast());
    if address.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&address))
    }
}

impl GdkPixbufApi {
    fn load() -> Option<Self> {
        unsafe {
            let gdk_name = b"libgdk_pixbuf-2.0.so.0\0";
            let gobject_name = b"libgobject-2.0.so.0\0";
            let gdk = libc::dlopen(gdk_name.as_ptr().cast(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            let gobject = libc::dlopen(
                gobject_name.as_ptr().cast(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            );
            if gdk.is_null() || gobject.is_null() {
                return None;
            }
            Some(Self {
                _gdk: gdk,
                _gobject: gobject,
                new_from_file: symbol(gdk, b"gdk_pixbuf_new_from_file\0")?,
                get_width: symbol(gdk, b"gdk_pixbuf_get_width\0")?,
                get_height: symbol(gdk, b"gdk_pixbuf_get_height\0")?,
                scale_simple: symbol(gdk, b"gdk_pixbuf_scale_simple\0")?,
                new_subpixbuf: symbol(gdk, b"gdk_pixbuf_new_subpixbuf\0")?,
                copy: symbol(gdk, b"gdk_pixbuf_copy\0")?,
                savev: symbol(gdk, b"gdk_pixbuf_savev\0")?,
                object_unref: symbol(gobject, b"g_object_unref\0")?,
                error_free: symbol(gobject, b"g_error_free\0")?,
            })
        }
    }
}

fn report_error(api: &GdkPixbufApi, context: &str, error: *mut GError) {
    unsafe {
        if error.is_null() {
            eprintln!("wallpaper-scale: {context}");
            return;
        }
        let message = if (*error).message.is_null() {
            "unknown GError".into()
        } else {
            CStr::from_ptr((*error).message)
                .to_string_lossy()
                .into_owned()
        };
        eprintln!("wallpaper-scale: {context}: {message}");
        (api.error_free)(error);
    }
}

fn usage() -> i32 {
    eprintln!("usage: wallpaper-scale SRC DST W H");
    64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        std::process::exit(usage());
    }
    let (src, dst) = (&args[1], &args[2]);
    let Ok(width) = args[3].parse::<i32>() else {
        std::process::exit(usage());
    };
    let Ok(height) = args[4].parse::<i32>() else {
        std::process::exit(usage());
    };
    if width <= 0 || height <= 0 {
        std::process::exit(usage());
    }
    let Ok(src_c) = CString::new(src.as_bytes()) else {
        std::process::exit(64);
    };
    let Ok(dst_c) = CString::new(dst.as_bytes()) else {
        std::process::exit(64);
    };

    let Some(api) = GdkPixbufApi::load() else {
        eprintln!("wallpaper-scale: cannot load GdkPixbuf/GObject");
        std::process::exit(1);
    };

    let mut error: *mut GError = ptr::null_mut();
    let source = unsafe { (api.new_from_file)(src_c.as_ptr(), &mut error) };
    if source.is_null() {
        report_error(&api, "cannot open source image", error);
        std::process::exit(1);
    }

    let source_width = unsafe { (api.get_width)(source) };
    let source_height = unsafe { (api.get_height)(source) };
    if source_width == width && source_height == height {
        unsafe { (api.object_unref)(source) };
        std::process::exit(3);
    }

    let scale = f64::max(
        width as f64 / source_width.max(1) as f64,
        height as f64 / source_height.max(1) as f64,
    );
    let scaled_width = width.max((source_width as f64 * scale).round() as i32);
    let scaled_height = height.max((source_height as f64 * scale).round() as i32);
    let scaled = unsafe { (api.scale_simple)(source, scaled_width, scaled_height, 2) };
    unsafe { (api.object_unref)(source) };
    if scaled.is_null() {
        eprintln!("wallpaper-scale: gdk_pixbuf_scale_simple failed");
        std::process::exit(1);
    }

    let x = (scaled_width - width) / 2;
    let y = (scaled_height - height) / 2;
    let cropped = unsafe { (api.new_subpixbuf)(scaled, x, y, width, height) };
    unsafe { (api.object_unref)(scaled) };
    if cropped.is_null() {
        eprintln!("wallpaper-scale: gdk_pixbuf_new_subpixbuf failed");
        std::process::exit(1);
    }
    let detached = unsafe { (api.copy)(cropped) };
    unsafe { (api.object_unref)(cropped) };
    if detached.is_null() {
        eprintln!("wallpaper-scale: gdk_pixbuf_copy failed");
        std::process::exit(1);
    }

    let png = b"png\0";
    let saved = unsafe {
        (api.savev)(
            detached,
            dst_c.as_ptr(),
            png.as_ptr().cast(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut error,
        )
    };
    unsafe { (api.object_unref)(detached) };
    if saved == 0 {
        report_error(&api, "cannot save destination image", error);
        std::process::exit(1);
    }
}
