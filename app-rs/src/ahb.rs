//! One-shot AHardwareBuffer / dma-buf capability probe.
//!
//! This runs inside the real NativeActivity process, not in the glibc guest.
//! The public NDK exposes AHardwareBuffer allocation and Unix-socket handle
//! transfer, while the native handle itself is an implementation ABI:
//!
//!   struct native_handle_t {
//!       int version;
//!       int numFds;
//!       int numInts;
//!       int data[]; // numFds descriptors followed by numInts integers
//!   };
//!
//! We resolve every symbol dynamically so the minimum-SDK-24 APK can still be
//! loaded on devices where the API-26 AHardwareBuffer symbols do not exist.
//! The probe is intentionally read-only with respect to product data: it
//! allocates one small graphics buffer, inspects and transfers it, then closes
//! every descriptor and releases both references.

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RTLD_NOW: c_int = 2;
const AF_UNIX: c_int = 1;
const SOCK_SEQPACKET: c_int = 5;
const F_GETFD: c_int = 1;
const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
const AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER: u64 = 1 << 9;
const AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN: u64 = 1 << 1;
const AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN: u64 = 1 << 5;
const MAX_HANDLE_FDS: c_int = 16;

const PROBE_WIDTH: u32 = 64;
const PROBE_HEIGHT: u32 = 64;

static PROBED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Desc {
    width: u32,
    height: u32,
    layers: u32,
    format: u32,
    usage: u64,
    stride: u32,
    rfu0: u32,
    rfu1: u64,
}

type Ahb = c_void;
type FnAllocate = unsafe extern "C" fn(*const Desc, *mut *mut Ahb) -> c_int;
type FnDescribe = unsafe extern "C" fn(*const Ahb, *mut Desc);
type FnRelease = unsafe extern "C" fn(*mut Ahb);
type FnGetNativeHandle = unsafe extern "C" fn(*const Ahb) -> *const c_void;
type FnSendToSocket = unsafe extern "C" fn(*const Ahb, c_int) -> c_int;
type FnRecvFromSocket = unsafe extern "C" fn(c_int, *mut *mut Ahb) -> c_int;
type FnLock = unsafe extern "C" fn(*mut Ahb, u64, c_int, *const c_void, *mut *mut c_void) -> c_int;
type FnUnlock = unsafe extern "C" fn(*mut Ahb, *mut c_int) -> c_int;

struct Symbols {
    allocate: FnAllocate,
    describe: FnDescribe,
    release: FnRelease,
    native_handle: FnGetNativeHandle,
    send: FnSendToSocket,
    recv: FnRecvFromSocket,
    lock: FnLock,
    unlock: FnUnlock,
}

#[repr(C)]
struct ARect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

type SurfaceControl = *mut c_void;
type SurfaceTransaction = *mut c_void;
type FnCreateSurfaceControl = unsafe extern "C" fn(*mut c_void, *const c_char) -> SurfaceControl;
type FnReleaseSurfaceControl = unsafe extern "C" fn(SurfaceControl);
type FnCreateSurfaceTransaction = unsafe extern "C" fn() -> SurfaceTransaction;
type FnDeleteSurfaceTransaction = unsafe extern "C" fn(SurfaceTransaction);
type FnSetBuffer = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *mut Ahb, c_int);
type FnSetGeometry =
    unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *const ARect, *const ARect, i32);
type FnSetZOrder = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, i32);
type FnApplySurfaceTransaction = unsafe extern "C" fn(SurfaceTransaction);

struct SurfaceSymbols {
    create_from_window: FnCreateSurfaceControl,
    release_control: FnReleaseSurfaceControl,
    create_transaction: FnCreateSurfaceTransaction,
    delete_transaction: FnDeleteSurfaceTransaction,
    set_buffer: FnSetBuffer,
    set_geometry: FnSetGeometry,
    set_z_order: FnSetZOrder,
    apply: FnApplySurfaceTransaction,
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn socketpair(domain: c_int, ty: c_int, protocol: c_int, sv: *mut c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
}

pub fn spawn_once() {
    if PROBED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(probe);
    });
}

