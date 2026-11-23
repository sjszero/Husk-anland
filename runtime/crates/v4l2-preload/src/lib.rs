//! libhusk-v4l2.so — LD_PRELOAD 版的 `/dev/videoN`（no_std）。
//!
//! 为什么需要它：loader 里的 shim 只能改**启动时**已加载对象的 GOT；
//! GStreamer 的 `libgstvideo4linux2.so` 是运行时 dlopen 的，它调用 `open/ioctl`
//! 会绕过 loader shim，直接落到 Android 的 `/dev/video1`（存在但 0660 media:system）
//! → EACCES。所以和 DRM 一样，dlopen 的重度用户要挂这一层。
//!
//! **状态（2026-11-23，UNVERIFIED/WIP）**：本层能拦截 `open/ioctl/mmap/...` 的
//! 普通调用，但 GStreamer 的 `libgstvideo4linux2.so` 对设备用的是
//! `open64@GLIBC_2.17` **带版本引用**，ld.so 不把无版本定义绑上去
//! （实测：本层的 openat 完全没被调用，插件仍拿到 Android 那个 0660 节点 → EACCES）。
//! 解决要么给本层加 version script（每个符号按 glibc 版本打标签：open64 是 2.17、
//! statx 是 2.28、fstatat 是 2.33…），要么在 dlopen 时给新对象补 GOT。
//! 在那之前：guest 侧的 GStreamer 用不了 /dev/videoN，自研工具/普通程序可以。
//!
//! 两个实例不能同时连 app（camera.sock 一次只收一个客户端）。

#![no_std]

use core::sync::atomic::{AtomicBool, Ordering};
use v4l2_shim::{Ctx, MAX_BUFFERS};

extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn __errno_location() -> *mut i32;
    fn getenv(name: *const u8) -> *const u8;
    fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
}

const SYS_OPENAT: i64 = 56;
const SYS_IOCTL: i64 = 29;
const SYS_MMAP: i64 = 222;
const SYS_CLOSE: i64 = 57;
const SYS_FCNTL: i64 = 25;
const SYS_NEWFSTATAT: i64 = 79;
const SYS_STATX: i64 = 291;
const SYS_FACCESSAT: i64 = 48;
const AT_FDCWD: i32 = -100;
const O_PATH: i32 = 0o10000000;

static LOCK: AtomicBool = AtomicBool::new(false);
static mut CTX: Ctx = Ctx::empty();
static mut READY: bool = false;
static mut ROOT: [u8; 200] = [0; 200];
static mut ROOT_LEN: usize = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

fn lock() {
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

unsafe fn set_errno(e: i32) -> i32 {
    *__errno_location() = e;
    -1
}

fn errno_of(r: i64) -> i32 {
    (-r) as i32
}

unsafe fn c_str_len(p: *const u8) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

unsafe fn rootfs() -> Option<&'static [u8]> {
    if ROOT_LEN > 0 {
        return Some(&ROOT[..ROOT_LEN]);
    }
    let v = getenv(b"NR_ROOTFS\0".as_ptr());
    if v.is_null() {
        return None;
    }
    let n = c_str_len(v);
    if n == 0 || n >= ROOT.len() {
        return None;
    }
    core::ptr::copy_nonoverlapping(v, ROOT.as_mut_ptr(), n);
    ROOT_LEN = n;
    Some(&ROOT[..n])
}

unsafe fn ensure_init() -> bool {
    if READY {
        return true;
    }
    let root = match rootfs() {
        Some(r) => r,
        None => return false,
    };
    let suffix = b"/run/husk/camera.sock";
    let mut p = [0u8; 256];
    if root.len() + suffix.len() >= p.len() {
        return false;
    }
    p[..root.len()].copy_from_slice(root);
    p[root.len()..root.len() + suffix.len()].copy_from_slice(suffix);
    if !CTX.init(&p[..root.len() + suffix.len()]) {
        return false;
    }
    READY = true;
    true
}

fn video_index(path: &[u8]) -> Option<u32> {
    let pre = b"/dev/video";
    if path.len() <= pre.len() || !path.starts_with(pre) {
        return None;
    }
    let rest = &path[pre.len()..];
    if rest.len() > 3 {
        return None;
    }
    let mut n = 0u32;
    for &c in rest {
        if !c.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (c - b'0') as u32;
    }
    Some(n)
}

