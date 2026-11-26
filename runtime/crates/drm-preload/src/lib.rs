//! libhusk-drm.so — LD_PRELOAD 版的 DRM 语义层（no_std：不引入 Rust 运行时，
//! 也不跟 glibc 的 `open` 符号声明打架）。
//!
//! 为什么用 LD_PRELOAD 而不是编进 loader：loader 只能 patch **启动时**已加载
//! 对象的 GOT；Mesa 的 libgbm / libdrm / panfrost_dri.so 都是运行时 dlopen 的，
//! 只有 glibc 的 preload 机制能覆盖它们。
//!
//! 职责三件，其余一律透传：
//!   open/openat("/dev/dri/renderD128"|"card0") -> 假 fd（真 /dev/null）
//!   ioctl(fake fd, PANFROST_*|DRM_*)           -> drm_shim::DrmCtx::dispatch
//!   mmap(fake fd, off=GPU VA)                  -> 转到 /dev/mali0 上真映射

#![no_std]

mod drmapi;

use core::sync::atomic::{AtomicBool, Ordering};

extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn __errno_location() -> *mut i32;
}

const SYS_OPENAT: i64 = 56;
const SYS_IOCTL: i64 = 29;
const SYS_MMAP: i64 = 222;
const SYS_CLOSE: i64 = 57;
const AT_FDCWD: i32 = -100;
const O_RDWR: i32 = 2;

static LOCK: AtomicBool = AtomicBool::new(false);
static mut CTX: drm_shim::DrmCtx = drm_shim::DrmCtx::empty();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("svc 0", in("x8") 94usize, in("x0") 103usize,
            lateout("x0") _, options(nostack));
    }
    loop {
        core::hint::spin_loop();
    }
}

/// panic=abort 下仍然会被链接器要这个符号（依赖里带展开信息时）
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

fn lock() {
    let mut n: u64 = 0;
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
        n += 1;
        if n == 50_000_000 {
            unsafe {
                sycall_write(b"[preload] LOCK SPIN DEADLOCK\n");
            }
        }
    }
}
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

fn set_errno(e: i32) {
    unsafe { *__errno_location() = e }
}

fn ioctl_raw(fd: i32, req: u64, arg: *mut u8) -> i64 {
    unsafe { syscall(SYS_IOCTL, fd as i64, req, arg as i64, 0i64) as i64 }
}

fn open_raw(path: &[u8], flags: i32, mode: i32) -> i32 {
    unsafe {
        syscall(
            SYS_OPENAT,
            AT_FDCWD as i64,
            path.as_ptr() as i64,
            flags as i64,
            mode as i64,
        ) as i32
    }
}

const SYS_READLINKAT: i64 = 78;
const SYS_WRITE: i64 = 64;

fn dbgw(tag: &[u8], body: &[u8]) {
    unsafe {
        syscall(SYS_WRITE, 2i64, tag.as_ptr() as i64, tag.len() as i64, 0i64);
        sycall_write(body);
    }
}
unsafe fn sycall_write(b: &[u8]) {
    syscall(SYS_WRITE, 2i64, b.as_ptr() as i64, b.len() as i64, 0i64);
}

unsafe fn is_fake(fd: i32) -> bool {
    // Never trust a fd number kept in a private table. dup2/dup3 can close the
    // old destination and install an ordinary file under the same number; glibc
    // internals can also close without entering this preload's close symbol.
    // The kernel's /proc/self/fd target is the identity. Both fake forms are
    // recognizable: loader memfd "husk-drm" and the real kbase node "mali".
    if fd < 0 {
        return false;
    }
    let mut link = [0u8; 64];
    for (i, c) in b"/proc/self/fd/".iter().enumerate() {
        link[i] = *c;
    }
    let mut v = fd as u32;
    let mut d = [0u8; 10];
    let mut dn = 0;
    if v == 0 {
        d[0] = b'0';
        dn = 1;
    }
    while v > 0 {
        d[dn] = b'0' + (v % 10) as u8;
        v /= 10;
        dn += 1;
    }
    let mut j = 14;
    while dn > 0 {
        dn -= 1;
        link[j] = d[dn];
        j += 1;
    }
    let mut out = [0u8; 96];
    let r = syscall(
        SYS_READLINKAT,
        AT_FDCWD as i64,
        link.as_ptr() as i64,
        out.as_mut_ptr() as i64,
        out.len() as i64,
    );
    if r <= 0 {
        return false;
    }
    let n = r as usize;
    let s = &out[..n];
    let hit = s.windows(12).any(|w| w == b"/memfd:husk-") || s.windows(4).any(|w| w == b"mali");
    hit
}