unsafe fn load() -> Option<(*mut c_void, Symbols)> {
    let handle = dlopen(c"libandroid.so".as_ptr(), RTLD_NOW);
    if handle.is_null() {
        crate::log("ahb|loader|FAIL|libandroid.so");
        return None;
    }
    crate::log("ahb|loader|PASS|libandroid.so");

    let names = [
        "AHardwareBuffer_allocate",
        "AHardwareBuffer_describe",
        "AHardwareBuffer_release",
        "AHardwareBuffer_getNativeHandle",
        "AHardwareBuffer_sendHandleToUnixSocket",
        "AHardwareBuffer_recvHandleFromUnixSocket",
        "AHardwareBuffer_lock",
        "AHardwareBuffer_unlock",
    ];
    let mut resolved = [core::ptr::null_mut::<c_void>(); 8];
    for (slot, name) in resolved.iter_mut().zip(names) {
        let cname = std::ffi::CString::new(name).ok()?;
        *slot = dlsym(handle, cname.as_ptr());
        if slot.is_null() {
            crate::log(&format!("ahb|symbol|missing|{name}"));
            dlclose(handle);
            return None;
        }
        crate::log(&format!("ahb|symbol|PASS|{name}"));
    }

    let symbols = Symbols {
        allocate: core::mem::transmute(resolved[0]),
        describe: core::mem::transmute(resolved[1]),
        release: core::mem::transmute(resolved[2]),
        native_handle: core::mem::transmute(resolved[3]),
        send: core::mem::transmute(resolved[4]),
        recv: core::mem::transmute(resolved[5]),
        lock: core::mem::transmute(resolved[6]),
        unlock: core::mem::transmute(resolved[7]),
    };
    Some((handle, symbols))
}

unsafe fn load_surface(lib: *mut c_void) -> Option<SurfaceSymbols> {
    let names = [
        "ASurfaceControl_createFromWindow",
        "ASurfaceControl_release",
        "ASurfaceTransaction_create",
        "ASurfaceTransaction_delete",
        "ASurfaceTransaction_setBuffer",
        "ASurfaceTransaction_setGeometry",
        "ASurfaceTransaction_setZOrder",
        "ASurfaceTransaction_apply",
    ];
    let mut resolved = [core::ptr::null_mut::<c_void>(); 8];
    for (slot, name) in resolved.iter_mut().zip(names) {
        let cname = std::ffi::CString::new(name).ok()?;
        *slot = dlsym(lib, cname.as_ptr());
        if slot.is_null() {
            crate::log(&format!("ahb|surface|missing|{name}"));
            return None;
        }
    }
    Some(SurfaceSymbols {
        create_from_window: core::mem::transmute(resolved[0]),
        release_control: core::mem::transmute(resolved[1]),
        create_transaction: core::mem::transmute(resolved[2]),
        delete_transaction: core::mem::transmute(resolved[3]),
        set_buffer: core::mem::transmute(resolved[4]),
        set_geometry: core::mem::transmute(resolved[5]),
        set_z_order: core::mem::transmute(resolved[6]),
        apply: core::mem::transmute(resolved[7]),
    })
}

fn cookie(i: usize, x: u32, y: u32) -> u32 {
    0x9a5c_0000 ^ ((i as u32) << 12) ^ (x << 1) ^ y
}

fn probe_points(width: u32, height: u32) -> [(u32, u32); 9] {
    let cx = width / 2;
    let cy = height / 2;
    [
        (0, 0),
        (cx, 0),
        (width - 1, 0),
        (0, cy),
        (cx, cy),
        (width - 1, cy),
        (0, height - 1),
        (cx, height - 1),
        (width - 1, height - 1),
    ]
}