/// `/sys/class/video4linux*`, `/sys/subsystem/video4linux*` → rootfs 影子树
unsafe fn sys_host(path: &[u8], out: &mut [u8; 256]) -> Option<usize> {
    let root = rootfs()?;
    const C: &[u8] = b"/sys/class/video4linux";
    const S: &[u8] = b"/sys/subsystem/video4linux";
    let (dir, rest) = if path.starts_with(C) {
        (b"/sys-v4l2/class".as_ref(), &path[C.len()..])
    } else if path.starts_with(S) {
        (b"/sys-v4l2/subsystem".as_ref(), &path[S.len()..])
    } else {
        return None;
    };
    if !rest.is_empty() && rest[0] != b'/' {
        return None;
    }
    let n = root.len() + dir.len() + rest.len();
    if n + 1 > out.len() {
        return None;
    }
    out[..root.len()].copy_from_slice(root);
    out[root.len()..root.len() + dir.len()].copy_from_slice(dir);
    out[root.len() + dir.len()..n].copy_from_slice(rest);
    out[n] = 0;
    Some(n)
}

unsafe fn open_video(n: u32) -> Option<i32> {
    if !ensure_init() {
        return None;
    }
    let fd = CTX.open(n);
    if fd < 0 {
        None
    } else {
        Some(fd)
    }
}

unsafe fn synth_stat(n: u32, st: *mut u8) -> i32 {
    core::ptr::write_bytes(st, 0, 128);
    core::ptr::write_unaligned(st.add(8) as *mut u64, n as u64 + 1);
    core::ptr::write_unaligned(st.add(16) as *mut u32, 0o020666u32);
    core::ptr::write_unaligned(st.add(20) as *mut u32, 1u32);
    core::ptr::write_unaligned(st.add(32) as *mut u64, (81u64 << 8) | n as u64);
    core::ptr::write_unaligned(st.add(56) as *mut i64, 4096i64);
    0
}

unsafe fn synth_statx(n: u32, buf: *mut u8) -> i32 {
    core::ptr::write_bytes(buf, 0, 256);
    core::ptr::write_unaligned(buf.add(0) as *mut u32, 0x7ffu32);
    core::ptr::write_unaligned(buf.add(4) as *mut u32, 4096u32);
    core::ptr::write_unaligned(buf.add(16) as *mut u32, 1u32);
    core::ptr::write_unaligned(buf.add(28) as *mut u16, 0o020666u16);
    core::ptr::write_unaligned(buf.add(32) as *mut u64, n as u64 + 1);
    core::ptr::write_unaligned(buf.add(128) as *mut u32, 81u32);
    core::ptr::write_unaligned(buf.add(132) as *mut u32, n);
    0
}

unsafe fn path_bytes(p: *const i8) -> Option<&'static [u8]> {
    if p.is_null() || *p != b'/' as i8 {
        return None;
    }
    let n = c_str_len(p as *const u8);
    Some(core::slice::from_raw_parts(p as *const u8, n))
}

// ---- hooks ----

