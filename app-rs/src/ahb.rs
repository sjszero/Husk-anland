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
use std::sync::atomic::{AtomicBool, Ordering};

const RTLD_NOW: c_int = 2;
const AF_UNIX: c_int = 1;
const SOCK_SEQPACKET: c_int = 5;
const F_GETFD: c_int = 1;
const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
const AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER: u64 = 1 << 9;
const MAX_HANDLE_FDS: c_int = 16;

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

struct Symbols {
    allocate: FnAllocate,
    describe: FnDescribe,
    release: FnRelease,
    native_handle: FnGetNativeHandle,
    send: FnSendToSocket,
    recv: FnRecvFromSocket,
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
    ];
    let mut resolved = [core::ptr::null_mut::<c_void>(); 6];
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
    };
    Some((handle, symbols))
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
            width: 64,
            height: 64,
            layers: 1,
            format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
            usage: AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE | AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER,
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

        (s.release)(buffer);
        crate::log("ahb|allocated|released");
        dlclose(lib);
        crate::log("ahb|probe|DONE");
    }
}
