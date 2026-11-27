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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RTLD_NOW: c_int = 2;
const AF_UNIX: c_int = 1;
const SOCK_SEQPACKET: c_int = 5;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const F_GETFD: c_int = 1;
const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
const AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER: u64 = 1 << 9;
const AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN: u64 = 1 << 1;
const AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN: u64 = 1 << 5;
const MAX_HANDLE_FDS: c_int = 16;

const PROBE_WIDTH: u32 = 64;
const PROBE_HEIGHT: u32 = 64;
const RING_MAX: usize = 4;
const RING_FRAMES_DEFAULT: usize = 12;
const POLLIN: i16 = 0x0001;

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
type FnCreateChildSurfaceControl =
    unsafe extern "C" fn(SurfaceControl, *const c_char) -> SurfaceControl;
type FnReleaseSurfaceControl = unsafe extern "C" fn(SurfaceControl);
type FnCreateSurfaceTransaction = unsafe extern "C" fn() -> SurfaceTransaction;
type FnDeleteSurfaceTransaction = unsafe extern "C" fn(SurfaceTransaction);
type FnSetBuffer = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *mut Ahb, c_int);
type FnSetGeometry =
    unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *const ARect, *const ARect, i32);
type FnSetZOrder = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, i32);
type FnApplySurfaceTransaction = unsafe extern "C" fn(SurfaceTransaction);
type FnSetOnComplete = unsafe extern "C" fn(
    SurfaceTransaction,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, *mut c_void),
);
type FnGetPreviousReleaseFenceFd = unsafe extern "C" fn(*mut c_void, SurfaceControl) -> c_int;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    name_len: u32,
    iov: *mut IoVec,
    iov_len: usize,
    control: *mut c_void,
    control_len: usize,
    flags: c_int,
}

#[repr(C)]
struct CmsgHdr {
    len: usize,
    level: c_int,
    ty: c_int,
}

struct SurfaceSymbols {
    create_from_window: FnCreateSurfaceControl,
    create_child: FnCreateChildSurfaceControl,
    release_control: FnReleaseSurfaceControl,
    create_transaction: FnCreateSurfaceTransaction,
    delete_transaction: FnDeleteSurfaceTransaction,
    set_buffer: FnSetBuffer,
    set_geometry: FnSetGeometry,
    set_z_order: FnSetZOrder,
    apply: FnApplySurfaceTransaction,
    set_on_complete: FnSetOnComplete,
    get_previous_release_fence: FnGetPreviousReleaseFenceFd,
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn socketpair(domain: c_int, ty: c_int, protocol: c_int, sv: *mut c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn send(fd: c_int, buf: *const c_void, len: usize, flags: c_int) -> isize;
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
        "ASurfaceControl_create",
        "ASurfaceControl_release",
        "ASurfaceTransaction_create",
        "ASurfaceTransaction_delete",
        "ASurfaceTransaction_setBuffer",
        "ASurfaceTransaction_setGeometry",
        "ASurfaceTransaction_setZOrder",
        "ASurfaceTransaction_apply",
        "ASurfaceTransaction_setOnComplete",
        "ASurfaceTransactionStats_getPreviousReleaseFenceFd",
    ];
    let mut resolved = [core::ptr::null_mut::<c_void>(); 11];
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
        create_child: core::mem::transmute(resolved[1]),
        release_control: core::mem::transmute(resolved[2]),
        create_transaction: core::mem::transmute(resolved[3]),
        delete_transaction: core::mem::transmute(resolved[4]),
        set_buffer: core::mem::transmute(resolved[5]),
        set_geometry: core::mem::transmute(resolved[6]),
        set_z_order: core::mem::transmute(resolved[7]),
        apply: core::mem::transmute(resolved[8]),
        set_on_complete: core::mem::transmute(resolved[9]),
        get_previous_release_fence: core::mem::transmute(resolved[10]),
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
    let parent = crate::surface::wait_parent(Duration::from_secs(10));
    let control = match parent {
        Some(parent) => {
            crate::log(&format!("ahb|present|surface_parent={parent:#x}"));
            (surface.create_child)(parent as SurfaceControl, c"husk-ahb".as_ptr())
        }
        None => {
            crate::log("ahb|present|surface_parent|fallback|window");
            (surface.create_from_window)(window, c"husk-ahb".as_ptr())
        }
    };
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

fn ring_meta(desc: &Desc, count: usize, frames: usize) -> String {
    let mut cookies = Vec::with_capacity(9);
    for (i, (x, y)) in probe_points(desc.width, desc.height)
        .into_iter()
        .enumerate()
    {
        cookies.push(cookie(i, x, y).to_string());
    }
    format!(
        "HUSK-AHB-2
width={}
height={}
stride={}
format={}
usage=0x{:x}
cookies={}
count={count}
frames={frames}
",
        desc.width,
        desc.height,
        desc.stride,
        desc.format,
        desc.usage,
        cookies.join(",")
    )
}

fn bridge_config() -> Option<(usize, usize)> {
    let files = files_dir()?;
    let text = std::fs::read_to_string(files.join("ahb-bridge")).ok()?;
    let mut ring = 1usize;
    let mut frames = RING_FRAMES_DEFAULT;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ring=") {
            ring = value.parse().ok()?;
        } else if let Some(value) = line.strip_prefix("frames=") {
            frames = value.parse().ok()?;
        }
    }
    Some((ring.clamp(1, RING_MAX), frames.clamp(1, 10_000)))
}