unsafe fn probe_linear_view(s: &Symbols, buffer: *mut Ahb, desc: &Desc) -> bool {
    if desc.layers != 1
        || desc.width == 0
        || desc.height == 0
        || desc.stride < desc.width
        || desc.format != AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM
    {
        crate::log(&format!(
            "ahb|linear|SKIP|unsupported {}x{}x{} format={} stride={}",
            desc.width, desc.height, desc.layers, desc.format, desc.stride
        ));
        return false;
    }

    let mut cpu: *mut c_void = core::ptr::null_mut();
    let lock_rc = (s.lock)(
        buffer,
        AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN,
        -1,
        core::ptr::null(),
        &mut cpu,
    );
    crate::log(&format!("ahb|linear|lock_write|rc={lock_rc} cpu={cpu:p}"));
    if lock_rc != 0 || cpu.is_null() {
        return false;
    }
    let base = cpu as *mut u8;
    for (i, (x, y)) in probe_points(desc.width, desc.height)
        .into_iter()
        .enumerate()
    {
        let pixel = base.add(((y * desc.stride + x) * 4) as usize) as *mut u32;
        pixel.write_unaligned(cookie(i, x, y));
    }
    let unlock_rc = (s.unlock)(buffer, core::ptr::null_mut());
    crate::log(&format!("ahb|linear|unlock_write|rc={unlock_rc}"));
    if unlock_rc != 0 {
        return false;
    }

    cpu = core::ptr::null_mut();
    let lock_rc = (s.lock)(
        buffer,
        AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN,
        -1,
        core::ptr::null(),
        &mut cpu,
    );
    crate::log(&format!("ahb|linear|lock_read|rc={lock_rc} cpu={cpu:p}"));
    if lock_rc != 0 || cpu.is_null() {
        return false;
    }
    let base = cpu as *const u8;
    let mut matched = 0;
    let mut seen = [0u32; 9];
    for (i, (x, y)) in probe_points(desc.width, desc.height)
        .into_iter()
        .enumerate()
    {
        let pixel = base.add(((y * desc.stride + x) * 4) as usize) as *const u32;
        let got = pixel.read_unaligned();
        seen[i] = got;
        if got == cookie(i, x, y) {
            matched += 1;
        }
    }
    let unlock_rc = (s.unlock)(buffer, core::ptr::null_mut());
    crate::log(&format!(
        "ahb|linear|cpu_view|match={matched}/9|got={seen:?}|unlock={unlock_rc}"
    ));
    matched == 9 && unlock_rc == 0
}

unsafe fn buffer_is_red(s: &Symbols, buffer: *mut Ahb, desc: &Desc) -> bool {
    let mut cpu: *mut c_void = core::ptr::null_mut();
    let lock_rc = (s.lock)(
        buffer,
        AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN,
        -1,
        core::ptr::null(),
        &mut cpu,
    );
    if lock_rc != 0 || cpu.is_null() {
        crate::log(&format!("ahb|present|lock_read|FAIL|rc={lock_rc}"));
        return false;
    }
    let base = cpu as *const u8;
    let mut red = 0;
    for (x, y) in probe_points(desc.width, desc.height) {
        let pixel = base.add(((y * desc.stride + x) * 4) as usize) as *const u32;
        if pixel.read_unaligned() == 0xff00_00ff {
            red += 1;
        }
    }
    let unlock_rc = (s.unlock)(buffer, core::ptr::null_mut());
    crate::log(&format!(
        "ahb|present|readback|red={red}/9|unlock={unlock_rc}"
    ));
    red == 9 && unlock_rc == 0
}

unsafe fn egl_fallback(buffer: *mut Ahb, desc: &Desc) -> bool {
    crate::request_ahb_present(buffer, desc.width, desc.height);
    let ok = crate::wait_ahb_present(Duration::from_secs(10));
    crate::log(&format!("ahb|present|EGL_FALLBACK|{ok}"));
    ok
}