#[no_mangle]
pub unsafe extern "C" fn open(path: *const i8, flags: i32, mode: i32) -> i32 {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn open64(path: *const i8, flags: i32, mode: i32) -> i32 {
    openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut buf = [0u8; 256];
        if let Some(n) = sys_host(p, &mut buf) {
            return syscall(SYS_OPENAT, AT_FDCWD as i64, buf.as_ptr() as i64, flags as i64, mode as i64)
                as i32;
        }
        if let Some(vn) = video_index(p) {

            if let Some(fd) = open_video(vn) {
                return fd;
            }

        }
    }
    let r = syscall(SYS_OPENAT, dirfd as i64, path as i64, flags as i64, mode as i64);
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn openat64(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i32 {
    openat(dirfd, path, flags, mode)
}

// Fortified builds (Ubuntu's GStreamer) never call `open` directly: with
// _FORTIFY_SOURCE, open(path, flags) becomes __open_2(path, flags) and
// openat() becomes __openat_2(). Those live in libc and are entered through a
// direct symbol, so without these four aliases the plugin bypasses us.
#[no_mangle]
pub unsafe extern "C" fn __open_2(path: *const i8, flags: i32) -> i32 {
    openat(AT_FDCWD, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __open64_2(path: *const i8, flags: i32) -> i32 {
    openat(AT_FDCWD, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __openat_2(dirfd: i32, path: *const i8, flags: i32) -> i32 {
    openat(dirfd, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __openat64_2(dirfd: i32, path: *const i8, flags: i32) -> i32 {
    openat(dirfd, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    if CTX.is_video_fd(fd) {

        let r = CTX.ioctl(fd, request, arg);
        return if r < 0 { set_errno(errno_of(r)) } else { 0 };
    }
    let r = syscall(SYS_IOCTL, fd as i64, request as i64, arg as i64);
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
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
    if CTX.is_video_fd(fd) {
        return match CTX.mmap(fd, len, off) {
            Some(p) => p,
            None => set_errno(22) as *mut u8,
        };
    }
    let r = syscall(
        SYS_MMAP,
        addr as i64,
        len as i64,
        prot as i64,
        flags as i64,
        fd as i64,
        off,
    );
    if r < 0 && r > -4096 {
        return set_errno(errno_of(r)) as *mut u8;
    }
    r as *mut u8
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
    mmap(addr, len, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn close(fd: i32) -> i32 {
    if CTX.is_video_fd(fd) {
        CTX.close(fd);
    }
    let r = syscall(SYS_CLOSE, fd as i64);
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: i32, cmd: i32, arg: usize) -> i32 {
    let r = syscall(SYS_FCNTL, fd as i64, cmd as i64, arg as i64);
    if r >= 0 && CTX.is_video_fd(fd) {
        if cmd == 4 {
            CTX.update_nonblock(fd, (arg & 0o4000) != 0);
        } else if cmd == 3 {
            CTX.update_nonblock(fd, (r as usize & 0o4000) != 0);
        }
    }
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn stat(path: *const i8, st: *mut u8) -> i32 {
    fstatat(AT_FDCWD, path, st, 0)
}

#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const i8, st: *mut u8) -> i32 {
    fstatat(AT_FDCWD, path, st, 0x100)
}

#[no_mangle]
pub unsafe extern "C" fn fstatat(dirfd: i32, path: *const i8, st: *mut u8, flags: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut buf = [0u8; 256];
        if let Some(n) = sys_host(p, &mut buf) {
            let r = syscall(
                SYS_NEWFSTATAT,
                AT_FDCWD as i64,
                buf.as_ptr() as i64,
                st as i64,
                flags as i64,
            );
            if r < 0 {
                return set_errno(errno_of(r));
            }
            return 0;
        }
        if let Some(vn) = video_index(p) {
            return synth_stat(vn, st);
        }
    }
    let r = syscall(
        SYS_NEWFSTATAT,
        dirfd as i64,
        path as i64,
        st as i64,
        flags as i64,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn statx(
    dirfd: i32,
    path: *const i8,
    flags: i32,
    mask: u32,
    buf: *mut u8,
) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut sb = [0u8; 256];
        if let Some(n) = sys_host(p, &mut sb) {
            let r = syscall(
                SYS_STATX,
                AT_FDCWD as i64,
                sb.as_ptr() as i64,
                flags as i64,
                mask as i64,
                buf as i64,
            );
            if r < 0 {
                return set_errno(errno_of(r));
            }
            return 0;
        }
        if let Some(vn) = video_index(p) {
            return synth_statx(vn, buf);
        }
    }
    let r = syscall(
        SYS_STATX,
        dirfd as i64,
        path as i64,
        flags as i64,
        mask as i64,
        buf as i64,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn access(path: *const i8, mode: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        if video_index(p).is_some() {
            return 0;
        }
    }
    let r = syscall(SYS_FACCESSAT, AT_FDCWD as i64, path as i64, mode as i64, 0i64);
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

// ---- libv4l2 symbols ------------------------------------------------------
//
// GStreamer's libgstvideo4linux2.so links libv4l2.so.0 and calls v4l2_open(),
// v4l2_ioctl(), v4l2_mmap(), v4l2_close() instead of the libc entry points.
// Those run inside libv4l2 (dlopen'd), so neither the loader shim nor our
// open/ioctl hooks see them. The global scope is searched before the plugin's
// own dependencies, so defining the same names here takes over the whole
// libv4l2 API surface the plugin uses.
//
// v4l2_ioctl() is variadic in libv4l2's header; a fixed (fd, request, arg)
// definition is ABI-compatible for every caller that passes an argument.

#[no_mangle]
pub unsafe extern "C" fn v4l2_open(path: *const i8, flags: i32) -> i32 {
    open(path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_fd_open(fd: i32, _flags: i32) -> i32 {
    fd
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    ioctl(fd, request, arg)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_read(fd: i32, buf: *mut u8, n: usize) -> isize {
    syscall(63, fd as i64, buf as i64, n as i64) as isize
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_mmap(
    start: *mut u8,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    mmap(start, length, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_munmap(start: *mut u8, length: usize) -> i32 {
    syscall(215, start as i64, length as i64) as i32
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_close(fd: i32) -> i32 {
    close(fd)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_dup(fd: i32) -> i32 {
    syscall(23, fd as i64) as i32
}

// ---- ABI for the loader shim -------------------------------------------
//
// Both layers must share ONE Ctx (the app accepts one camera client), so the
// preload owns it and the loader shim forwards here when HUSK_V4L2_API is set.
// Same handshake as the DRM shim.

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_open(n: u32) -> i32 {
    match open_video(n) {
        Some(fd) => fd,
        None => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_ioctl(fd: i32, req: u64, arg: *mut u8) -> i64 {
    if !CTX.is_video_fd(fd) {
        return -25;
    }
    CTX.ioctl(fd, req, arg)
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_mmap(fd: i32, len: usize, off: i64) -> *mut u8 {
    match CTX.mmap(fd, len, off) {
        Some(p) => p,
        None => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_is_fake(fd: i32) -> i32 {
    if CTX.is_video_fd(fd) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_close(fd: i32) {
    if CTX.is_video_fd(fd) {
        CTX.close(fd);
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_nonblock(fd: i32, on: i32) {
    CTX.update_nonblock(fd, on != 0);
}

static mut API: [usize; 6] = [0; 6];

unsafe extern "C" fn announce() {
    API = [
        husk_v4l2_open as usize,
        husk_v4l2_ioctl as usize,
        husk_v4l2_mmap as usize,
        husk_v4l2_is_fake as usize,
        husk_v4l2_close as usize,
        husk_v4l2_nonblock as usize,
    ];
    let _ = setenv(b"HUSK_V4L2_PRELOAD\0".as_ptr(), b"1\0".as_ptr(), 1);
    // Hex table address, same encoding path-shim already parses for DRM.
    let mut hex = [0u8; 20];
    let mut p = (core::ptr::addr_of!(API)) as usize;
    let mut i = 2;
    hex[0] = b'0';
    hex[1] = b'x';
    let mut shift = (core::mem::size_of::<usize>() * 8) / 4;
    let mut started = false;
    let digits = b"0123456789abcdef";
    while shift > 0 {
        shift -= 4;
        let d = (p >> shift) & 0xf;
        if d != 0 {
            started = true;
        }
        if started {
            hex[i] = digits[d];
            i += 1;
        }
    }
    if !started {
        hex[i] = b'0';
        i += 1;
    }
    hex[i] = 0;
    // NOTE: 2026-11-23 — publishing the table and letting the loader shim forward
    // crashed inside the shim on the first /dev/video open (evidence 91 §crash).
    // Until that is diagnosed the two layers keep their own Ctx: startup objects
    // use the loader shim, dlopen'd consumers (GStreamer) use this one. A process
    // must not open the device through both paths at once (camera.sock is one
    // client); gst-launch and our tools each use exactly one.
    let _ = &hex;
    let _ = O_PATH;
    let _ = MAX_BUFFERS;
    let _ = &mut p;
}

#[used]
#[link_section = ".init_array"]
static ANNOUNCE: unsafe extern "C" fn() = announce;