fn write_bridge_meta(files: &std::path::Path, desc: &Desc) {
    let meta = bridge_meta(desc);
    let path = match crate::guest_rootfs::resolve_path(files, "run/husk/ahb-bridge.meta") {
        Ok(path) => path,
        Err(e) => {
            crate::log(&format!("ahb|bridge|META_FAIL|{e}"));
            return;
        }
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            crate::log(&format!(
                "ahb|bridge|META_FAIL|mkdir {}: {e}",
                dir.display()
            ));
            return;
        }
    }
    match std::fs::write(&path, meta) {
        Ok(()) => crate::log(&format!("ahb|bridge|META|{}", path.display())),
        Err(e) => crate::log(&format!("ahb|bridge|META_FAIL|{e}")),
    }
}

pub(crate) fn files_dir_pub() -> Option<std::path::PathBuf> {
    files_dir()
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
    let path = match crate::guest_rootfs::resolve_path(&files, "run/husk/ahb-bridge.sock") {
        Ok(path) => path,
        Err(e) => {
            crate::log(&format!("ahb|bridge|FAIL|{e}"));
            return;
        }
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            crate::log(&format!("ahb|bridge|FAIL|mkdir {}: {e}", dir.display()));
            return;
        }
    }
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

#[derive(Default)]
struct RingProgress {
    completed: AtomicUsize,
}

struct ReleaseContext {
    previous_slot: Option<usize>,
    current_slot: usize,
    stream: std::os::unix::net::UnixStream,
    control: SurfaceControl,
    get_previous_release_fence: FnGetPreviousReleaseFenceFd,
    progress: Arc<RingProgress>,
}

unsafe fn wait_release_fence(fd: c_int, timeout_ms: c_int) -> bool {
    if fd < 0 {
        return true;
    }
    let mut pfd = PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    };
    let rc = loop {
        let rc = poll(&mut pfd, 1, timeout_ms);
        if rc >= 0 || std::io::Error::last_os_error().raw_os_error() != Some(4) {
            break rc;
        }
    };
    let signaled = rc > 0 && (pfd.revents & POLLIN) != 0;
    close(fd);
    signaled
}