unsafe fn present_buffer(lib: *mut c_void, s: &Symbols, buffer: *mut Ahb, desc: &Desc) -> bool {
    if !buffer_is_red(s, buffer, desc) {
        crate::log("ahb|present|SKIP|guest pixels not red");
        return false;
    }
    let mut window = core::ptr::null_mut();
    for _ in 0..100 {
        window = crate::WINDOW.load(Ordering::SeqCst);
        if !window.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if window.is_null() {
        crate::log("ahb|present|FAIL|no ANativeWindow");
        return false;
    }
    let Some(surface) = load_surface(lib) else {
        crate::log("ahb|present|FAIL|surface symbols");
        return egl_fallback(buffer, desc);
    };
    let control = (surface.create_from_window)(window, c"husk-ahb".as_ptr());
    crate::log(&format!("ahb|present|control={control:p}"));
    if control.is_null() {
        crate::log("ahb|present|surface_control_unavailable|egl_fallback");
        return egl_fallback(buffer, desc);
    }
    let transaction = (surface.create_transaction)();
    if transaction.is_null() {
        crate::log("ahb|present|FAIL|transaction|egl_fallback");
        (surface.release_control)(control);
        return egl_fallback(buffer, desc);
    }
    let source = ARect {
        left: 0,
        top: 0,
        right: desc.width as i32,
        bottom: desc.height as i32,
    };
    let win_w = crate::sys::ANativeWindow_getWidth(window);
    let win_h = crate::sys::ANativeWindow_getHeight(window);
    let destination = ARect {
        left: 0,
        top: 0,
        right: win_w,
        bottom: win_h,
    };
    (surface.set_geometry)(transaction, control, &source, &destination, 0);
    (surface.set_z_order)(transaction, control, 100);
    (surface.set_buffer)(transaction, control, buffer, -1);
    (surface.apply)(transaction);
    (surface.delete_transaction)(transaction);
    crate::log(&format!(
        "ahb|present|PASS|SurfaceControl {win_w}x{win_h} from {}x{}",
        desc.width, desc.height
    ));
    std::thread::sleep(Duration::from_secs(5));
    (surface.release_control)(control);
    crate::log("ahb|present|released");
    true
}

fn bridge_meta(desc: &Desc) -> String {
    let mut cookies = Vec::with_capacity(9);
    for (i, (x, y)) in probe_points(desc.width, desc.height)
        .into_iter()
        .enumerate()
    {
        cookies.push(cookie(i, x, y).to_string());
    }
    format!(
        "HUSK-AHB-1\nwidth={}\nheight={}\nstride={}\nformat={}\nusage=0x{:x}\ncookies={}\n",
        desc.width,
        desc.height,
        desc.stride,
        desc.format,
        desc.usage,
        cookies.join(",")
    )
}

fn write_bridge_meta(files: &std::path::Path, desc: &Desc) {
    let meta = bridge_meta(desc);
    let path = files.join("ahb-bridge.meta");
    match std::fs::write(&path, meta) {
        Ok(()) => crate::log(&format!("ahb|bridge|META|{}", path.display())),
        Err(e) => crate::log(&format!("ahb|bridge|META_FAIL|{e}")),
    }
}

fn files_dir() -> Option<std::path::PathBuf> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return None;
    }
    let path = unsafe { (*act).internalDataPath };
    if path.is_null() {
        return None;
    }
    Some(std::path::PathBuf::from(
        unsafe { CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    ))
}

/// Keep the buffer alive long enough for a same-UID glibc guest process to
/// connect and receive the native handle. This is gated by a marker file so a
/// normal launch only runs the non-invasive allocation test.
unsafe fn serve_bridge(lib: *mut c_void, s: &Symbols, buffer: *mut Ahb, desc: &Desc) {
    let Some(files) = files_dir() else {
        crate::log("ahb|bridge|SKIP|no files dir");
        return;
    };
    let marker = files.join("ahb-bridge");
    if !marker.exists() {
        crate::log("ahb|bridge|SKIP|no marker");
        return;
    }
    write_bridge_meta(&files, desc);
    let path = files.join("ahb-bridge.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            crate::log(&format!("ahb|bridge|FAIL|bind {e}"));
            return;
        }
    };
    let _ = listener.set_nonblocking(true);
    crate::log(&format!("ahb|bridge|LISTEN|{}", path.display()));
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let meta = bridge_meta(desc);
                let meta_rc = stream
                    .write_all(&(meta.len() as u32).to_be_bytes())
                    .and_then(|_| stream.write_all(meta.as_bytes()));
                crate::log(&format!("ahb|bridge|META_SEND|rc={meta_rc:?}"));
                if meta_rc.is_err() {
                    continue;
                }
                let rc = (s.send)(buffer, stream.as_raw_fd());
                crate::log(&format!("ahb|bridge|SEND|rc={rc}"));
                if rc == 0 {
                    let timeout = stream.set_read_timeout(Some(Duration::from_secs(30)));
                    crate::log(&format!("ahb|bridge|ACK_TIMEOUT|{timeout:?}"));
                    let mut ack = [0u8; 1];
                    match stream.read_exact(&mut ack) {
                        Ok(()) => {
                            crate::log(&format!("ahb|bridge|ACK|byte=0x{:02x}", ack[0]));
                            if ack[0] == b'D' {
                                let presented = present_buffer(lib, s, buffer, desc);
                                crate::log(&format!("ahb|bridge|PRESENT|{presented}"));
                            }
                        }
                        Err(e) => crate::log(&format!("ahb|bridge|ACK_FAIL|{e}")),
                    }
                }
                let _ = std::fs::remove_file(&path);
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                crate::log(&format!("ahb|bridge|FAIL|accept {e}"));
                break;
            }
        }
    }
    crate::log("ahb|bridge|TIMEOUT");
    let _ = std::fs::remove_file(&path);
}