unsafe fn cstr_len(p: *const u8) -> usize {
    let mut n = 0;
    while n < 4096 && *p.add(n) != 0 {
        n += 1;
    }
    n
}

unsafe fn is_drm_node(p: *const u8) -> bool {
    if p.is_null() {
        return false;
    }
    let n = cstr_len(p);
    let s = core::slice::from_raw_parts(p, n);
    s == b"/dev/dri/renderD128" || s == b"/dev/dri/card0"
}

/// 假节点：懒初始化 kbase context，返回真 fd（/dev/null）
unsafe fn fake_drm_open() -> i32 {
    if debug_on() {
        sycall_write(b"[preload] fake_drm_open enter\n");
    }
    lock();
    if !CTX.ready {
        let mali = open_raw(b"/dev/mali0\0", O_RDWR, 0);
        if mali < 0 {
            unlock();
            return -1;
        }
        let ir = CTX.init(mali, |f, req, arg| ioctl_raw(f, req, arg));
        if debug_on() {
            sycall_write(if ir.is_ok() {
                b"[preload] CTX.init ok\n"
            } else {
                b"[preload] CTX.init FAIL\n"
            });
        }
        if ir.is_err() {
            unlock();
            return -1;
        }
    }
    unlock();
    // 用真的 /dev/mali0：dup 出来的 fd 靠 readlink 里的 "mali" 认回来，
    // libdrm 那边 fstat 也能看到一个合法的字符设备（memfd 的 st_rdev 是 0:0，会被当成非 DRM 设备）。
    open_raw(b"/dev/mali0\0", O_RDWR, 0)
}