unsafe extern "C" fn on_transaction_complete(context: *mut c_void, stats: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() || stats.is_null() {
            crate::log("ahb|ring|complete|FAIL|null context/stats");
            return;
        }
        let ctx = Box::from_raw(context as *mut ReleaseContext);
        let completed = ctx.progress.completed.fetch_add(1, Ordering::SeqCst) + 1;
        let release_fd = (ctx.get_previous_release_fence)(stats, ctx.control);
        let released = wait_release_fence(release_fd, 2_000);
        crate::log(&format!(
            "ahb|ring|complete|frame_slot={}|previous={:?}|release_fd={release_fd}|released={released}|completed={completed}",
            ctx.current_slot, ctx.previous_slot
        ));
        if let Some(previous_slot) = ctx.previous_slot {
            if released {
                let message = format!("RELEASE {previous_slot}\n");
                if send_ring_text(&ctx.stream, &message) {
                    crate::log(&format!("ahb|ring|release_sent|slot={previous_slot}"));
                } else {
                    crate::log(&format!("ahb|ring|release_send|FAIL|slot={previous_slot}"));
                }
            }
        }
    }));
}

unsafe fn present_ring_frame(
    surface: &SurfaceSymbols,
    control: SurfaceControl,
    buffer: *mut Ahb,
    desc: &Desc,
    window: *mut c_void,
    current_slot: usize,
    previous_slot: Option<usize>,
    acquire_fd: c_int,
    stream: &std::os::unix::net::UnixStream,
    progress: &Arc<RingProgress>,
) -> bool {
    let stream = match stream.try_clone() {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("ahb|ring|present|FAIL|clone stream {e}"));
            return false;
        }
    };
    let transaction = (surface.create_transaction)();
    if transaction.is_null() {
        crate::log("ahb|ring|present|FAIL|transaction");
        return false;
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
    (surface.set_buffer)(transaction, control, buffer, acquire_fd);
    let context = Box::new(ReleaseContext {
        previous_slot,
        current_slot,
        stream,
        control,
        get_previous_release_fence: surface.get_previous_release_fence,
        progress: Arc::clone(progress),
    });
    (surface.set_on_complete)(
        transaction,
        Box::into_raw(context) as *mut c_void,
        on_transaction_complete,
    );
    (surface.apply)(transaction);
    (surface.delete_transaction)(transaction);
    crate::log(&format!(
        "ahb|ring|present|slot={current_slot}|previous={previous_slot:?}|acquire_fd={acquire_fd}|{win_w}x{win_h}"
    ));
    true
}

fn cmsg_align(len: usize) -> usize {
    (len + core::mem::size_of::<usize>() - 1) & !(core::mem::size_of::<usize>() - 1)
}

unsafe fn send_ring_text(stream: &std::os::unix::net::UnixStream, text: &str) -> bool {
    let sent = send(
        stream.as_raw_fd(),
        text.as_ptr() as *const c_void,
        text.len(),
        0,
    );
    if sent == text.len() as isize {
        true
    } else {
        crate::log(&format!(
            "ahb|ring|send|FAIL|sent={sent}|len={}|{}",
            text.len(),
            std::io::Error::last_os_error()
        ));
        false
    }
}

unsafe fn recv_ring_record(
    stream: &std::os::unix::net::UnixStream,
) -> Result<(String, Vec<c_int>), String> {
    let mut data = [0u8; 256];
    let mut control = [0u8; 64];
    let mut iov = IoVec {
        base: data.as_mut_ptr() as *mut c_void,
        len: data.len(),
    };
    let mut msg = MsgHdr {
        name: core::ptr::null_mut(),
        name_len: 0,
        iov: &mut iov,
        iov_len: 1,
        control: control.as_mut_ptr() as *mut c_void,
        control_len: control.len(),
        flags: 0,
    };
    let n = recvmsg(stream.as_raw_fd(), &mut msg, 0);
    if n < 0 {
        return Err(format!("recvmsg: {}", std::io::Error::last_os_error()));
    }
    if n == 0 {
        return Err("peer closed the ring socket".to_string());
    }
    let text = String::from_utf8_lossy(&data[..n as usize])
        .trim()
        .to_string();
    let mut fds = Vec::new();
    let control_len = msg.control_len.min(control.len());
    let mut offset = 0usize;
    while offset + core::mem::size_of::<CmsgHdr>() <= control_len {
        let hdr = &*(control.as_ptr().add(offset) as *const CmsgHdr);
        if hdr.len < core::mem::size_of::<CmsgHdr>() || offset + hdr.len > control_len {
            break;
        }
        if hdr.level == SOL_SOCKET && hdr.ty == SCM_RIGHTS {
            let data_offset = offset + cmsg_align(core::mem::size_of::<CmsgHdr>());
            let data_end = offset + hdr.len;
            if data_offset <= data_end {
                let count = (data_end - data_offset) / core::mem::size_of::<c_int>();
                for i in 0..count {
                    fds.push(
                        (control.as_ptr().add(data_offset) as *const c_int)
                            .add(i)
                            .read_unaligned(),
                    );
                }
            }
        }
        offset += cmsg_align(hdr.len);
    }
    crate::log(&format!(
        "ahb|ring|recv|bytes={n}|control_len={}|fds={}|flags={:#x}|text={text}",
        msg.control_len,
        fds.len(),
        msg.flags
    ));
    Ok((text, fds))
}