unsafe fn inspect_handle(label: &str, handle: *const c_void) -> c_int {
    if handle.is_null() {
        crate::log(&format!("ahb|{label}|handle=null"));
        return -1;
    }
    let words = handle as *const c_int;
    let version = words.read_unaligned();
    let num_fds = words.add(1).read_unaligned();
    let num_ints = words.add(2).read_unaligned();
    crate::log(&format!(
        "ahb|{label}|handle version={version} fds={num_fds} ints={num_ints}"
    ));
    if num_fds < 0 {
        return -1;
    }
    let count = num_fds.min(MAX_HANDLE_FDS);
    let ints_off = 3 + num_fds.max(0) as usize;
    let int_count = num_ints.max(0).min(32) as usize;
    if int_count > 0 {
        let mut words_out = Vec::with_capacity(int_count);
        for i in 0..int_count {
            words_out.push(words.add(ints_off + i).read_unaligned());
        }
        crate::log(&format!("ahb|{label}|ints[0..{int_count}]={words_out:?}"));
    }
    let mut first = -1;
    for i in 0..count {
        let fd = words.add(3 + i as usize).read_unaligned();
        if first < 0 {
            first = fd;
        }
        let valid = fcntl(fd, F_GETFD) >= 0;
        let target = std::fs::read_link(format!("/proc/self/fd/{fd}"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|e| format!("<readlink: {e}>"));
        crate::log(&format!(
            "ahb|{label}|fd[{i}]={fd} valid={valid} target={target}"
        ));
    }
    first
}

fn probe() {
    crate::log("ahb|probe|BEGIN");
    unsafe {
        let Some((lib, s)) = load() else {
            crate::log("ahb|probe|FAIL|symbols");
            return;
        };

        let desc = Desc {
            width: PROBE_WIDTH,
            height: PROBE_HEIGHT,
            layers: 1,
            format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
            usage: AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE
                | AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER
                | AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN
                | AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN,
            ..Desc::default()
        };
        let mut buffer: *mut Ahb = core::ptr::null_mut();
        let rc = (s.allocate)(&desc, &mut buffer);
        crate::log(&format!("ahb|allocate|rc={rc} ptr={buffer:p}"));
        if rc != 0 || buffer.is_null() {
            dlclose(lib);
            return;
        }

        let mut actual = Desc::default();
        (s.describe)(buffer, &mut actual);
        crate::log(&format!(
            "ahb|describe|{}x{}x{} format={} usage=0x{:x} stride={}",
            actual.width, actual.height, actual.layers, actual.format, actual.usage, actual.stride
        ));

        let linear_ok = probe_linear_view(&s, buffer, &actual);
        crate::log(&format!(
            "ahb|linear|cpu_contract|{}",
            if linear_ok { "PASS" } else { "FAIL" }
        ));

        let native = (s.native_handle)(buffer);
        let first_fd = inspect_handle("allocated", native);
        crate::log(&format!("ahb|allocated|first_fd={first_fd}"));

        let mut sv = [-1 as c_int; 2];
        let pair_rc = socketpair(AF_UNIX, SOCK_SEQPACKET, 0, sv.as_mut_ptr());
        crate::log(&format!(
            "ahb|socketpair|rc={pair_rc} tx={} rx={}",
            sv[0], sv[1]
        ));
        if pair_rc == 0 {
            let send_rc = (s.send)(buffer, sv[0]);
            crate::log(&format!("ahb|send|rc={send_rc}"));
            if send_rc == 0 {
                let mut received: *mut Ahb = core::ptr::null_mut();
                let recv_rc = (s.recv)(sv[1], &mut received);
                crate::log(&format!("ahb|recv|rc={recv_rc} ptr={received:p}"));
                if recv_rc == 0 && !received.is_null() {
                    let received_native = (s.native_handle)(received);
                    let received_fd = inspect_handle("received", received_native);
                    crate::log(&format!("ahb|received|first_fd={received_fd}"));
                    (s.release)(received);
                    crate::log("ahb|received|released");
                }
            }
            close(sv[0]);
            close(sv[1]);
        }

        serve_bridge(lib, &s, buffer, &actual);
        (s.release)(buffer);
        crate::log("ahb|allocated|released");
        dlclose(lib);
        crate::log("ahb|probe|DONE");
    }
}