#[no_mangle]
pub unsafe extern "C" fn open(path: *const u8, flags: i32, _mode: i32) -> i32 {
    if is_drm_node(path) {
        return fake_drm_open();
    }
    if path.is_null() {
        set_errno(22);
        return -1;
    }
    let n = cstr_len(path);
    if debug_on() {
        dbgw(b"[preload open] ", core::slice::from_raw_parts(path, n));
        sycall_write(b"\n");
    }
    open_raw(core::slice::from_raw_parts(path, n + 1), flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn open64(path: *const u8, flags: i32, _mode: i32) -> i32 {
    open(path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: i32, path: *const u8, flags: i32, _mode: i32) -> i32 {
    if is_drm_node(path) {
        return fake_drm_open();
    }
    let r = syscall(SYS_OPENAT, dirfd as i64, path as i64, flags as i64, 0i64);
    if r < 0 && r > -4096 {
        set_errno((-r) as i32);
        return -1;
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn openat64(dirfd: i32, path: *const u8, flags: i32, _mode: i32) -> i32 {
    openat(dirfd, path, flags, 0)
}

#[no_mangle]
pub(crate) unsafe fn ioctl_impl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    if debug_on() {
        sycall_write(b"[ioctl enter]\n");
    }
    if debug_on() && is_fake(fd) {
        let mut b = [0u8; 40];
        let hex = b"0123456789abcdef";
        let pre = b"[preload ioctl] ";
        let mut i = 0;
        while i < pre.len() {
            b[i] = pre[i];
            i += 1;
        }
        let mut sh = 60;
        while sh >= 0 {
            b[i] = hex[((request >> sh) & 0xf) as usize];
            i += 1;
            sh -= 4;
        }
        b[i] = b'\n';
        i += 1;
        sycall_write(&b[..i]);
    }
    if is_fake(fd) {
        lock();
        if CTX.ready {
            let r = CTX.dispatch(|f, req, a| ioctl_raw(f, req, a), request, arg);
            unlock();
            if debug_on() {
                let mut b = [0u8; 64];
                let hex = b"0123456789abcdef";
                let p0 = b"[ioctl] req=";
                let mut i = 0;
                while i < p0.len() {
                    b[i] = p0[i];
                    i += 1;
                }
                let mut sh = 60;
                while sh >= 0 {
                    b[i] = hex[((request >> sh) & 0xf) as usize];
                    i += 1;
                    sh -= 4;
                }
                let p1 = b" ret=";
                let mut k = 0;
                while k < p1.len() {
                    b[i] = p1[k];
                    i += 1;
                    k += 1;
                }
                let neg = r < 0;
                let v = if neg { (-r) as u64 } else { r as u64 };
                if neg {
                    b[i] = b'-';
                    i += 1;
                }
                let mut sh2 = 28;
                while sh2 >= 0 {
                    b[i] = hex[((v >> sh2) & 0xf) as usize];
                    i += 1;
                    sh2 -= 4;
                }
                b[i] = b'\n';
                i += 1;
                sycall_write(&b[..i]);
            }
            if debug_on() && request == 0xc0106444 && !arg.is_null() {
                let param = *(arg as *const u32);
                let value = *((arg as *const u8).add(8) as *const u64);
                let mut b = [0u8; 48];
                let hex = b"0123456789abcdef";
                let p0 = b"[getparam] id=";
                let mut i = 0;
                while i < p0.len() {
                    b[i] = p0[i];
                    i += 1;
                }
                b[i] = hex[((param >> 4) & 0xf) as usize];
                i += 1;
                b[i] = hex[(param & 0xf) as usize];
                i += 1;
                let p1 = b" val=";
                let mut k = 0;
                while k < p1.len() {
                    b[i] = p1[k];
                    i += 1;
                    k += 1;
                }
                let mut sh = 60;
                while sh >= 0 {
                    b[i] = hex[((value >> sh) & 0xf) as usize];
                    i += 1;
                    sh -= 4;
                }
                b[i] = b'\n';
                i += 1;
                sycall_write(&b[..i]);
            }
            if r < 0 {
                set_errno((-r) as i32);
                return -1;
            }
            return 0;
        }
        unlock();
        set_errno(25);
        return -1;
    }
    let r = ioctl_raw(fd, request, arg);
    if r < 0 && r > -4096 {
        set_errno((-r) as i32);
        return -1;
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    ioctl_impl(fd, request, arg)
}

unsafe fn do_mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8 {
    let (real, close_after) = if is_fake(fd) {
        let m = open_raw(b"/dev/mali0\0", O_RDWR, 0);
        if m < 0 {
            set_errno(2);
            return core::ptr::null_mut();
        }
        (m, true)
    } else {
        (fd, false)
    };
    let r = syscall(
        SYS_MMAP,
        addr as i64,
        len as i64,
        prot as i64,
        flags as i64,
        real as i64,
        off,
    );
    if close_after {
        syscall(SYS_CLOSE, real as i64);
    }
    if r < 0 {
        set_errno((-r) as i32);
        return core::ptr::null_mut();
    }
    r as *mut u8
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    addr: *mut u8,
    len: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(addr, len, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn mmap64(
    addr: *mut u8,
    len: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(addr, len, prot, flags, fd, off)
}

/* ---- 桥：把这一层的能力交给 loader 里的 path-shim --------------------------
 *
 * loader 的 shim 抢在 LD_PRELOAD 前面拿到了进程中**启动时**那些对象的 open/ioctl/
 * mmap（它靠 _dl_init 钩子改 GOT）；而 Mesa 运行时 dlopen 的 libdrm / libgbm /
 * panfrost_dri.so 只有 preload 能看到。两层必须共用一份 DRM 状态（一个 kbase
 * context、一张 BO 表），所以由 preload 独占，path-shim 通过这里转交。
 *
 * 交接方式：preload 在构造函数里把函数表地址写进环境变量 HUSK_DRM_API；
 * path-shim 在需要时读一次。没有 preload 时变量不存在，path-shim 用自己的实现。
 */
pub(crate) unsafe fn debug_on_pub() -> bool {
    debug_on()
}

pub(crate) unsafe fn syscall_readlinkat(path: *const u8, out: *mut u8, len: usize) -> i64 {
    syscall(
        SYS_READLINKAT,
        AT_FDCWD as i64,
        path as i64,
        out as i64,
        len as i64,
    )
}

static DBG: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);

unsafe fn debug_on() -> bool {
    use core::sync::atomic::Ordering;
    match DBG.load(Ordering::Relaxed) {
        0 => return false,
        1 => return true,
        _ => {}
    }
    let mut on = false;
    const SYS_OPENAT2: i64 = 56;
    const SYS_READ: i64 = 63;
    let fd = syscall(
        SYS_OPENAT2,
        AT_FDCWD as i64,
        b"/proc/self/environ\0".as_ptr() as i64,
        0i64,
        0i64,
    );
    if fd >= 0 {
        let mut buf = [0u8; 4096];
        let n = syscall(SYS_READ, fd, buf.as_mut_ptr() as i64, 4096i64);
        syscall(SYS_CLOSE, fd);
        if n > 0 {
            let s = &buf[..n as usize];
            let needle = b"NR_DRM_DEBUG=";
            let mut i = 0;
            while i + needle.len() <= s.len() {
                if &s[i..i + needle.len()] == needle {
                    on = true;
                    break;
                }
                i += 1;
            }
        }
    }
    DBG.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    on
}

extern "C" {
    static mut environ: *mut *mut u8;
    fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
}
const ENVIRON: *mut *mut u8 = &raw mut environ as *mut *mut u8;

#[no_mangle]
pub unsafe extern "C" fn husk_drm_fake_open() -> i32 {
    fake_drm_open()
}

#[no_mangle]
pub unsafe extern "C" fn husk_drm_ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    ioctl(fd, request, arg)
}

#[no_mangle]
pub unsafe extern "C" fn husk_drm_mmap(
    addr: *mut u8,
    len: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(addr, len, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn drm_shim_is_fake(fd: i32) -> i32 {
    if is_fake(fd) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_drm_is_fake(fd: i32) -> i32 {
    if is_fake(fd) {
        1
    } else {
        0
    }
}

static mut API: [usize; 4] = [0; 4];
static mut API_ENV: [u8; 32] = [0; 32];

unsafe extern "C" fn api_init() {
    API[0] = husk_drm_fake_open as usize;
    API[1] = husk_drm_ioctl as usize;
    API[2] = husk_drm_mmap as usize;
    API[3] = husk_drm_is_fake as usize;
    const HEX: &[u8] = b"0123456789abcdef";
    let mut v = &raw const API as usize;
    let mut buf = [0u8; 20];
    let mut i = 0;
    buf[i] = b'0';
    i += 1;
    buf[i] = b'x';
    i += 1;
    let mut digits = 0;
    let mut tmp = [0u8; 16];
    while v > 0 {
        tmp[digits] = HEX[v & 0xf];
        v >>= 4;
        digits += 1;
    }
    if digits == 0 {
        tmp[0] = b'0';
        digits = 1;
    }
    while digits > 0 {
        digits -= 1;
        buf[i] = tmp[digits];
        i += 1;
    }
    buf[i] = 0;
    let mut j = 0;
    while j <= i {
        API_ENV[j] = buf[j];
        j += 1;
    }
    setenv(
        b"HUSK_DRM_API\0".as_ptr(),
        (&raw const API_ENV) as *const u8,
        1,
    );
}

#[used]
#[link_section = ".init_array"]
static API_CTOR: unsafe extern "C" fn() = api_init;