unsafe fn serve_ring_bridge(
    lib: *mut c_void,
    s: &Symbols,
    buffers: &[*mut Ahb],
    desc: &Desc,
    frames: usize,
) -> bool {
    let Some(files) = files_dir() else {
        crate::log("ahb|ring|SKIP|no files dir");
        return false;
    };
    let path = files.join("ahb-bridge.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("ahb|ring|FAIL|bind {e}"));
            return false;
        }
    };
    let _ = listener.set_nonblocking(true);
    crate::log(&format!("ahb|ring|LISTEN|{}", path.display()));
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut stream = loop {
        if Instant::now() >= deadline {
            crate::log("ahb|ring|TIMEOUT|accept");
            let _ = std::fs::remove_file(&path);
            return false;
        }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                crate::log(&format!("ahb|ring|FAIL|accept {e}"));
                let _ = std::fs::remove_file(&path);
                return false;
            }
        }
    };

    let meta = ring_meta(desc, buffers.len(), frames);
    if stream
        .write_all(&(meta.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(meta.as_bytes()))
        .is_err()
    {
        crate::log("ahb|ring|FAIL|meta send");
        let _ = std::fs::remove_file(&path);
        return false;
    }
    for (slot, &buffer) in buffers.iter().enumerate() {
        let rc = (s.send)(buffer, stream.as_raw_fd());
        crate::log(&format!("ahb|ring|send|slot={slot}|rc={rc}"));
        if rc != 0 {
            let _ = std::fs::remove_file(&path);
            return false;
        }
    }

    let reader_stream = match stream.try_clone() {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("ahb|ring|FAIL|clone reader {e}"));
            let _ = std::fs::remove_file(&path);
            return false;
        }
    };
    let _ = reader_stream.set_read_timeout(Some(Duration::from_secs(60)));
    let Some(surface) = load_surface(lib) else {
        crate::log("ahb|ring|FAIL|surface symbols");
        let _ = std::fs::remove_file(&path);
        return false;
    };
    let Some(parent) = crate::surface::wait_parent(Duration::from_secs(10)) else {
        crate::log("ahb|ring|FAIL|surface parent");
        let _ = std::fs::remove_file(&path);
        return false;
    };
    let mut window = core::ptr::null_mut();
    for _ in 0..100 {
        window = crate::WINDOW.load(Ordering::SeqCst);
        if !window.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if window.is_null() {
        crate::log("ahb|ring|FAIL|no ANativeWindow");
        let _ = std::fs::remove_file(&path);
        return false;
    }
    let control = (surface.create_child)(parent as SurfaceControl, c"husk-ahb-ring".as_ptr());
    if control.is_null() {
        crate::log("ahb|ring|FAIL|SurfaceControl");
        let _ = std::fs::remove_file(&path);
        return false;
    }

    let progress = Arc::new(RingProgress::default());
    let mut last_submitted = None;
    let mut submitted = 0usize;
    let mut guest_done = false;
    loop {
        let (text, received_fds) = match recv_ring_record(&reader_stream) {
            Ok(v) => v,
            Err(e) => {
                crate::log(&format!("ahb|ring|read|FAIL|{e}"));
                break;
            }
        };
        if text.starts_with("DONE ") {
            for fd in received_fds {
                close(fd);
            }
            crate::log(&format!("ahb|ring|GUEST_DONE|{text}"));
            guest_done = true;
            break;
        }
        let fields: Vec<&str> = text.split_whitespace().collect();
        if fields.len() != 4 || fields[0] != "READY" {
            for fd in received_fds {
                close(fd);
            }
            crate::log(&format!("ahb|ring|RX|{text}"));
            continue;
        }
        let acquire_fd = received_fds.first().copied().unwrap_or(-1);
        for &extra in received_fds.iter().skip(1) {
            crate::log(&format!("ahb|ring|FAIL|extra acquire fd={extra}"));
            close(extra);
        }
        let Ok(slot) = fields[1].parse::<usize>() else {
            if acquire_fd >= 0 {
                close(acquire_fd);
            }
            crate::log(&format!("ahb|ring|FAIL|bad slot|{text}"));
            break;
        };
        if slot >= buffers.len() {
            if acquire_fd >= 0 {
                close(acquire_fd);
            }
            crate::log(&format!("ahb|ring|FAIL|slot out of range|{text}"));
            break;
        }
        if !present_ring_frame(
            &surface,
            control,
            buffers[slot],
            desc,
            window,
            slot,
            last_submitted,
            acquire_fd,
            &stream,
            &progress,
        ) {
            if acquire_fd >= 0 {
                close(acquire_fd);
            }
            break;
        }
        last_submitted = Some(slot);
        submitted += 1;
    }

    let wait_deadline = Instant::now() + Duration::from_secs(5);
    while progress.completed.load(Ordering::SeqCst) < submitted && Instant::now() < wait_deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let completed = progress.completed.load(Ordering::SeqCst);
    crate::log(&format!(
        "ahb|ring|SUMMARY|submitted={submitted}|completed={completed}|guest_done={guest_done}"
    ));
    std::thread::sleep(Duration::from_secs(3));
    (surface.release_control)(control);
    let _ = std::fs::remove_file(&path);
    guest_done && submitted == frames && completed > 0
}

unsafe fn probe_ring(lib: *mut c_void, s: &Symbols, ring: usize, frames: usize) -> bool {
    crate::log(&format!("ahb|ring|BEGIN|buffers={ring}|frames={frames}"));
    let requested = Desc {
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
    let mut buffers = Vec::with_capacity(ring);
    let mut actual_desc = Desc::default();
    for slot in 0..ring {
        let mut buffer: *mut Ahb = core::ptr::null_mut();
        let rc = (s.allocate)(&requested, &mut buffer);
        crate::log(&format!(
            "ahb|ring|allocate|slot={slot}|rc={rc}|ptr={buffer:p}"
        ));
        if rc != 0 || buffer.is_null() {
            for &buffer in &buffers {
                (s.release)(buffer);
            }
            return false;
        }
        actual_desc = Desc::default();
        (s.describe)(buffer, &mut actual_desc);
        let linear_ok = probe_linear_view(s, buffer, &actual_desc);
        crate::log(&format!(
            "ahb|ring|describe|slot={slot}|{}x{}|stride={}|linear={linear_ok}",
            actual_desc.width, actual_desc.height, actual_desc.stride
        ));
        if !linear_ok {
            (s.release)(buffer);
            for &buffer in &buffers {
                (s.release)(buffer);
            }
            return false;
        }
        buffers.push(buffer);
    }

    let ok = serve_ring_bridge(lib, s, &buffers, &actual_desc, frames);
    for &buffer in &buffers {
        (s.release)(buffer);
    }
    crate::log(&format!(
        "ahb|ring|DONE|ok={ok}|buffers_released={}",
        buffers.len()
    ));
    ok
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
        let (ring, frames) = bridge_config().unwrap_or((1, 1));
        if ring > 1 {
            let ok = probe_ring(lib, &s, ring, frames);
            crate::log(&format!("ahb|probe|RING|ok={ok}"));
            dlclose(lib);
            return;
        }

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
